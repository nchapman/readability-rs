# readability-rs Implementation Plan

## Context

[Readability.js](https://github.com/mozilla/readability) is Mozilla's algorithm for extracting readable article content from web pages (used in Firefox Reader View). This crate is a Rust port of [go-readability](https://codeberg.org/readeck/go-readability), which is itself a faithful Go port of Readability.js 0.6.0.

**Why this crate exists**: [trafilatura-rs](../trafilatura-rs) uses readability as a fallback extraction engine. The existing Rust readability crates (`readable-readability`, `readability-rust`) have significant algorithmic gaps — incomplete ancestor traversal, broken candidate selection, missing re2go patterns — resulting in ~2.5pp lower recall than Go trafilatura. A faithful port of go-readability closes this gap.

**Source references**:
- Go source (port from this): `/Users/nchapman/Code/lessisbetter/refs/go-readability`
- Mozilla JS original (behavioral reference): https://github.com/mozilla/readability

**Target directory**: `/Users/nchapman/Drive/Code/lessisbetter/readability-rs`

---

## Go Source Overview

The go-readability codebase is ~3,000 lines of real logic (plus ~4,000 lines of re2c-generated FSMs we replace with regex):

```
go-readability/
├── readability.go          (79 lines)   Public API entry points
├── article.go              (108 lines)  Article result type + accessors
├── parser.go               (2,525 lines) Core algorithm — THE MAIN FILE
├── parser-parse.go         (96 lines)   Parse/ParseDocument/ParseAndMutate
├── parser-check.go         (68 lines)   CheckDocument (readability check)
├── traverse.go             (93 lines)   DOM traversal helpers
├── utils.go                (135 lines)  Text utilities, URL resolution
├── inspect_node.go         (116 lines)  Debug logging helpers
├── render/
│   └── inner_text.go       (256 lines)  MDN innerText plain text extraction
└── internal/re2go/
    ├── grab-article.re     Source: IsUnlikelyCandidates, MaybeItsACandidate patterns
    ├── class-weight.re     Source: IsPositiveClass, IsNegativeClass patterns
    ├── check-byline.re     Source: IsByline pattern
    └── normalize.re        Source: NormalizeSpaces pattern
```

Test fixtures: 134 cases in `test-pages/`, each with `source.html` + `expected.html` + optional `expected-metadata.json`.

---

## File Map

Every Go source file maps to exactly one Rust file. When porting a file, open the Go source and work top-to-bottom — port functions in the order they appear. Add a `// Port of <GoFile>` comment at the top of each Rust file.

| Go file | Rust file | Phase | Status |
|---------|-----------|-------|--------|
| *(new — no Go equivalent)* | `src/dom/mod.rs` | 1 | 🔲 |
| `internal/re2go/*.re` + `parser.go` var block | `src/regexp/mod.rs` | 2 | 🔲 |
| `utils.go` | `src/utils.rs` | 3 | 🔲 |
| `traverse.go` | `src/traverse.rs` | 3 | 🔲 |
| `article.go` | `src/article.rs` | 4 | 🔲 |
| `render/inner_text.go` | `src/render/mod.rs` | 4 | 🔲 |
| `inspect_node.go` | `src/inspect.rs` | 4 | 🔲 |
| `parser.go` (lines 1–~800: prep + metadata) | `src/parser.rs` | 5 | 🔲 |
| `parser.go` (lines ~800–1400: `grabArticle` scoring) | `src/parser.rs` | 6 | 🔲 |
| `parser.go` (lines ~1400–end: `prepArticle`, `postProcessContent`, cleaning) | `src/parser.rs` | 7 | 🔲 |
| `parser-parse.go` | `src/parser.rs` | 8 | 🔲 |
| `parser-check.go` | `src/parser.rs` | 8 | 🔲 |
| `readability.go` | `src/lib.rs` | 8 | 🔲 |

Update the Status column as each file is completed (🔲 TODO → 🔄 In Progress → ✅ Done).

### Per-file porting discipline

For every Go file you port:

1. **Open the Go file** at the top. Read the entire file before writing any Rust.
2. **Port functions top-to-bottom** in the order they appear in the Go file.
3. **For each function**: write the Rust equivalent, then write tests for it before moving to the next function.
4. **Run `cargo test` and `cargo clippy`** after each function is complete. Do not accumulate untested work.
5. **Add a doc comment** to each ported function: `/// Port of goFunctionName`.
6. When a Go function's behavior is unclear, check the Mozilla JS original before assuming.

---

## Implementation Phases

### Phase 1: Project Scaffold + DOM Abstraction

> **Status: 🔲 TODO**
> **Go source**: *(none — new abstraction layer)*
> **Rust output**: `src/dom/mod.rs`

**Goal**: Establish the DOM abstraction layer that all subsequent phases depend on. Get `cargo build` and `cargo test` working with empty stubs.

**Dependencies**: None.

**Key decisions established in this phase**:
- Use `scraper` + `ego-tree` for HTML parsing and mutation (consistent with trafilatura-rs)
- Use `HashMap<NodeId, f64>` for content scores (not DOM attributes) — cleaner, no float formatting overhead
- Use `HashSet<NodeId>` for boolean flags (data tables, etc.)
- `NodeId` = `ego_tree::NodeId`, re-exported from `src/dom/mod.rs`
- Internally keep a `scraper::Html` (not just a raw tree) to support CSS selector queries via `Html::select`. Access `html.tree` (pub field in scraper 0.22) for mutations.
- Use `ego_tree::NodeMut::detach()` for node removal during traversal. **Do not use tree node removal that deallocates the NodeId** — detached nodes remain valid and accessible by their `NodeId`; they are simply no longer connected to the tree. Callers must not access detached nodes for tree-relative operations (parent, siblings), but can still read their data.

**What to build**:

`src/dom/mod.rs` — Document wrapper providing:

```rust
pub struct Document {
    html: scraper::Html,  // html.tree is the ego_tree::Tree<scraper::Node>
}

impl Document {
    pub fn parse(html: &str) -> Self
    pub fn clone(&self) -> Self                    // deep clone of entire document
    pub fn root(&self) -> NodeId
    pub fn document_element(&self) -> Option<NodeId>  // <html> root element
    pub fn body(&self) -> Option<NodeId>

    // Node identity
    pub fn tag_name(&self, id: NodeId) -> &str
    pub fn is_element(&self, id: NodeId) -> bool

    // Attributes
    pub fn attr(&self, id: NodeId, name: &str) -> Option<&str>
    pub fn has_attribute(&self, id: NodeId, name: &str) -> bool
    pub fn set_attr(&mut self, id: NodeId, name: &str, value: &str)
    pub fn remove_attr(&mut self, id: NodeId, name: &str)

    // Text
    pub fn text_content(&self, id: NodeId) -> String
    pub fn inner_html(&self, id: NodeId) -> String
    pub fn outer_html(&self, id: NodeId) -> String
    pub fn set_text(&mut self, id: NodeId, text: &str)

    // Tree navigation
    pub fn parent(&self, id: NodeId) -> Option<NodeId>
    pub fn children(&self, id: NodeId) -> Vec<NodeId>           // element children only
    pub fn child_nodes(&self, id: NodeId) -> Vec<NodeId>        // all children incl. text nodes
    pub fn first_element_child(&self, id: NodeId) -> Option<NodeId>
    pub fn last_element_child(&self, id: NodeId) -> Option<NodeId>
    pub fn ancestors(&self, id: NodeId) -> Vec<NodeId>          // parent, grandparent, ...
    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId>    // next node (any type)
    pub fn prev_sibling(&self, id: NodeId) -> Option<NodeId>    // prev node (any type)
    pub fn next_element_sibling(&self, id: NodeId) -> Option<NodeId>  // next element only
    pub fn prev_element_sibling(&self, id: NodeId) -> Option<NodeId>  // prev element only
    pub fn descendants(&self, id: NodeId) -> Vec<NodeId>        // all element descendants
    pub fn get_elements_by_tag_name(&self, id: NodeId, tag: &str) -> Vec<NodeId>
    // Multi-tag version: tag "*" returns all element descendants
    pub fn get_all_nodes_with_tag(&self, id: NodeId, tags: &[&str]) -> Vec<NodeId>
    pub fn query_selector(&self, id: NodeId, selector: &str) -> Option<NodeId>
    pub fn query_selector_all(&self, id: NodeId, selector: &str) -> Vec<NodeId>

    // Tree mutation
    pub fn remove(&mut self, id: NodeId)                            // detach node (NodeId remains valid but orphaned)
    pub fn replace_with_children(&mut self, id: NodeId)            // unwrap element, keep children
    pub fn append_child(&mut self, parent: NodeId, child: NodeId)  // move child to end of parent
    pub fn insert_before(&mut self, id: NodeId, new_node: NodeId)  // insert new_node before id
    pub fn create_element(&mut self, tag: &str) -> NodeId
    pub fn create_text_node(&mut self, text: &str) -> NodeId
    // Change element tag name by creating new element node with same attrs + children
    pub fn set_tag_name(&mut self, id: NodeId, new_tag: &str) -> NodeId  // returns new NodeId

    // Visibility (used by isProbablyVisible)
    pub fn is_hidden(&self, id: NodeId) -> bool   // display:none, visibility:hidden, hidden attr

    // Convenience
    pub fn has_child_nodes(&self, id: NodeId) -> bool
    pub fn has_children(&self, id: NodeId) -> bool   // element children only
}
```

**Important notes on `set_tag_name`**: `scraper::Node::Element` stores the tag name as a `QualName` which is immutable. Changing a tag requires creating a new element node with the desired tag, copying all attributes, reparenting all children, inserting the new node before the old one, then detaching the old node. `set_tag_name` returns the new `NodeId` — callers must use the new ID from this point on. Used in `replace_node_tags` (converting `<div>` to `<p>` etc).

**Notes on `query_selector` / `query_selector_all`**: Use `scraper::Selector::parse(selector)` and iterate via `scraper::Html::select()`. The `Html` type's `select` method works on the internal tree. For subtree-scoped queries, filter results to descendants of `id`.

**Tests**: Unit tests for each DOM operation with small HTML snippets. Verify:
- `text_content` collects all text recursively
- `replace_with_children` correctly re-parents children
- `remove` detaches a subtree (node's data still readable via its `NodeId`)
- `is_hidden` detects `display:none` and `visibility:hidden` in style attributes
- `get_elements_by_tag_name("*")` returns all descendants in document order
- `set_tag_name` changes the tag and returns a new valid `NodeId`
- `clone` produces a deep copy where mutating one does not affect the other

**Acceptance criteria**:
- [ ] `cargo build` clean
- [ ] `cargo clippy` clean
- [ ] All DOM unit tests pass

---

### Phase 2: Regexp Patterns

> **Status: 🔲 TODO**
> **Go source**: `internal/re2go/grab-article.re`, `internal/re2go/class-weight.re`, `internal/re2go/check-byline.re`, `internal/re2go/normalize.re`, `parser.go` (var block, lines ~24–51)
> **Rust output**: `src/regexp/mod.rs`

**Goal**: Port all re2go pattern functions AND all inline `parser.go` regex patterns as `LazyLock<Regex>`. These are the foundation for all scoring and cleaning decisions.

**Dependencies**: Phase 1 (project compiles).

**Source**: `internal/re2go/*.re` files AND `parser.go` lines ~24–51 (the `var ( rx... )` block).

#### 2a. Re2go patterns — `src/regexp/mod.rs`

```rust
use std::sync::LazyLock;
use regex::Regex;

// Port of IsUnlikelyCandidates (grab-article.re)
// Substring match: returns true if input contains any unlikely pattern
pub fn is_unlikely_candidate(input: &str) -> bool

// Port of MaybeItsACandidate (grab-article.re)
pub fn maybe_its_a_candidate(input: &str) -> bool

// Port of IsPositiveClass (class-weight.re)
pub fn is_positive_class(input: &str) -> bool

// Port of IsNegativeClass (class-weight.re)
// Combines two patterns: anchored hid/hidden/d-none + substring negatives
// Uses TWO separate LazyLock<Regex> OR'd together — do not combine into one pattern
pub fn is_negative_class(input: &str) -> bool

// Port of IsByline (check-byline.re)
pub fn is_byline(input: &str) -> bool

// Port of NormalizeSpaces (normalize.re)
// Replaces runs of 2+ whitespace chars with a single space
pub fn normalize_spaces(input: &str) -> String
```

All substring-match functions use `Regex::is_match()` on the full input.

#### 2b. Parser-level patterns — also in `src/regexp/mod.rs`

Port all 20+ patterns from the `var ( rx... )` block at the top of `parser.go`. These must be `LazyLock<Regex>` constants in `src/regexp/mod.rs` (not inline in parser functions). Key patterns include:

| Go name | Use |
|---------|-----|
| `rxVideos` | Match video embed hostnames (used in `clean`) |
| `rxTokenize` | `\W+` — split text into word tokens (used in `text_similarity`) |
| `rxHasContent` | Check non-whitespace content |
| `rxPropertyPattern` | Meta tag property attribute parsing |
| `rxNamePattern` | Meta tag name attribute parsing |
| `rxTitleSeparator` | `\|` separator in `<title>` |
| `rxTitleHierarchySep` | `»` hierarchy separator |
| `rxTitleRemoveFinalPart` | Strip site name from title |
| `rxTitleRemove1stPart` | Strip leading site name |
| `rxTitleAnySeparator` | Any title separator character |
| `rxDisplayNone` | CSS `display:none` in style attr |
| `rxVisibilityHidden` | CSS `visibility:hidden` in style attr |
| `rxSentencePeriod` | Detect sentence-ending period |
| `rxShareElements` | Detect share/social widgets |
| `rxFaviconSize` | Parse favicon size hints |
| `rxLazyImageSrcset` | Detect lazy-load srcset attributes |
| `rxLazyImageSrc` | Detect lazy-load src attributes |
| `rxImgExtensions` | Common image file extensions |
| `rxSrcsetURL` | Parse individual URLs in srcset |
| `rxB64DataURL` | Base64 data: URIs |
| `rxJsonLdArticleTypes` | Schema.org article type URIs |
| `rxCDATA` | CDATA section wrappers |
| `rxSchemaOrg` | schema.org domain check |
| `rxAdWords` | Ad-related class/id terms (used in `cleanConditionally`) |
| `rxLoadingWords` | Loading placeholder class/id terms |

**Tests**: Port `internal/re2go/re2go_test.go` faithfully:
- Known-positive inputs for each pattern
- Known-negative inputs
- `NormalizeSpaces` whitespace collapsing

**Acceptance criteria**:
- [ ] All re2go tests pass
- [ ] `cargo clippy` clean

---

### Phase 3: Utilities + Traversal Helpers

> **Status: 🔲 TODO**
> **Go source**: `utils.go`, `traverse.go`
> **Rust output**: `src/utils.rs`, `src/traverse.rs`
> **Porting order**: `utils.go` top-to-bottom, then `traverse.go` top-to-bottom

**Goal**: Port `utils.go` and `traverse.go` — the text and DOM utilities used throughout the parser.

**Dependencies**: Phase 1 (DOM), Phase 2 (regexp for `normalize_spaces` and `rxTokenize`).

**`src/utils.rs`** (port of `utils.go`):

```rust
/// Port of hasContent — returns true if string has non-whitespace
pub fn has_content(s: &str) -> bool

/// Port of wordCount — split on whitespace, count words
pub fn word_count(s: &str) -> usize

/// Port of charCount — simple Unicode codepoint count (NOT the same as charCounter in traverse.rs)
/// Used for: byline length checks, title string comparisons, JSON-LD field sizes
pub fn char_count(s: &str) -> usize  // s.chars().count()

/// Port of indexOf — find string in slice
pub fn index_of(slice: &[&str], target: &str) -> Option<usize>

/// Port of toAbsoluteURI — resolve relative URL against base, return absolute
pub fn to_absolute_uri(uri: &str, base: &url::Url) -> String

/// Port of isValidURL — check if string is a parseable URL
pub fn is_valid_url(s: &str) -> bool

/// Port of textSimilarity — token-based similarity between two strings (0.0..=1.0)
/// Uses rxTokenize (\W+) to split, NOT str::split_whitespace()
/// Algorithm: |intersection(tokens_a, tokens_b)| / max(|tokens_a|, |tokens_b|)
pub fn text_similarity(a: &str, b: &str) -> f64

/// Port of strOr — return first non-empty string from the slice
pub fn str_or<'a>(candidates: &[&'a str]) -> &'a str
```

**`src/traverse.rs`** (port of `traverse.go`):

The `charCounter` struct in `traverse.go` is a whitespace-normalizing character counter. It is **distinct** from `char_count` in `utils.rs`:
- `char_count` (utils.rs): simple `s.chars().count()`. Used for byline/title length checks.
- `charCounter` / `count_chars_and_commas` (traverse.rs): strips leading/trailing whitespace, collapses consecutive whitespace runs to count as a single space, counts non-space chars plus inter-word spaces. Used for scoring and link density.

```rust
/// Port of hasTextContent — true if node or any descendant has non-whitespace text
pub fn has_text_content(doc: &Document, id: NodeId) -> bool

/// Port of countCharsAndCommas — whitespace-normalizing char/comma count across all text in subtree.
/// Uses charCounter semantics: collapses whitespace, strips leading/trailing.
/// Returns (char_count, comma_count).
/// Also used internally by getLinkDensity and cleanConditionally.
pub fn count_chars_and_commas(doc: &Document, id: NodeId) -> (usize, usize)
```

**Tests**: Unit tests for each function, especially:
- `text_similarity("", "")` = 0.0 (no divide-by-zero)
- `to_absolute_uri` with relative, absolute, and protocol-relative URLs
- `count_chars_and_commas` on a subtree with nested elements
- `char_count` vs `count_chars_and_commas` give different results for whitespace-heavy inputs

**Acceptance criteria**:
- [ ] All utils unit tests pass
- [ ] All traverse unit tests pass
- [ ] `cargo clippy` clean

---

### Phase 4: Article Type + Render (InnerText)

> **Status: 🔲 TODO**
> **Go source**: `article.go`, `render/inner_text.go`, `inspect_node.go`
> **Rust output**: `src/article.rs`, `src/render/mod.rs`, `src/inspect.rs`
> **Porting order**: `article.go` → `render/inner_text.go` → `inspect_node.go` (stub ok)

**Goal**: Port `article.go` and `render/inner_text.go`. The `inner_text` function is how the final article text is produced — its correctness directly affects output quality.

**Dependencies**: Phase 1 (DOM).

**`src/article.rs`** (port of `article.go`):

The Go `Article` type holds a `*html.Node` DOM node and produces HTML/text via `RenderHTML`/`RenderText` methods. Our Rust version eagerly serializes to strings at construction time.

```rust
#[derive(Debug, Clone, Default)]
pub struct Article {
    pub title: String,
    pub byline: String,
    /// Excerpt from metadata. If empty at construction time, fall back to inner text
    /// of the first <p> child of the article node (port of Go's Excerpt() accessor).
    pub excerpt: String,
    pub site_name: String,
    pub image: String,
    pub favicon: String,
    pub language: String,
    pub published_time: String,
    pub modified_time: String,
    /// Cleaned article HTML (article node serialized via outer_html)
    pub content: String,
    /// Plain text via InnerText algorithm
    pub text_content: String,
    /// Character count of text_content
    pub length: usize,
    /// Direction: "ltr", "rtl", or ""
    pub dir: String,
}
```

The `excerpt` fallback: if no excerpt was found in metadata, extract the inner text of the first `<p>` in the article content (same as Go's lazy `Excerpt()` accessor).

**`src/render/mod.rs`** (port of `render/inner_text.go`):

```rust
/// Port of InnerText — extract plain text respecting visual layout.
///
/// Differences from textContent:
/// - Block elements (p, div, h1-h6, table, ul, ol, etc.) add newlines before/after
/// - Table cells (td, th) add tabs
/// - display:none elements are skipped entirely
/// - MathJax/LaTeX: output LaTeX source, not rendered text
/// - Consecutive whitespace collapsed to single space
pub fn inner_text(doc: &Document, id: NodeId) -> String
```

Block-level elements that trigger newlines (from Go source):
- `article`, `aside`, `br`, `dd`, `details`, `dt`, `figcaption`, `figure`
- `footer`, `h1`-`h6`, `header`, `hr`, `li`, `main`, `nav`
- `ol`, `p`, `pre`, `section`, `summary`, `table`, `ul`, `div`, `blockquote`
- `dl`, `fieldset`, `form`, `tfoot`, `thead`, `caption`, `tr`

Tab-separated: `td`, `th`

**Tests**: Port `render/inner_text_test.go`. Key cases:
- Block elements produce newlines
- `display:none` content is absent
- MathJax elements output `\(...\)` or `$$...$$` LaTeX
- Nested blocks don't double-add newlines
- Text nodes inside inline elements are included

**Acceptance criteria**:
- [ ] All inner_text tests pass
- [ ] `cargo clippy` clean

---

### Phase 5: Core Parser — Preparation + Metadata

> **Status: 🔲 TODO**
> **Go source**: `parser.go` lines 1–~800 (struct definition, `prepDocument`, `replaceBrs`, `unwrapNoscriptImages`, `fixLazyImages`, `getArticleTitle`, `getArticleMetadata`, `getJSONLD`, `getArticleFavicon`)
> **Rust output**: `src/parser.rs` (first section)
> **Porting order**: Follow `parser.go` top-to-bottom. Port the `Parser` struct first, then each function in declaration order.

**Goal**: Port the first section of `parser.go` — document preparation, metadata extraction, and JSON-LD parsing. This is everything that runs *before* `grab_article`.

**Dependencies**: Phases 1–4.

**Source**: `parser.go` lines 1–~800, roughly.

**Parser struct**:

```rust
pub struct Parser {
    pub max_elems_to_parse: usize,  // 0 = unlimited; default 0
    pub n_top_candidates: usize,    // default 5
    pub char_threshold: usize,      // default 500
    pub classes_to_preserve: Vec<String>,  // default ["page"]
    pub keep_classes: bool,         // default false
    pub tags_to_score: Vec<String>, // default: ["section","h2","h3","h4","h5","h6","p","td","pre"]
    pub disable_json_ld: bool,      // default false
    pub allowed_video_regex: Option<regex::Regex>,
}
```

**Functions to port in this phase**:

| Go function | Description |
|-------------|-------------|
| `prepDocument` | Remove scripts, styles, fonts; replace `<br>` chains; unwrap noscript images |
| `replaceBrs` | Convert consecutive `<br>` tags into `<p>` elements |
| `unwrapNoscriptImages` | Extract `<img>` from `<noscript>` tags (lazy-loaded images) |
| `fixLazyImages` | Copy `data-src`/`data-srcset` to `src`/`srcset` |
| `getArticleTitle` | Extract title from `<title>`, `og:title`, `dc:title`, etc. |
| `getArticleMetadata` | Extract all metadata from `<meta>` tags |
| `getJSONLD` | Parse JSON-LD `<script type="application/ld+json">` for Schema.org metadata |
| `getArticleFavicon` | Find favicon URL |

**`replaceBrs` algorithm** (critical — port line-by-line from Go source):
1. Find all `<br>` elements (snapshot list before mutation)
2. For each `<br>`, check if it starts a chain: iterate next siblings collecting those that are `<br>` elements or whitespace-only text nodes
3. If chain length ≥ 2: replace the leading `<br>` with a new `<p>` element, then iterate further siblings (after the chain), moving each into the new `<p>` until hitting another `<br>` chain or a block element — at that point stop
4. Remove all the original `<br>` elements in the chain

The key detail: after the chain, the following content (text nodes, inline elements) gets moved into the new `<p>`, not just wrapped.

**`getArticleTitle` algorithm**: Port line-by-line from Go source. The function is ~100 lines with complex logic: get `<title>` text → try splitting on separators (`|`, `-`, `--`, `\`, `/`, `»`) → check word count of each part → compare similarity against `og:title` → trim site name → handle hierarchy separators. Do not summarize — read the Go source.

**`getJSONLD`**: Parse `<script type="application/ld+json">`. Handle `@graph` arrays. Extract: `name`/`headline`, `author.name`, `description`, `publisher.name`, `datePublished`, `dateModified`, `image.url`.

**Tests**:
- `prep_document` correctly removes scripts/styles
- `replace_brs` correctly converts `<br><br>` to `<p>` (port the Go test cases)
- `get_article_title` handles different meta tag combinations
- `get_json_ld` handles `@graph`, nested authors, missing fields

**Acceptance criteria**:
- [ ] All unit tests for this phase pass
- [ ] `cargo clippy` clean

---

### Phase 6: Core Parser — Article Extraction (`grab_article`)

> **Status: 🔲 TODO**
> **Go source**: `parser.go` lines ~800–1400 (`grabArticle` and its helper functions: `getNextNode`, `removeAndGetNext`, `initializeNode`, `getClassWeight`, `getLinkDensity`, `getContentScore`, `setContentScore`, `hasContentScore`, `getDataTable`, `markDataTables`, `hasAncestorTag`, `isProbablyVisible`, `isPhrasingContent`, `isElementWithoutContent`, `hasSingleTagInsideElement`, `hasChildBlockElement`, `getInnerText`, `checkByline`)
> **Rust output**: `src/parser.rs` (second section, appended after Phase 5 output)
> **Porting order**: Port helpers first in the order they appear in the Go file, then `grabArticle` last.

**Goal**: Port `grab_article` — the heart of the algorithm. This is the most complex single function (~600 lines in Go).

**Dependencies**: Phases 1–5.

**Functions to port in this phase**:

| Go function | Description |
|-------------|-------------|
| `grabArticle` | Main extraction: multi-pass scoring and candidate selection |
| `getNextNode` | Depth-first traversal returning the next node (see below) |
| `removeAndGetNext` | Remove current node and return its logical next in traversal |
| `initializeNode` | Set initial content score by tag type + class weight |
| `getClassWeight` | Class/id weight: checks class AND id separately, each ±25; total range ±50 |
| `getLinkDensity` | Fraction of text in `<a>` tags using `charCounter` (same-page links weighted 0.3×) |
| `getContentScore` / `setContentScore` | Read/write score from `HashMap<NodeId, f64>` side table |
| `hasContentScore` | Check if node has a score: `scores.contains_key(&id)` — NOT a comparison to 0.0 |
| `getDataTable` / `markDataTables` | Identify data tables vs layout tables |
| `hasAncestorTag` | Check if node has ancestor with given tag (with optional depth limit and filter fn) |
| `isProbablyVisible` | Check for hidden elements |
| `isPhrasingContent` | Is this an inline (phrasing) element? |
| `isElementWithoutContent` | Empty or only br/hr children? |
| `hasSingleTagInsideElement` | Single child of specific type? |
| `hasChildBlockElement` | Has block-level children? Uses `DIV_TO_P_ELEMS` set |
| `getInnerText` | Text content with optional whitespace normalization via `normalize_spaces` |
| `checkByline` | Is this element a byline (author)? |

#### Traversal pattern — `getNextNode` / `removeAndGetNext` (CRITICAL)

`grab_article` does NOT iterate over a pre-collected flat list of elements. It uses a custom depth-first traversal that **mutates the tree during iteration**. Porting this incorrectly is the single most likely source of algorithmic failure.

```
getNextNode(node, ignore_self_and_children):
  if not ignore_self_and_children and node has first_child:
    return first_child
  if node has next_sibling:
    return next_sibling
  // walk up until we find an ancestor with a next sibling
  parent = node.parent
  while parent != nil:
    if parent has next_sibling:
      return parent.next_sibling
    parent = parent.parent
  return nil

removeAndGetNext(node):
  next = getNextNode(node, ignore_self_and_children=true)
  node.detach()   // removes from tree but NodeId stays valid
  return next
```

The outer loop in `grab_article` becomes:
```
node = first child of doc
while node != nil:
  if should_remove(node):
    node = removeAndGetNext(node)
    continue
  node = getNextNode(node, false)
```

In Rust, implement `get_next_node(doc, id, ignore_self_and_children: bool) -> Option<NodeId>` and `remove_and_get_next(doc, id) -> Option<NodeId>`.

#### Document cloning per pass

Each pass of the multi-pass loop operates on a **fresh clone** of the original document (Go: `doc := dom.Clone(ps.doc, true)`). Without this, mutations from pass 1 (removed bylines, converted divs, etc.) would carry over to pass 2. The `Document::clone()` method added in Phase 1 is used here.

#### Div-to-P conversion — three paths (port line-by-line from Go ~lines 876–931)

When `grab_article` encounters a `<div>`:
1. **Has phrasing content children but no block children**: iterate children, group runs of phrasing content into new `<p>` wrappers. Whitespace-only text nodes adjacent to block elements are dropped. This is the most complex path.
2. **Single `<p>` child with low link density**: replace the `<div>` with its `<p>` child (promoting it, preserving any id/class from the div — check Go source for exact behavior).
3. **No block-level children** (checked via `hasChildBlockElement` using `DIV_TO_P_ELEMS`): change tag to `<p>` via `set_tag_name`.

#### Scoring algorithm

**Initial node score by tag** (`initializeNode`):
- `div`: +5
- `pre`, `td`, `blockquote`: +3
- `address`, `ol`, `ul`, `dl`, `dd`, `dt`, `li`, `form`: -3
- `h1`–`h6`, `th`: -5

**Class weight** (`getClassWeight`): checks `class` attribute AND `id` attribute separately. Each can contribute ±25. Total range is **-50 to +50** (not ±25). Returns 0 if `use_weight_classes` flag is false (this check is inside `getClassWeight` itself).

**Per-paragraph content score**:

```
// NOTE: Readability.js has a bug: it always adds 1 to comma count
// The Go port faithfully replicates this bug
content_score = 1 + (num_commas + 1) + min(num_chars / 100, 3)
// Simplified: content_score = 2 + num_commas + min(num_chars / 100, 3)
if use_weight_classes: content_score += get_class_weight(elem)
```

**Score propagation to ancestors** (up to depth 5):
- Level 0 (parent): divide by 1
- Level 1 (grandparent): divide by 2
- Level 2+ (great-grandparent+): divide by (level × 3)

**`getLinkDensity`**: Uses `charCounter` struct (whitespace-normalizing counting), NOT simple `str.len()`. Iterates the tree counting total chars and chars inside `<a>` elements. Same-page links (`href` starts with `#`) are weighted at 0.3×.

**Final adjustments**:
- Scale each top candidate score by `(1 - link_density)`
- Walk up tree if parent has same or higher score (prefer broader container)
- Include siblings with score ≥ 20% of top candidate score
- Also include siblings that are `<p>` or `<div>` with low link density and sufficient text length

**Phase 6 acceptance criteria**:
- [ ] Scoring loop produces correct candidate scores (verify via debug logging against go-readability on same inputs)
- [ ] `stub prepArticle` (no-op) allows Phase 6 to compile — full fixture tests come in Phase 7
- [ ] `cargo clippy` clean

> **Note**: Full fixture tests require `prepArticle` (Phase 7). Phase 6 acceptance does NOT require fixture tests.

---

### Phase 7: Core Parser — Article Cleaning

> **Status: 🔲 TODO**
> **Go source**: `parser.go` lines ~1400–end (`prepArticle`, `postProcessContent`, `clean`, `cleanConditionally`, `cleanStyles`, `cleanClasses`, `fixRelativeURIs`, `simplifyNestedElements`, `markDataTables`)
> **Rust output**: `src/parser.rs` (third and final section)
> **Porting order**: Follow `parser.go` top-to-bottom through the remaining functions.

**Goal**: Port `prepArticle`, `postProcessContent`, and all conditional cleaning functions.

**Dependencies**: Phase 6.

**Critical distinction — `prepArticle` vs `postProcessContent`**:

These are two separate functions called at different times:
- `prepArticle` is called **inside** `grab_article`, on the candidate article node, before the `char_threshold` check. It runs on each pass's candidate.
- `postProcessContent` is called **after** `grab_article` returns, once on the final accepted result. It handles URL fixing, element simplification, and class cleanup.

Putting `fixRelativeURIs`, `simplifyNestedElements`, or `cleanClasses` inside `prepArticle` would be wrong.

**Functions to port**:

| Go function | Called from | Description |
|-------------|-------------|-------------|
| `prepArticle` | `grabArticle` (inner loop) | Runs cleaning steps on the candidate article node |
| `postProcessContent` | `ParseAndMutate` (outer, after grabArticle) | URL fixing, simplification, class stripping |
| `clean` | `prepArticle` | Remove all elements of a tag (except video embeds) |
| `cleanConditionally` | `prepArticle` | Remove elements if "fishy" by heuristics |
| `cleanClasses` | `postProcessContent` | Remove class attributes (keep `classes_to_preserve`) |
| `cleanStyles` | `prepArticle` | Remove presentational style attributes |
| `fixRelativeURIs` | `postProcessContent` | Convert relative URLs in href/src/srcset to absolute |
| `simplifyNestedElements` | `postProcessContent` | Unwrap redundant single-child div/section wrappers |
| `markDataTables` | `prepArticle` | Mark tables as data vs layout tables |

**`prepArticle` sequence** (must be exactly this order — port line-by-line):
1. `cleanStyles`
2. `markDataTables`
3. `fixLazyImages` (again, in case any survived `prepDocument`)
4. Remove unlikely elements that are not tables/figures/embeds
5. Remove form elements
6. `clean("object")`, `clean("embed")`, `clean("footer")`, `clean("link")`, `clean("aside")`
7. `cleanConditionally("form")`
8. `cleanConditionally("fieldset")`
9. `clean("h1")` if title similarity is high
10. `clean("h2")` if there are many h2s (unlikely article structure)
11. `cleanConditionally("table")`, `cleanConditionally("ul")`, `cleanConditionally("div")`
12. Remove extra paragraphs with no content
13. **Stop here** — `simplifyNestedElements` and `cleanClasses` are NOT called here; they are in `postProcessContent`

**`postProcessContent` sequence**:
1. `fixRelativeURIs`
2. `simplifyNestedElements`
3. `cleanClasses`
4. `clearReadabilityAttr` — with the HashMap approach, this is a no-op (no DOM attributes to clear)

**`cleanConditionally` algorithm** (port line-by-line from Go source ~lines 2108–2340 — do NOT use the simplified sketch below as a substitute):

The function is ~230 lines. Key structure:
- Skip if `tag == "table"` and node is in the data-table set
- Compute class weight (if `use_weight_classes`)
- Perform a single-pass tree walk collecting: `chars`, `textChars`, `listChars`, `headingChars`, `linkCharsWeighted`, `commas`, `pCount`, `imgCount`, `liCount`, `inputCount`, `embedCount`
- Also collect: `adWords` match (via `rxAdWords`), `loadingWords` match (via `rxLoadingWords`)
- Compute derived ratios: `textDensity`, `linkDensity`, `headingDensity`, `listDensity`
- Remove conditions include (but are not limited to): low text density + low char count, high link density + low weight, input-heavy forms, list-heavy with high link density, figcaption special case, embed + link density combinations
- The exact thresholds and conditions must come from the Go source. Do not guess.

**Tests**: After this phase, run the full fixture test suite and fix failures. Use `pretty_assertions` to see exactly what differs.

**Acceptance criteria**:
- [ ] At least 80 of 134 fixture tests pass
- [ ] `cargo clippy` clean

---

### Phase 8: Full Integration + Public API

> **Status: 🔲 TODO**
> **Go source**: `readability.go`, `parser-parse.go`, `parser-check.go`
> **Rust output**: `src/lib.rs`, `src/parser.rs` (Parse/Check methods appended)
> **Porting order**: `parser-parse.go` → `parser-check.go` → `readability.go`

**Goal**: Wire everything together into the public API. Port `readability.go`, `parser-parse.go`, and `parser-check.go`. Run all 134 fixture tests.

**Dependencies**: Phases 1–7.

**`src/lib.rs`** (port of `readability.go`):

```rust
/// Parse readable content from an HTML string.
/// Internally clones the document before mutating.
/// Port of `FromReader` / `FromDocument`.
pub fn from_html(html: &str, url: Option<&url::Url>) -> Result<Article, Error>

/// Parse readable content, mutating the document in place (no internal clone).
/// Port of `ParseAndMutate`.
pub fn from_html_mut(html: &str, url: Option<&url::Url>) -> Result<Article, Error>

/// Check whether the document appears to be readable without full parsing.
/// Port of `CheckDocument`.
pub fn is_readable(html: &str) -> bool
```

Both `from_html` and `from_html_mut` take `&str`, so both must parse the HTML. The distinction: `from_html` clones the parsed document before calling `grab_article` (so the caller's HTML is unaffected), `from_html_mut` skips the clone.

**`Parser` configuration**:

```rust
impl Parser {
    pub fn new() -> Self
    pub fn with_char_threshold(mut self, n: usize) -> Self
    pub fn with_n_top_candidates(mut self, n: usize) -> Self
    pub fn with_keep_classes(mut self, keep: bool) -> Self
    pub fn with_classes_to_preserve(mut self, classes: Vec<String>) -> Self
    pub fn with_max_elems_to_parse(mut self, n: usize) -> Self

    pub fn parse(&self, html: &str, url: Option<&url::Url>) -> Result<Article, Error>
    pub fn check(&self, html: &str) -> bool
}
```

**Fixture test structure**:

```rust
// tests/fixtures_test.rs

fn run_fixture(name: &str) {
    let source = read_fixture(name, "source.html");
    let expected_html = read_fixture(name, "expected.html");
    let expected_meta = read_fixture_metadata(name);  // Option<ExpectedMetadata>

    let article = readability::from_html(&source, None).expect("extraction failed");

    assert_eq!(normalize_html(&article.content), normalize_html(&expected_html));

    if let Some(meta) = expected_meta {
        if !meta.title.is_empty() { assert_eq!(article.title, meta.title); }
        if !meta.byline.is_empty() { assert_eq!(article.byline, meta.byline); }
        // etc.
    }
}

#[test] fn fixture_001() { run_fixture("001"); }
#[test] fn fixture_002() { run_fixture("002"); }
// ... all 134
```

**HTML normalization strategy**: `scraper`/`html5ever` serializes HTML differently from Go's `net/html` (attribute ordering, void elements, etc.). To make fixture comparison work:

**Recommended approach**: Parse both `article.content` and `expected_html` through `scraper::Html::parse_fragment`, then serialize both using the same serializer. This normalizes attribute order and void element handling to the same baseline. If this still produces spurious failures, fall back to comparing `text_content` (plain text) for content equivalence and verify structural element presence separately (h1 count, p count, img count).

Do NOT attempt byte-for-byte HTML comparison without normalization.

**Acceptance criteria**:
- [ ] At least 110 of 134 fixture tests pass (some may differ due to serialization differences)
- [ ] `is_readable` returns correct results on the same fixtures
- [ ] `cargo clippy` clean

---

### Phase 9: Benchmarks + Integration with trafilatura-rs

> **Status: 🔲 TODO**
> **Go source**: *(none — new Rust benchmarks + trafilatura-rs wiring)*
> **Rust output**: `benches/extraction.rs`, trafilatura-rs changes

**Goal**: Add benchmarks. Measure performance vs. go-readability. Update trafilatura-rs to use this crate as its fallback engine and verify the recall improvement.

**Dependencies**: Phase 8.

**`benches/extraction.rs`**:

```rust
fn bench_single_document(c: &mut Criterion) {
    // Benchmark on a few representative test-pages fixtures
    // (small, medium, large HTML)
}

fn bench_all_fixtures(c: &mut Criterion) {
    // Run all 134 fixtures
}
```

**Go reference performance**: go-readability processes 960 documents in ~1.6s (Readability mode in the go-trafilatura comparison tool).

**trafilatura-rs integration**:
1. Add `readability = { path = "../readability-rs" }` to trafilatura-rs `Cargo.toml`
2. Replace the `readable-readability` fallback in `src/extraction/external.rs`
3. Run the trafilatura-rs comparison suite and verify recall improves from ~0.896 toward ~0.921+

**Acceptance criteria**:
- [ ] Benchmark suite runs without errors
- [ ] Full fixture run completes in < 5s (single-threaded)
- [ ] trafilatura-rs recall improves by ≥ 1.5pp in balanced+fallback mode
- [ ] No trafilatura-rs tests regress

---

## Implementation Order Summary

```
Phase 1: DOM abstraction
  ↓
Phase 2: Regexp patterns (re2go + parser.go patterns)
  ↓
Phase 3: Utilities + traversal
  ↓
Phase 4: Article type + InnerText render
  ↓
Phase 5: Parser preparation + metadata
  ↓
Phase 6: grab_article (scoring + selection) — stub prepArticle
  ↓
Phase 7: Article cleaning (prepArticle + postProcessContent)
  ↓
Phase 8: Public API + full fixture tests
  ↓
Phase 9: Benchmarks + trafilatura-rs integration
```

Phases 1–4 can be done in any order relative to each other but must all complete before Phase 5. Phases 5–8 are strictly sequential.

---

## Risk Areas

### 1. `getNextNode` Traversal + Mutation During Iteration (Phase 6 — CRITICAL RISK)
**Risk**: `grab_article` mutates the DOM tree while traversing it using a custom depth-first traversal (`getNextNode`). Using a standard iterator or pre-collecting elements will produce completely wrong behavior — removed nodes will still be visited, and the traversal order after removal will be wrong.
**Mitigation**: Implement `get_next_node(doc, id, ignore_self_and_children)` faithfully as documented in Phase 6. Always call `remove_and_get_next` when removing a node during traversal; never use `doc.remove()` directly in the traversal loop.

### 2. `cleanConditionally` Complexity (Phase 7 — HIGH RISK)
**Risk**: The plan's sketch of `cleanConditionally` omits most of the actual logic. The real function is ~230 lines with many heuristic branches, multiple density ratios, and pattern matches. Implementing from the sketch will produce systematically wrong cleaning decisions.
**Mitigation**: Port `cleanConditionally` strictly line-by-line from the Go source. Do not simplify, combine, or reorder the conditions.

### 3. HTML Serialization Differences (Phase 8 — HIGH RISK)
**Risk**: `scraper`/`html5ever` serializes HTML differently than Go's `net/html` (attribute ordering, void elements, whitespace). Fixture comparisons will fail even if logic is correct.
**Mitigation**: Parse both expected and actual through `scraper` before comparing, to normalize to the same serializer. See Phase 8 normalization strategy.

### 4. `prepArticle` vs `postProcessContent` Ordering (Phase 7 — HIGH RISK)
**Risk**: `fixRelativeURIs`, `simplifyNestedElements`, and `cleanClasses` are called in `postProcessContent` (after `grabArticle` returns), not in `prepArticle` (which runs inside the scoring loop). Calling them in the wrong place will affect scoring across passes.
**Mitigation**: Phase 7 explicitly separates these two functions. Follow the documented call sites.

### 5. Document Cloning Per Pass (Phase 6 — HIGH RISK)
**Risk**: If each `grab_article` pass doesn't clone the document, mutations from one pass pollute the next.
**Mitigation**: Each pass clones the document using `Document::clone()` before beginning its traversal. Verify this is happening.

### 6. `replace_brs` Correctness (Phase 5 — MEDIUM RISK)
**Risk**: The `<br>` chain-to-`<p>` conversion is stateful and tricky to port exactly. The plan sketch omits the "move following siblings into the new `<p>`" step.
**Mitigation**: Port line-by-line from Go source. Port the test cases from `test-pages/replace-brs/` first.

### 7. `grab_article` Ancestor Tracking (Phase 6 — MEDIUM RISK)
**Risk**: Off-by-one in the level/divisor calculation produces wrong scores.
**Mitigation**: Add debug logging of candidate scores and cross-check against go-readability on the same inputs.

### 8. `is_negative_class` Anchored Pattern (Phase 2 — LOW RISK)
**Risk**: The `IsNegativeClass` pattern requires two separate patterns — anchored `(^| )(hid|hidden|d-none)( |$)` and a substring match for the rest.
**Mitigation**: Port as two `LazyLock<Regex>` OR'd in the `is_negative_class` function.

### 9. `inner_text` Block Element Handling (Phase 4 — MEDIUM RISK)
**Risk**: Block-level element list must match Go exactly. Any differences cause text content mismatches.
**Mitigation**: Use the exact list documented above. Port `render/inner_text_test.go` first.

---

## Key Constants (from parser.go)

```rust
const DEFAULT_CHAR_THRESHOLD: usize = 500;
const DEFAULT_N_TOP_CANDIDATES: usize = 5;
const DEFAULT_MAX_ELEMS_TO_PARSE: usize = 0;  // unlimited
const DEFAULT_CLASSES_TO_PRESERVE: &[&str] = &["page"];

// Elements that count as "block children" when deciding whether to convert <div> to <p>
// Source: parser.go divToPElems
const DIV_TO_P_ELEMS: &[&str] = &[
    "blockquote", "dl", "div", "img", "ol", "p", "pre", "table", "ul", "select"
];

// Tags exempt from div→p conversion
// Source: parser.go alterToDivExceptions
const ALTER_TO_DIV_EXCEPTIONS: &[&str] = &["div", "article", "section", "p", "ol", "ul"];

const PRESENTATIONAL_ATTRIBUTES: &[&str] = &[
    "align", "background", "bgcolor", "border", "cellpadding",
    "cellspacing", "frame", "hspace", "rules", "style", "valign", "vspace",
];

const DEPRECATED_SIZE_ATTRIBUTE_ELEMS: &[&str] = &["table", "th", "td", "hr", "pre"];

const PHRASING_ELEMS: &[&str] = &[
    "abbr", "audio", "b", "bdo", "br", "button", "cite", "code",
    "data", "datalist", "dfn", "em", "embed", "i", "img", "input",
    "kbd", "label", "mark", "math", "meter", "noscript", "object",
    "output", "picture", "progress", "q", "ruby", "s", "samp",
    "script", "select", "small", "span", "strong", "sub", "sup",
    "textarea", "time", "u", "var", "wbr",
];

// Video hosts whose embeds should be preserved
const VIDEOS: &[&str] = &[
    "dailymotion.com", "youtube.com", "youtube-nocookie.com",
    "player.vimeo.com", "v.qq.com", "bilibili.com",
    "live.bilibili.com", "archive.org", "upload.wikimedia.org",
    "player.twitch.tv",
];
```
