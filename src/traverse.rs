// Port of go-readability/traverse.go

use ego_tree::NodeId;
use scraper::Node;

use crate::dom::Document;
use crate::utils::has_content;

/// Port of hasTextContent — true if the node or any descendant has non-whitespace text.
pub fn has_text_content(doc: &Document, id: NodeId) -> bool {
    let Some(node) = doc.html.tree.get(id) else {
        return false;
    };
    match node.value() {
        Node::Text(text) => has_content(&text.text),
        _ => {
            for child in node.children() {
                if has_text_content(doc, child.id()) {
                    return true;
                }
            }
            false
        }
    }
}

/// Port of countCharsAndCommas — whitespace-normalizing character and comma count
/// across all text content in the subtree.
///
/// Semantics (from Go's `charCounter`):
/// - Leading and trailing whitespace is not counted.
/// - Consecutive runs of whitespace between words are counted as a single space.
///
/// Returns `(char_count, comma_count)`.
///
/// This is distinct from `utils::char_count`, which simply counts Unicode codepoints.
/// This version is used for scoring and link density calculations.
pub fn count_chars_and_commas(doc: &Document, id: NodeId) -> (usize, usize) {
    let mut chars = CharCounter::new();
    let mut commas: usize = 0;

    walk_text(doc, id, &mut |text: &str| {
        for r in text.chars() {
            chars.count(r);
            if is_comma(r) {
                commas += 1;
            }
        }
    });

    (chars.total(), commas)
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Walk all text nodes in a subtree, calling `f` with each text string.
fn walk_text<F: FnMut(&str)>(doc: &Document, id: NodeId, f: &mut F) {
    let Some(node) = doc.html.tree.get(id) else {
        return;
    };
    match node.value() {
        Node::Text(text) => f(&text.text),
        _ => {
            for child in node.children() {
                walk_text(doc, child.id(), f);
            }
        }
    }
}

/// Port of Go's `charCounter` — counts characters with whitespace normalization.
///
/// Rules:
/// - Leading whitespace: ignored (not counted until the first non-space character).
/// - Trailing whitespace: ignored (the pending space is never flushed at end).
/// - Consecutive spaces: counted as one.
struct CharCounter {
    total: usize,
    last_was_space: bool,
    seen_non_space: bool,
}

impl CharCounter {
    fn new() -> Self {
        CharCounter {
            total: 0,
            last_was_space: false,
            seen_non_space: false,
        }
    }

    /// Port of charCounter.Count(r rune)
    fn count(&mut self, r: char) {
        if r.is_whitespace() {
            self.last_was_space = true;
            return;
        }
        if self.last_was_space && self.seen_non_space {
            // Space between words counts as 2 (the space + the char itself)
            self.total += 2;
        } else {
            self.total += 1;
        }
        self.last_was_space = false;
        self.seen_non_space = true;
    }

    fn total(&self) -> usize {
        self.total
    }

    /// Port of charCounter.ResetContext — reset word-boundary tracking without
    /// clearing the running total. Used in `clean_conditionally` to restart
    /// per-element context while keeping the accumulated global count.
    #[allow(dead_code)] // used in clean_conditionally (Phase 7)
    pub fn reset_context(&mut self) {
        self.last_was_space = false;
        self.seen_non_space = false;
    }
}

/// Port of commaCounter — true if the rune is a comma variant.
///
/// Covers commas as used in Latin, Sindhi, Chinese and various other scripts.
/// See: https://en.wikipedia.org/wiki/Comma#Comma_variants
fn is_comma(r: char) -> bool {
    matches!(
        r,
        '\u{002C}' // COMMA (,)
        | '\u{060C}' // ARABIC COMMA
        | '\u{FE50}' // SMALL COMMA
        | '\u{FE10}' // PRESENTATION FORM FOR VERTICAL COMMA
        | '\u{FE11}' // PRESENTATION FORM FOR VERTICAL IDEOGRAPHIC COMMA
        | '\u{2E41}' // REVERSED COMMA
        | '\u{2E34}' // RAISED COMMA
        | '\u{2E32}' // TURNED COMMA
        | '\u{FF0C}' // FULLWIDTH COMMA
    )
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::Document;

    fn doc(html: &str) -> Document {
        Document::parse(html)
    }

    #[test]
    fn has_text_content_finds_text() {
        let d = doc("<div><p>hello</p></div>");
        let body = d.body().unwrap();
        assert!(has_text_content(&d, body));
    }

    #[test]
    fn has_text_content_empty_is_false() {
        let d = doc("<div></div>");
        let body = d.body().unwrap();
        let div = d.first_element_child(body).unwrap();
        assert!(!has_text_content(&d, div));
    }

    #[test]
    fn has_text_content_whitespace_only_is_false() {
        let d = doc("<div>   </div>");
        let body = d.body().unwrap();
        let div = d.first_element_child(body).unwrap();
        assert!(!has_text_content(&d, div));
    }

    #[test]
    fn count_chars_strips_leading_trailing_whitespace() {
        // "  hello  " — leading/trailing spaces ignored, "hello" = 5 chars
        let d = doc("<p>  hello  </p>");
        let p = d
            .query_selector(d.document_element().unwrap(), "p")
            .unwrap();
        let (chars, _) = count_chars_and_commas(&d, p);
        assert_eq!(chars, 5, "leading/trailing whitespace should not count");
    }

    #[test]
    fn count_chars_collapses_internal_whitespace() {
        // "a   b" — 'a', space, 'b' = 3 chars (space counts as 1)
        let d = doc("<p>a   b</p>");
        let p = d
            .query_selector(d.document_element().unwrap(), "p")
            .unwrap();
        let (chars, _) = count_chars_and_commas(&d, p);
        assert_eq!(chars, 3, "internal whitespace run should count as 1 space");
    }

    #[test]
    fn count_commas_finds_unicode_variants() {
        // ASCII comma + ARABIC COMMA + FULLWIDTH COMMA
        let d = doc("<p>a,b\u{060C}c\u{FF0C}d</p>");
        let p = d
            .query_selector(d.document_element().unwrap(), "p")
            .unwrap();
        let (_, commas) = count_chars_and_commas(&d, p);
        assert_eq!(commas, 3);
    }

    #[test]
    fn count_chars_versus_simple_char_count() {
        // Whitespace-heavy string: "  a  b  "
        // utils::char_count returns 8 (all code points)
        // count_chars_and_commas returns 3 (a, space, b — leading/trailing ignored)
        let d = doc("<p>  a  b  </p>");
        let p = d
            .query_selector(d.document_element().unwrap(), "p")
            .unwrap();
        let text = d.text_content(p);
        let simple = crate::utils::char_count(&text);
        let (normalizing, _) = count_chars_and_commas(&d, p);
        assert!(
            simple > normalizing,
            "simple char_count ({simple}) should exceed normalizing count ({normalizing})"
        );
    }

    #[test]
    fn count_chars_across_nested_elements() {
        // <div><p>hello</p> <span>world</span></div>
        // "hello world" → h,e,l,l,o, ,w,o,r,l,d = 11
        let d = doc("<div><p>hello</p> <span>world</span></div>");
        let div = d
            .query_selector(d.document_element().unwrap(), "div")
            .unwrap();
        let (chars, _) = count_chars_and_commas(&d, div);
        assert_eq!(chars, 11);
    }
}
