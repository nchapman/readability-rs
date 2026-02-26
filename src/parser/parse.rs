// Public API methods: parse, check_html, check_document, parse_document, parse_and_mutate

use ego_tree::NodeId;
use url::Url;

use super::{Flags, Parser, Result};
use crate::dom::Document;
use crate::error::Error;
use crate::regexp::*;
use crate::render::inner_text;
use crate::utils::char_count;

impl Parser {
    // ── Public API (parser-parse.go) ──────────────────────────────────────

    /// Port of `Parse` — parse an HTML string and return the article.
    pub fn parse(&mut self, html: &str, page_url: Option<&Url>) -> Result {
        let doc = Document::parse(html);
        self.parse_and_mutate(doc, page_url)
    }

    /// Convenience wrapper: parse `html` and check readability without a pre-parsed Document.
    ///
    /// Equivalent to `CheckDocument(html.Parse(html))` in Go tests.
    pub fn check_html(&self, html: &str) -> bool {
        let doc = Document::parse(html);
        self.check_document(&doc)
    }

    /// Port of `CheckDocument` — returns true if the document is likely a readable article.
    ///
    /// Checks without running the full extraction pipeline.
    pub(crate) fn check_document(&self, doc: &Document) -> bool {
        // Get <p>, <pre>, and <article> nodes.
        let root = doc.root();
        let mut nodes = doc.query_selector_all(root, "p, pre, article");

        // Also collect unique parent <div> elements that contain <br> children.
        // These match the `div > br` pattern and indicate text-heavy divs.
        let br_parents = doc.query_selector_all(root, "div > br");
        let mut seen = std::collections::HashSet::new();
        for br in br_parents {
            if let Some(parent) = doc.parent(br) {
                if seen.insert(parent) {
                    nodes.push(parent);
                }
            }
        }

        // Walk nodes and accumulate a score. Return true when score exceeds 20.
        // This mirrors Go's `someNode` — it short-circuits on first qualifying node.
        let mut score = 0.0f64;
        for node in nodes {
            // Skip hidden nodes.
            if !Self::is_probably_visible_in(doc, node) {
                continue;
            }

            // Skip unlikely candidates that aren't maybe-candidates.
            let class = doc.attr(node, "class").unwrap_or("");
            let id = doc.attr(node, "id").unwrap_or("");
            let match_string = format!("{class} {id}");
            if crate::regexp::is_unlikely_candidate(&match_string)
                && !crate::regexp::maybe_its_a_candidate(&match_string)
            {
                continue;
            }

            // Skip <p> nodes that are inside <li> elements.
            if doc.tag_name(node) == "p" && Self::has_ancestor_tag_in(doc, node, "li") {
                continue;
            }

            let node_text = doc.text_content(node);
            let node_text = node_text.trim();
            // Go uses len() (UTF-8 byte count), not Unicode codepoint count.
            let len = node_text.len();
            if len < 140 {
                continue;
            }

            score += ((len - 140) as f64).sqrt();
            if score > 20.0 {
                return true;
            }
        }
        false
    }

    /// Static version of `is_probably_visible` that operates on an external Document.
    fn is_probably_visible_in(doc: &Document, id: NodeId) -> bool {
        let style = doc.attr(id, "style").unwrap_or("");
        let aria_hidden = doc.attr(id, "aria-hidden").unwrap_or("");
        let class = doc.attr(id, "class").unwrap_or("");

        (style.is_empty() || !RX_DISPLAY_NONE.is_match(style))
            && (style.is_empty() || !RX_VISIBILITY_HIDDEN.is_match(style))
            && !doc.has_attribute(id, "hidden")
            && (aria_hidden.is_empty() || aria_hidden != "true" || class.contains("fallback-image"))
    }

    /// Static version of `has_ancestor_tag` for `check_document`.
    fn has_ancestor_tag_in(doc: &Document, id: NodeId, tag: &str) -> bool {
        let mut cur = id;
        while let Some(parent) = doc.parent(cur) {
            if doc.tag_name(parent) == tag {
                return true;
            }
            cur = parent;
        }
        false
    }

    /// Port of `ParseDocument` — parse a document (clones it to leave original untouched).
    #[allow(dead_code)] // mirrors Go API; kept for parity
    pub(crate) fn parse_document(&mut self, doc: &Document, page_url: Option<&Url>) -> Result {
        self.parse_and_mutate(doc.clone(), page_url)
    }

    /// Port of `ParseAndMutate` — main entry point; mutates `doc` in place during parsing.
    pub(crate) fn parse_and_mutate(&mut self, doc: Document, page_url: Option<&Url>) -> Result {
        // Clamp n_top_candidates to at least 1 to avoid meaningless results.
        if self.n_top_candidates == 0 {
            self.n_top_candidates = 1;
        }

        // Reset per-parse state.
        self.doc = doc;
        self.document_uri = page_url.cloned();
        self.article_title = String::new();
        self.article_byline = String::new();
        self.article_dir = String::new();
        self.article_lang = String::new();
        self.flags = Flags::default();
        self.score_map.clear();
        self.data_tables.clear();
        self.attempts.clear();

        // Enforce element limit.
        if self.max_elems_to_parse > 0 {
            let root = self.doc.root();
            let n = self.doc.get_elements_by_tag_name(root, "*").len();
            if n > self.max_elems_to_parse {
                return Err(Error::Parse(format!("document too large: {n} elements")));
            }
        }

        // Unwrap noscript images before removing scripts.
        self.unwrap_noscript_images();

        // Extract JSON-LD metadata before removing scripts.
        let json_ld = if !self.disable_jsonld {
            self.get_jsonld()
        } else {
            super::metadata::JsonLdMetadata::default()
        };

        // Remove script/noscript tags.
        self.remove_scripts();

        // Prepare the HTML document (remove comments, style tags, replace <br>, <font>).
        self.prep_document();

        // Extract metadata.
        let metadata = self.get_article_metadata(&json_ld);
        self.article_title = metadata.title.clone();
        self.article_byline = metadata.byline.clone();

        // Grab article content.
        let article_content = self.grab_article();

        let (content, text_content, length, dir) = if let Some(content_id) = article_content {
            self.post_process_content(content_id);

            // The content node is a <div> wrapper; take its first element child.
            let readable = self
                .doc
                .first_element_child(content_id)
                .unwrap_or(content_id);
            let content_html = self.doc.outer_html(readable);
            let text = inner_text(&self.doc, readable);
            let len = char_count(&text);

            // Read direction from the content node.
            let dir = self
                .doc
                .attr(readable, "dir")
                .or_else(|| self.doc.attr(content_id, "dir"))
                .unwrap_or("")
                .to_string();

            (content_html, text, len, dir)
        } else {
            (String::new(), String::new(), 0, String::new())
        };

        // Excerpt fallback: if metadata has no excerpt, use InnerText of the first <p>
        // in the article content. Port of Go's `article.Excerpt()` lazy fallback.
        let excerpt_meta = metadata.excerpt.clone();
        let excerpt = if excerpt_meta.is_empty() {
            // Find the first <p> inside the readable node.
            let readable_for_excerpt =
                article_content.and_then(|cid| self.doc.first_element_child(cid));
            let first_p = readable_for_excerpt.and_then(|r| self.get_element_by_tag_name(r, "p"));
            if let Some(p) = first_p {
                let p_text = inner_text(&self.doc, p);
                let normalized: String = p_text.split_whitespace().collect::<Vec<_>>().join(" ");
                normalized
            } else {
                String::new()
            }
        } else {
            // Mirror Go's `article.Excerpt()`: always normalize whitespace
            // with strings.Fields semantics regardless of source.
            excerpt_meta
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };

        Ok(crate::article::Article {
            title: self.article_title.clone(),
            byline: self.article_byline.clone(),
            excerpt,
            site_name: metadata.site_name,
            image: metadata.image,
            favicon: metadata.favicon,
            language: self.article_lang.clone(),
            published_time: metadata.published_time,
            modified_time: metadata.modified_time,
            content,
            text_content,
            length,
            dir,
        })
    }
}
