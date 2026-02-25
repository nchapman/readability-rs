// Port of go-readability/render/inner_text.go

use ego_tree::NodeId;
use scraper::Node;

use crate::dom::Document;

const NO_BREAK_SPACE: char = '\u{00A0}';

/// Port of InnerText — extract plain text respecting visual layout.
///
/// Differences from `text_content`:
/// - Block elements (`p`, `div`, `h1`–`h6`, `table`, `ul`, `ol`, etc.) add newlines
/// - Table cells (`td`, `th`) add tabs
/// - `aria-hidden="true"` elements (and certain non-text elements) are skipped entirely
/// - MathJax/LaTeX: output LaTeX source, not rendered text
/// - Consecutive whitespace collapsed to single space within inline content
pub fn inner_text(doc: &Document, id: NodeId) -> String {
    let mut tb = InnerTextBuilder::new();
    render(doc, id, false, &mut tb);
    tb.into_string()
}

// ── Recursive renderer ───────────────────────────────────────────────────────

fn render(doc: &Document, id: NodeId, keep_whitespace: bool, tb: &mut InnerTextBuilder) {
    let Some(node) = doc.html.tree.get(id) else {
        return;
    };

    match node.value() {
        Node::Text(text) => {
            render_text(&text.text, keep_whitespace, tb);
        }
        Node::Element(el) => {
            // Skip aria-hidden elements (port of isHiddenElement)
            if is_hidden_element(el) {
                return;
            }

            let tag = el.name();

            // Elements that never hold user-facing text — skip entirely.
            match tag {
                "head" | "meta" | "style" | "iframe" | "audio" | "video" | "track" | "source"
                | "canvas" | "svg" | "map" | "area" => return,

                "script" => {
                    // MathJax 2: <script type="math/tex; mode=display">…</script>
                    if let Some((is_ok, is_block)) = is_mathjax_script(el) {
                        if is_ok {
                            let content = script_text_content(doc, id);
                            render_tex(&content, is_block, tb);
                        }
                    }
                    return;
                }

                "math" => {
                    // MathML: look for <annotation encoding="application/x-tex">
                    let is_block = el.attr("display") == Some("block");
                    if let Some(annotation_id) =
                        find_annotation(doc, id, "application/x-tex")
                    {
                        let content = doc.text_content(annotation_id);
                        render_tex(&content, is_block, tb);
                    }
                    return;
                }

                "mjx-container" => {
                    // MathJax 3: look for data-latex attribute in descendants
                    let is_block = el.attr("display") == Some("true");
                    if let Some(latex) = find_latex(doc, id) {
                        render_tex(&latex, is_block, tb);
                    }
                    return;
                }

                _ => {}
            }

            // Emit whitespace/newlines before recursing into children.
            let mut keep_ws = keep_whitespace;
            match tag {
                "br" => {
                    tb.write_newline(1, false);
                }
                "hr" | "p" | "blockquote" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul"
                | "ol" | "dl" | "table" => {
                    tb.write_newline(2, true);
                }
                "pre" => {
                    tb.write_newline(2, true);
                    keep_ws = true;
                }
                "th" | "td" => {
                    tb.queue_space('\t');
                }
                "div" | "figure" | "figcaption" | "picture" | "li" | "dt" | "dd" | "header"
                | "footer" | "main" | "section" | "article" | "aside" | "nav" | "address"
                | "details" | "summary" | "dialog" | "form" | "fieldset" | "caption"
                | "thead" | "tbody" | "tfoot" | "tr" => {
                    tb.write_newline(1, true);
                }
                _ => {}
            }

            // Recurse into children.
            for child in node.children() {
                render(doc, child.id(), keep_ws, tb);
            }
        }
        _ => {
            // Ignore document root, comments, processing instructions, etc.
        }
    }
}

/// Render a text node's content into the builder.
fn render_text(data: &str, keep_whitespace: bool, tb: &mut InnerTextBuilder) {
    if keep_whitespace {
        tb.write_pre(data);
        return;
    }

    // Split text into words separated by whitespace, queuing spaces between them.
    let mut start_of_word: Option<usize> = None;
    for (i, r) in data.char_indices() {
        if r.is_whitespace() {
            if let Some(start) = start_of_word {
                tb.write_word(&data[start..i]);
                start_of_word = None;
            }
            if r == NO_BREAK_SPACE {
                tb.queue_space(NO_BREAK_SPACE);
            } else {
                tb.queue_space(' ');
            }
        } else if start_of_word.is_none() {
            start_of_word = Some(i);
        }
    }
    if let Some(start) = start_of_word {
        tb.write_word(&data[start..]);
    }
}

/// Output a LaTeX expression. Block mode wraps in `$$…$$`, inline in `$…$`.
fn render_tex(expr: &str, is_block: bool, tb: &mut InnerTextBuilder) {
    if is_block {
        tb.write_newline(2, true);
        tb.write_pre("$$\n");
        tb.write_pre(expr.trim());
        tb.write_pre("\n$$");
        tb.write_newline(2, true);
    } else {
        tb.write_pre("$");
        tb.write_pre(expr.trim());
        tb.write_pre("$");
    }
}

// ── Helper: MathJax 2 script detection ──────────────────────────────────────

/// Port of isMathjaxScript — detect `<script type="math/tex; mode=display">`.
///
/// Returns `Some((true, is_block))` if it's a MathJax script, `None` otherwise.
fn is_mathjax_script(el: &scraper::node::Element) -> Option<(bool, bool)> {
    let type_attr = el.attr("type")?;
    let (mime, rest) = match type_attr.find(';') {
        Some(idx) => (&type_attr[..idx], &type_attr[idx + 1..]),
        None => (type_attr, ""),
    };
    if mime != "math/tex" {
        return None;
    }
    let is_block = rest.contains("mode=display");
    Some((true, is_block))
}

/// Get the direct text content of a `<script>` element (concatenation of text children).
fn script_text_content(doc: &Document, id: NodeId) -> String {
    doc.child_nodes(id)
        .into_iter()
        .filter_map(|cid| {
            let n = doc.html.tree.get(cid)?;
            if let Node::Text(text) = n.value() {
                Some(text.text.as_ref().to_string())
            } else {
                None
            }
        })
        .collect()
}

// ── Helper: MathML annotation search ────────────────────────────────────────

/// Port of findAnnotation — DFS search for `<annotation encoding="…">`.
fn find_annotation(doc: &Document, id: NodeId, mime_type: &str) -> Option<NodeId> {
    let node = doc.html.tree.get(id)?;
    for child in node.children() {
        if let Node::Element(el) = child.value() {
            if el.name() == "annotation" && el.attr("encoding") == Some(mime_type) {
                return Some(child.id());
            }
            if let Some(found) = find_annotation(doc, child.id(), mime_type) {
                return Some(found);
            }
        }
    }
    None
}

/// Port of findLatex — DFS search for `data-latex` attribute in element descendants.
fn find_latex(doc: &Document, id: NodeId) -> Option<String> {
    let node = doc.html.tree.get(id)?;
    for child in node.children() {
        if let Node::Element(el) = child.value() {
            if let Some(val) = el.attr("data-latex") {
                return Some(val.to_string());
            }
            if let Some(found) = find_latex(doc, child.id()) {
                return Some(found);
            }
        }
    }
    None
}

/// Port of isHiddenElement — true if `aria-hidden` is `""` or `"true"`.
fn is_hidden_element(el: &scraper::node::Element) -> bool {
    matches!(el.attr("aria-hidden"), Some("true") | Some(""))
}

// ── InnerTextBuilder ─────────────────────────────────────────────────────────

/// Port of innerTextBuilder — accumulates text with space/newline collapsing.
struct InnerTextBuilder {
    buf: String,
    /// Pending space character (will be emitted before the next word unless
    /// a newline is about to follow).
    queued_space: Option<char>,
    /// Number of trailing newlines already written.
    newline_count: u8,
}

impl InnerTextBuilder {
    fn new() -> Self {
        InnerTextBuilder {
            buf: String::new(),
            queued_space: None,
            newline_count: 0,
        }
    }

    fn into_string(self) -> String {
        self.buf
    }

    /// Queue a pending space (tab or regular space). Only one pending space is
    /// kept at a time; if one is already queued, the new one is ignored.
    fn queue_space(&mut self, c: char) {
        if self.queued_space.is_none() {
            self.queued_space = Some(c);
        }
    }

    /// Port of WriteNewline — write `n` newlines.
    ///
    /// If `collapse` is true, only write the difference from the current trailing
    /// newline count (and skip entirely if we're already at or above `n`).
    fn write_newline(&mut self, n: u8, collapse: bool) {
        let to_write = if collapse {
            if self.newline_count >= n {
                return;
            }
            n - self.newline_count
        } else {
            n
        };

        self.newline_count += to_write;

        // Don't write newlines at the very start of output.
        if collapse && self.buf.is_empty() {
            return;
        }
        for _ in 0..to_write {
            self.buf.push('\n');
        }
    }

    /// Port of WriteWord — write a word, flushing any pending space first.
    fn write_word(&mut self, w: &str) {
        if let Some(sp) = self.queued_space.take() {
            if self.newline_count == 0 {
                self.buf.push(sp);
            }
        }
        self.buf.push_str(w);
        self.newline_count = 0;
        // queued_space already taken above
    }

    /// Port of WritePre — write pre-formatted text (verbatim), flushing pending space.
    fn write_pre(&mut self, s: &str) {
        if let Some(sp) = self.queued_space.take() {
            if self.newline_count == 0 {
                self.buf.push(sp);
            }
        }
        self.buf.push_str(s);
        self.newline_count = 0;
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Tests (port of render/inner_text_test.go)
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::Document;

    fn render_body(html: &str) -> String {
        let d = Document::parse(html);
        let body = d.body().unwrap();
        inner_text(&d, body)
    }

    #[test]
    fn plain_text_is_included() {
        let s = render_body("<p>hello world</p>");
        assert!(s.contains("hello world"), "got: {s:?}");
    }

    #[test]
    fn block_elements_produce_newlines() {
        let s = render_body("<p>first</p><p>second</p>");
        // Two paragraphs should be separated by at least one newline
        assert!(s.contains('\n'), "got: {s:?}");
        assert!(s.contains("first"), "got: {s:?}");
        assert!(s.contains("second"), "got: {s:?}");
    }

    #[test]
    fn display_none_content_absent() {
        // aria-hidden="true" is how our inner_text hides elements
        let s = render_body(r#"<p aria-hidden="true">secret</p><p>visible</p>"#);
        assert!(!s.contains("secret"), "hidden content should be absent: {s:?}");
        assert!(s.contains("visible"), "got: {s:?}");
    }

    #[test]
    fn td_th_get_tabs() {
        let s = render_body("<table><tr><td>A</td><td>B</td></tr></table>");
        // Should have a tab between A and B
        assert!(s.contains('\t'), "td should produce tab: {s:?}");
        assert!(s.contains('A') && s.contains('B'), "got: {s:?}");
    }

    #[test]
    fn nested_blocks_dont_duplicate_newlines() {
        let s = render_body("<div><p>hello</p></div><p>world</p>");
        // No more than 2 consecutive newlines
        assert!(!s.contains("\n\n\n"), "got: {s:?}");
    }

    #[test]
    fn mathjax2_script_outputs_latex() {
        // Place script inside body explicitly so html5ever doesn't put it in <head>.
        let s = render_body(r#"<p><script type="math/tex">x^2</script></p>"#);
        assert!(s.contains("x^2"), "got: {s:?}");
    }

    #[test]
    fn mathjax2_block_script_uses_double_dollar() {
        let s = render_body(r#"<p><script type="math/tex; mode=display">E=mc^2</script></p>"#);
        assert!(s.contains("$$") && s.contains("E=mc^2"), "got: {s:?}");
    }

    #[test]
    fn script_elements_without_mathjax_are_empty() {
        // Inline script in body context
        let s = render_body(r#"<p><script>alert("xss")</script></p><p>text</p>"#);
        assert!(!s.contains("alert"), "script content should be skipped: {s:?}");
        assert!(s.contains("text"), "got: {s:?}");
    }

    #[test]
    fn inline_elements_included() {
        let s = render_body("<p>hello <em>world</em></p>");
        assert!(s.contains("hello") && s.contains("world"), "got: {s:?}");
    }

    #[test]
    fn pre_preserves_whitespace() {
        let s = render_body("<pre>  indented\n  code</pre>");
        assert!(s.contains("  indented"), "pre should preserve indentation: {s:?}");
    }

    #[test]
    fn br_produces_single_newline() {
        let s = render_body("<p>first<br>second</p>");
        assert!(s.contains('\n'), "br should produce newline: {s:?}");
        assert!(s.contains("first") && s.contains("second"), "got: {s:?}");
    }
}
