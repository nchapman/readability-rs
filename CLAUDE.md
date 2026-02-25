# readability-rs

Rust port of [go-readability](https://codeberg.org/readeck/go-readability), a Go implementation of Mozilla's Readability.js algorithm for extracting readable article content from web pages.

This library is used as the fallback extraction engine in [trafilatura-rs](../trafilatura-rs). Keeping it as a separate crate allows independent use and testing.

## Source References

- **Go source** (port from this): `/Users/nchapman/Code/lessisbetter/refs/go-readability`
- **Original JS reference**: [mozilla/readability](https://github.com/mozilla/readability) — check when Go behavior is ambiguous
- **Implementation plan**: `PLAN.md` in this repo

When porting a function, always read the Go source first. When behavior is unclear, check the Mozilla JS original. The Go source is a fork of `github.com/go-shiori/go-readability` updated for Readability.js 0.6.0 compatibility with performance optimizations.

## Porting Philosophy

This is a **faithful, careful port**. The goal is correctness first, then idiom, then performance. A faithful port is essential because the 134 test fixtures (`test-pages/`) encode the exact expected behavior and any deviation will fail tests.

### Mirror Go structure for maintainability

The Go codebase may evolve. Our Rust module layout, function names, and file organization should make it easy to find the corresponding Go code and port future changes. Specifically:

- **One Rust file per Go file** where practical. The Go→Rust file mapping is:
  - `readability.go` → `src/lib.rs` (public API: `from_html`, `from_reader`)
  - `article.go` → `src/article.rs` (Article struct and accessors)
  - `parser.go` → `src/parser.rs` (core algorithm: `grab_article`, `prep_document`, etc.)
  - `parser-parse.go` → `src/parser.rs` (Parse/ParseDocument/ParseAndMutate methods)
  - `parser-check.go` → `src/parser.rs` (check_document)
  - `traverse.go` → `src/traverse.rs` (has_text_content, count_chars_and_commas)
  - `utils.go` → `src/utils.rs` (text helpers: word_count, to_absolute_uri, etc.)
  - `render/inner_text.go` → `src/render/mod.rs` (InnerText — plain text extraction)
  - `inspect_node.go` → `src/inspect.rs` (debug/logging helpers, can stub initially)
  - `internal/re2go/*.re` → `src/regexp/mod.rs` (all RE2 patterns as LazyLock<Regex>)

- **Keep Go function names** as Rust equivalents. `grabArticle` → `grab_article`, `prepDocument` → `prep_document`, `cleanConditionally` → `clean_conditionally`, `getLinkDensity` → `get_link_density`. When someone reads a Go function name they should be able to find the Rust version instantly.
- **Port functions in the same order they appear in the Go file**. This makes diff-based comparison possible.
- **Add a comment at the top of each Rust file** noting which Go file it ports: `// Port of go-readability/parser.go`
- **When porting a function**, add a brief doc comment that includes the Go function name: `/// Port of grabArticle`

### Write idiomatic Rust

While mirroring Go's structure, the code itself should be idiomatic Rust:

- Use `Result<T, E>` and the `?` operator instead of Go's `(result, error)` pattern
- Use `Option<T>` instead of nil checks
- Use iterators and combinators where they're clearer than loops
- Use `&str` for borrowed strings, `String` for owned
- Use `std::sync::LazyLock` for compiled regex patterns (stable since Rust 1.80, do not use `once_cell`)
- Derive `Debug`, `Clone`, `Default` on public types where appropriate
- Use `thiserror` for error types
- No `unwrap()` in library code (only in tests and static regex compilation inside `LazyLock`)

### What NOT to do

- Do not refactor Go logic while porting. If the Go code has a weird branch or redundant check, port it faithfully. We can refactor later once tests prove equivalence.
- Do not add features that the Go version doesn't have.
- Do not skip a function because it "seems unnecessary." Port it, test it.
- Do not use `async` in the library. Extraction is synchronous.
- Do not port the generated re2go `.go` files — port the `.re` source patterns only, as `LazyLock<Regex>`.

## Workflow

### Cycle for each section of work

1. **Read the Go source** for the module you're porting
2. **Implement** the Rust equivalent
3. **Write tests** — port the corresponding Go tests, add Rust-specific edge cases
4. **Run `cargo test` and `cargo clippy`** — fix all warnings
5. **Request a code review** (use the `code-reviewer` agent)
6. **Fix review findings**, re-run tests
7. **Commit** with a clean, descriptive message

### Commit discipline

- Commit after completing each coherent piece of work (a module, a group of related functions)
- Do **not** reference plan phases or milestones in commit messages
- Write clear, specific descriptions of what changed:
  - Good: `Port regexp patterns from re2go source files`
  - Good: `Port grab_article scoring and candidate selection`
  - Good: `Port render::inner_text with block-level semantics`
  - Bad: `Phase 2 complete`
  - Bad: `WIP`
  - Bad: `Fix stuff`
- Use imperative mood: "Add", "Port", "Fix", "Implement"
- Include a brief bullet list of changes when the commit touches multiple concerns

### Testing standards

- **Port Go tests first**. `parser_test.go` uses the 134 `test-pages/` fixtures — port these as the primary integration tests.
- **Use the same test fixtures**. `test-pages/` is already copied into this repo. Each case has `source.html`, `expected.html`, and optionally `expected-metadata.json`.
- **Test each function in isolation** before integration. Regexp functions and text utilities should have standalone unit tests.
- **Use `pretty_assertions`** for string comparison — the diff output is essential for debugging HTML differences.
- **When a test fails**, compare output against the Go version before assuming the Rust code is wrong.
- Run `cargo test` after every change. Do not batch up untested work.

## Technical Reference

### DOM abstraction (`src/dom/`)

Unlike trafilatura-rs (which needed a text/tail abstraction for Python's ElementTree semantics), readability's DOM usage is more straightforward — it works directly with standard HTML nodes. However we still need a wrapper because:

- `scraper::Html` is designed read-only; we need mutation (removing nodes, setting attributes, replacing elements)
- We need stable `NodeId` handles to avoid borrow checker issues when traversing + mutating
- We need attribute get/set helpers (especially `data-readability-score` and `data-readability-table`)

Key operations the DOM layer must support:
- `tag_name(id)` — get element tag
- `attr(id, name)` — get attribute value
- `set_attr(id, name, value)` — set attribute
- `remove_attr(id, name)` — remove attribute
- `text_content(id)` — get all text content recursively
- `outer_html(id)` / `inner_html(id)` — serialize to HTML
- `children(id)` — direct element children
- `parent(id)` — parent node
- `ancestors(id)` — ancestor chain
- `remove(id)` — detach node from tree
- `replace_with_children(id)` — unwrap (keep children, remove element)
- `insert_before(id, new_id)` — insert sibling before node
- `append_child(parent_id, child_id)` — move node into parent
- `create_element(tag)` — create new element node
- `set_text(id, text)` — set text content of element
- `is_hidden(id)` — check display:none / visibility:hidden / hidden attribute

**Important**: The Go source uses `dom.Children()`, `dom.GetElementsByTagName()`, `dom.HasChildNodes()`, etc. from `github.com/go-shiori/dom`. Port these to our DOM layer.

### Scoring via side table

The Go implementation stores content scores as `data-readability-score` string attributes on nodes. In Rust, use a **`HashMap<NodeId, f64>`** side table instead. This avoids the overhead of string formatting and parsing floats, and keeps the tree clean. The side table is local to each `grab_article` invocation.

Similarly, `data-readability-table` boolean flags use a `HashSet<NodeId>`.

### RE2 patterns (`src/regexp/mod.rs`)

The Go source has five `.re` pattern files that were compiled to state machines. Port each as a `LazyLock<Regex>`. The patterns are documented in the `.re` source files:

| Go function | Source pattern | Notes |
|-------------|----------------|-------|
| `IsUnlikelyCandidates` | `(?i)-ad-\|ai2html\|banner\|breadcrumbs\|combx\|comment\|community\|cover-wrap\|disqus\|extra\|footer\|gdpr\|header\|legends\|menu\|related\|remark\|replies\|rss\|shoutbox\|sidebar\|skyscraper\|social\|sponsor\|supplemental\|ad-break\|agegate\|pagination\|pager\|popup\|yom-remote` | Substring match |
| `MaybeItsACandidate` | `(?i)and\|article\|body\|column\|content\|main\|mathjax\|shadow` | Substring match |
| `IsPositiveClass` | `(?i)article\|body\|content\|entry\|hentry\|h-entry\|main\|page\|pagination\|post\|text\|blog\|story` | Substring match |
| `IsNegativeClass` | Two parts: `(?i)(^| )(hid\|hidden\|d-none)( \|$)` + `(?i)-ad-\|banner\|combx\|comment\|com-\|contact\|footer\|gdpr\|masthead\|meta\|outbrain\|promo\|related\|share\|shoutbox\|sidebar\|skyscraper\|sponsor\|shopping\|tags\|widget` | Second is substring; first needs anchored match |
| `IsByline` | `(?i)byline\|author\|dateline\|writtenby\|p-author` | Substring match |
| `NormalizeSpaces` | `[\t\n\f\r ]{2,}` → `" "` | Replace, not match |

All substring-match functions should use `Regex::is_match()` on the full input string (same behavior as the FSM which scans for a substring).

### `render::inner_text` (`src/render/mod.rs`)

This is a faithful port of the [MDN `innerText`](https://developer.mozilla.org/en-US/docs/Web/API/HTMLElement/innerText) spec:
- Block-level elements (`p`, `div`, `h1`-`h6`, `table`, `ul`, `ol`, etc.) add newlines
- Table cells add tabs
- `display:none` elements are skipped
- MathJax/LaTeX elements: output their LaTeX source (not rendered text)
- Consecutive whitespace is collapsed

This is distinct from `textContent` (which just concatenates all text nodes). The difference matters for article quality — `innerText` respects visual layout.

### Multi-pass algorithm

`grab_article` runs up to 5 passes with progressively relaxed constraints:

| Pass | `strip_unlikelys` | `use_weight_classes` | `clean_conditionally` |
|------|-------------------|---------------------|----------------------|
| 1 | true | true | true |
| 2 | false | true | true |
| 3 | true | false | true |
| 4 | false | false | true |
| 5 | false | false | false |

Each pass: if content length ≥ `char_threshold` (default 500), accept and return. Otherwise try next pass.

### Scoring algorithm

**Initial node score by tag:**
- `div`: +5
- `pre`, `td`, `blockquote`: +3
- `address`, `ol`, `ul`, `dl`, `dd`, `dt`, `li`, `form`: -3
- `h1`–`h6`, `th`: -5

**Class weight:**
- IsPositiveClass match: +25
- IsNegativeClass match: -25

**Per-paragraph content score:**
- Base: +1
- Commas in text: +1 each
- Character bonus: +1 per 100 chars, max +3

**Score propagation to ancestors:**
- Level 0 (parent): divide by 1
- Level 1 (grandparent): divide by 2
- Level 2+ (great-grandparent+): divide by (level × 3)

**Final adjustments:**
- Scale top candidate score by `1 - link_density`
- Walk up tree if parent has same or higher score
- Include siblings with score ≥ 20% of top candidate score

### Key crate choices

| Crate | Purpose | Why |
|-------|---------|-----|
| `scraper` | HTML parsing + CSS selectors | Built on html5ever, spec-compliant HTML5 |
| `ego-tree` | Tree mutation | Exposed by scraper, stable NodeId handles |
| `regex` | Pattern matching | RE2-compatible, linear time |
| `url` | URL resolution | Used in `fix_relative_uris` |
| `serde` + `serde_json` | JSON-LD metadata parsing | Standard |
| `dateparser` | Date string parsing | Port of go's `araddon/dateparse` |
| `chrono` | Date types | Standard Rust date library |
| `tracing` | Structured logging | Port of go-readability's slog usage |
| `thiserror` | Error types | Derive Error implementations |
| `pretty_assertions` | Test diffs (dev only) | Essential for HTML comparison failures |

## Commands

```bash
cargo test                    # Run all tests
cargo clippy                  # Lint
cargo fmt --check             # Format check
cargo bench                   # Run benchmarks
```

## File layout

```
src/
├── lib.rs           # Public API: from_html(), from_reader()  [readability.go]
├── article.rs       # Article struct + accessors              [article.go]
├── error.rs         # Error enum
├── parser.rs        # Core algorithm + parse methods          [parser.go, parser-parse.go, parser-check.go]
├── traverse.rs      # has_text_content, count_chars_and_commas [traverse.go]
├── utils.rs         # Text helpers, URL resolution             [utils.go]
├── inspect.rs       # Debug/logging helpers (stub ok initially) [inspect_node.go]
├── dom/
│   └── mod.rs       # Document wrapper, NodeId, DOM operations
├── regexp/
│   └── mod.rs       # LazyLock<Regex> patterns                [internal/re2go/*.re]
└── render/
    └── mod.rs       # InnerText — plain text extraction        [render/inner_text.go]

test-pages/          # 134 test fixtures copied from go-readability
  001/
    source.html
    expected.html
    expected-metadata.json  (where present)
  ...

benches/
└── extraction.rs    # Criterion benchmarks
```
