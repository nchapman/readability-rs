# Intentional Deviations from go-readability

This document records every place where the Rust port **deliberately** differs from the Go
source. Each entry explains what changed, why, and what the behavioral impact is.

All other divergences should be treated as bugs and fixed.

---

## 1. Score side-table instead of DOM attributes

**Go**: stores candidate scores as `data-readability-score` string attributes on DOM nodes.

**Rust**: uses a `HashMap<NodeId, f64>` side table local to each `grab_article` pass.

**Why**: Avoids repeated float→string→float round-trips, keeps the DOM clean, and is idiomatic
for Rust. The `clear_readability_attr` function is a no-op in Rust (the HashMap is simply
dropped / cleared).

**Behavioral impact**: None. The numeric values are identical.

---

## 2. Readability-table flag as `HashSet` instead of DOM attribute

**Go**: marks table nodes with a `data-readability-table` attribute.

**Rust**: uses a `HashSet<NodeId>`.

**Why**: Same rationale as the score side-table.

**Behavioral impact**: None.

---

## 3. `RX_TOKENIZE` uses `[^a-zA-Z0-9_]+` instead of `\W+`

**Go**: `rxTokenize = regexp.MustCompile("(?i)\\W+")`. Go's RE2 engine treats `\W` as
ASCII-only (`[^a-zA-Z0-9_]`), so Unicode letters (including CJK) are word separators.

**Rust**: `Regex::new(r"[^a-zA-Z0-9_]+")`. The `regex` crate's `\W` is Unicode-aware and
would treat CJK characters as word chars, changing tokenisation results.

**Why**: Explicit ASCII character class replicates Go's behavior without the Unicode mismatch.
Using `(?-u)\W+` was considered but rejected because `(?-u)` operates on bytes, which would
split multi-byte UTF-8 sequences at byte boundaries (corrupting codepoints).

**Behavioral impact**: Identical to Go for all inputs. This was the fix for the `qq` fixture.

---

## 4. Eager content serialization in `Article`

**Go**: `Article.Node` holds the `*html.Node` tree; callers render via `RenderHTML()` /
`RenderText()` / the `render` package on demand.

**Rust**: `Article.content` (HTML string) and `Article.text_content` (plain text string) are
populated eagerly during `parse_and_mutate`.

**Why**: Avoids lifetime/ownership complexity of returning a tree node, and matches the
expected usage pattern (consumers read strings, not DOM trees).

**Behavioral impact**: None for the extraction result. Callers cannot perform further DOM
mutations on the article node (not a documented use case).

---

## 5. Extra fields on `Article`

**Go**: no `dir` or `length` fields.

**Rust**: `Article` has `pub dir: String` (text direction, e.g. `"rtl"`) and `pub length: usize`
(UTF-8 character count of `text_content`).

**Why**: Convenience fields for callers; `dir` carries information already extracted during
parsing. Adding them does not break any Go-equivalent behavior.

**Behavioral impact**: Additive only. Go-equivalent behavior is unchanged.

---

## 6. No `PublishedTime()` / `ModifiedTime()` methods

**Go**: `Article.PublishedTime() (time.Time, error)` and `ModifiedTime()` parse the stored
timestamp string via `dateparse.ParseAny`, returning `ErrTimestampMissing` for empty strings.

**Rust**: `Article.published_time` and `modified_time` are raw `String` fields. No typed
parsing method is exposed.

**Why**: The raw strings contain all the information. Parsing is the caller's responsibility.
The `ErrTimestampMissing` sentinel has no idiomatic Rust equivalent given that an empty
`Option<String>` / empty string serves the same purpose.

**Behavioral impact**: None for extraction. Callers needing parsed dates must call
`dateparser::parse` themselves.

---

## 7. No `FromURL` HTTP fetcher

**Go**: `FromURL(pageURL string, timeout time.Duration, ...) (Article, error)` fetches the
page via HTTP, validates `Content-Type: text/html`, and parses.

**Rust**: Not implemented.

**Why**: The library is synchronous and does not import an HTTP client. HTTP fetching is a
higher-level concern left to the caller. (See CLAUDE.md: "Do not use async in the library.")

**Behavioral impact**: None. Callers use `Parser::parse` with pre-fetched HTML.

---

## 8. No `FromDocument`, `FromReader`, `CheckDocument` free functions

**Go**: convenience free functions wrapping `Parser` methods.

**Rust**: callers use `Parser::parse`, `Parser::parse_document`, and `Parser::check_document` /
`Parser::check_html` directly.

**Why**: The extra wrappers add API surface without adding functionality. Rust callers
instantiate `Parser` explicitly.

**Behavioral impact**: None (same functionality, different call site).

---

## 9. `\s` / `\S` in parser patterns are Unicode-aware in Rust

Several patterns (e.g. `RX_HAS_CONTENT`, `RX_DISPLAY_NONE`, `RX_PROPERTY_PATTERN`) use `\s`
or `\S`. Go's RE2 treats these as ASCII-only; Rust's `regex` crate treats them as Unicode.

**Why not fixed**: In every context where these patterns are applied (HTML attribute values,
CSS style strings, URLs), the input contains only ASCII whitespace. The Unicode difference
is inconsequential in practice.

**Behavioral impact**: None for any real-world HTML document.

---

## 10. `text_similarity` returns `0.0` for empty inputs (vs Go's potential `NaN`)

**Go**: when both inputs tokenise to empty, `charCount("") / charCount("")` = `0/0` could
produce `NaN`; `1 - NaN = NaN`. In practice this path is unreachable since both the JSON-LD
title check and `headerDuplicatesTitle` always have non-empty inputs.

**Rust**: returns `0.0` explicitly when `merged_b_len == 0`.

**Why**: `NaN` propagation would break comparison operators. `0.0` (completely dissimilar) is
the only sensible sentinel for "no content to compare."

**Behavioral impact**: None in practice (empty-input path is unreachable).
