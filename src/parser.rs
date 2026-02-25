// Port of go-readability/parser.go + parser-parse.go

use std::collections::{HashMap, HashSet};

use ego_tree::NodeId;
use scraper::Node;
use url::Url;

use crate::article::Article;
use crate::dom::Document;
use crate::error::Error;
use crate::regexp::*;
use crate::render::inner_text;
use crate::traverse::has_text_content;
use crate::utils::{char_count, is_valid_url, str_or, to_absolute_uri, word_count};

pub type Result<T = Article> = std::result::Result<T, Error>;

// ── Constants ─────────────────────────────────────────────────────────────────

#[allow(dead_code)] // used in Phase 6 (grabArticle / replaceBrs)
const DIV_TO_P_ELEMS: &[&str] = &[
    "blockquote", "dl", "div", "img", "ol", "p", "pre", "table", "ul", "select",
];

#[allow(dead_code)] // used in Phase 6 (grabArticle)
const ALTER_TO_DIV_EXCEPTIONS: &[&str] = &["div", "article", "section", "p", "ol", "ul"];

const PHRASING_ELEMS: &[&str] = &[
    "abbr", "audio", "b", "bdo", "br", "button", "cite", "code", "data", "datalist", "dfn",
    "em", "embed", "i", "img", "input", "kbd", "label", "mark", "math", "meter", "noscript",
    "object", "output", "progress", "q", "ruby", "samp", "script", "select", "small", "span",
    "strong", "sub", "sup", "textarea", "time", "var", "wbr",
];

#[allow(dead_code)] // used in Phase 6 (grabArticle)
const UNLIKELY_ROLES: &[&str] = &[
    "menu", "menubar", "complementary", "navigation", "alert", "alertdialog", "dialog",
];

#[allow(dead_code)] // used in Phase 6 (cleanStyles via prepArticle)
const PRESENTATIONAL_ATTRS: &[&str] = &[
    "align", "background", "bgcolor", "border", "cellpadding", "cellspacing", "frame",
    "hspace", "rules", "style", "valign", "vspace",
];

#[allow(dead_code)] // used in Phase 6 (cleanStyles via prepArticle)
const DEPRECATED_SIZE_ATTR_ELEMS: &[&str] = &["table", "th", "td", "hr", "pre"];

// ── Internal structs ──────────────────────────────────────────────────────────

/// Port of `flags` — controls which phases of the algorithm are active.
#[derive(Clone, Debug)]
struct Flags {
    #[allow(dead_code)] // used in Phase 6 (grabArticle)
    strip_unlikelys: bool,
    #[allow(dead_code)] // used in Phase 6 (getClassWeight)
    use_weight_classes: bool,
    #[allow(dead_code)] // used in Phase 6 (cleanConditionally)
    clean_conditionally: bool,
}

impl Default for Flags {
    fn default() -> Self {
        Flags { strip_unlikelys: true, use_weight_classes: true, clean_conditionally: true }
    }
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Port of `Parser` — the core readability extraction engine.
///
/// Create with `Parser::new()`, then call `parse()` / `parse_document()`.
pub struct Parser {
    // ── Public configuration ──────────────────────────────────────────────
    /// Max DOM nodes to process. 0 = unlimited. Port of `MaxElemsToParse`.
    pub max_elems_to_parse: usize,
    /// Number of top candidates to compare during scoring. Port of `NTopCandidates`.
    pub n_top_candidates: usize,
    /// Minimum character count for accepted article content. Port of `CharThresholds`.
    pub char_thresholds: usize,
    /// CSS class names to preserve when `keep_classes` is false. Port of `ClassesToPreserve`.
    pub classes_to_preserve: Vec<String>,
    /// If true, keep all class attributes. Port of `KeepClasses`.
    pub keep_classes: bool,
    /// Tag names eligible for content scoring. Port of `TagsToScore`.
    pub tags_to_score: Vec<String>,
    /// Disable JSON-LD metadata extraction. Port of `DisableJSONLD`.
    pub disable_jsonld: bool,
    /// Optional regex for video URLs to allow. Port of `AllowedVideoRegex`.
    pub allowed_video_regex: Option<regex::Regex>,

    // ── Per-parse state (reset at the start of each parse_and_mutate call) ──
    doc: Document,
    document_uri: Option<Url>,
    article_title: String,
    article_byline: String,
    article_dir: String,
    article_lang: String,
    flags: Flags,
}

impl Parser {
    /// Port of `NewParser` — construct a parser with default settings.
    pub fn new() -> Self {
        Parser {
            max_elems_to_parse: 0,
            n_top_candidates: 5,
            char_thresholds: 500,
            classes_to_preserve: vec!["page".to_string()],
            keep_classes: false,
            tags_to_score: vec![
                "section".to_string(), "h2".to_string(), "h3".to_string(),
                "h4".to_string(), "h5".to_string(), "h6".to_string(),
                "p".to_string(), "td".to_string(), "pre".to_string(),
            ],
            disable_jsonld: false,
            allowed_video_regex: None,
            doc: Document::parse(""),
            document_uri: None,
            article_title: String::new(),
            article_byline: String::new(),
            article_dir: String::new(),
            article_lang: String::new(),
            flags: Flags::default(),
        }
    }

    // ── Public API (parser-parse.go) ──────────────────────────────────────

    /// Port of `Parse` — parse an HTML string and return the article.
    pub fn parse(&mut self, html: &str, page_url: Option<&Url>) -> Result {
        let doc = Document::parse(html);
        self.parse_and_mutate(doc, page_url)
    }

    /// Port of `ParseDocument` — parse a document (clones it to leave original untouched).
    pub fn parse_document(&mut self, doc: &Document, page_url: Option<&Url>) -> Result {
        self.parse_and_mutate(doc.clone(), page_url)
    }

    /// Port of `ParseAndMutate` — main entry point; mutates `doc` in place during parsing.
    pub fn parse_and_mutate(&mut self, doc: Document, page_url: Option<&Url>) -> Result {
        // Reset per-parse state.
        self.doc = doc;
        self.document_uri = page_url.cloned();
        self.article_title = String::new();
        self.article_byline = String::new();
        self.article_dir = String::new();
        self.article_lang = String::new();
        self.flags = Flags::default();

        // Enforce element limit.
        if self.max_elems_to_parse > 0 {
            let root = self.doc.root();
            let n = self.doc.get_elements_by_tag_name(root, "*").len();
            if n > self.max_elems_to_parse {
                return Err(Error::Parse(format!(
                    "document too large: {n} elements"
                )));
            }
        }

        // Unwrap noscript images before removing scripts.
        self.unwrap_noscript_images();

        // Extract JSON-LD metadata before removing scripts.
        let json_ld = if !self.disable_jsonld {
            self.get_jsonld()
        } else {
            HashMap::new()
        };

        // Remove script/noscript tags.
        self.remove_scripts();

        // Prepare the HTML document (remove comments, style tags, replace <br>, <font>).
        self.prep_document();

        // Extract metadata.
        let metadata = self.get_article_metadata(&json_ld);
        self.article_title = metadata.get("title").cloned().unwrap_or_default();
        self.article_byline = metadata.get("byline").cloned().unwrap_or_default();

        // Grab article content (Phase 6 — returns None until implemented).
        let article_content = self.grab_article();

        let (content, text_content, length, dir) = if let Some(content_id) = article_content {
            self.post_process_content(content_id);

            // The content node is a <div> wrapper; take its first element child.
            let readable = self.doc.first_element_child(content_id).unwrap_or(content_id);
            let content_html = self.doc.outer_html(readable);
            let text = inner_text(&self.doc, readable);
            let len = char_count(&text);

            // Read direction from the content node.
            let dir = self.doc.attr(readable, "dir")
                .or_else(|| self.doc.attr(content_id, "dir"))
                .unwrap_or("")
                .to_string();

            (content_html, text, len, dir)
        } else {
            (String::new(), String::new(), 0, String::new())
        };

        // Excerpt fallback: if metadata has no excerpt, use the article text.
        let excerpt = metadata.get("excerpt").cloned().unwrap_or_default();

        Ok(Article {
            title: self.article_title.clone(),
            byline: self.article_byline.clone(),
            excerpt,
            site_name: metadata.get("siteName").cloned().unwrap_or_default(),
            image: metadata.get("image").cloned().unwrap_or_default(),
            favicon: metadata.get("favicon").cloned().unwrap_or_default(),
            language: self.article_lang.clone(),
            published_time: metadata.get("publishedTime").cloned().unwrap_or_default(),
            modified_time: metadata.get("modifiedTime").cloned().unwrap_or_default(),
            content,
            text_content,
            length,
            dir,
        })
    }

    // ── Node iteration helpers ────────────────────────────────────────────

    /// Port of `removeNodes` — remove nodes (optionally filtered) from the tree.
    ///
    /// Iterates backwards to allow safe removal during iteration.
    /// If `filter` is `None`, removes all nodes; if `Some(f)`, removes those where `f` returns true.
    fn remove_nodes<F>(&mut self, nodes: Vec<NodeId>, filter: Option<F>)
    where
        F: Fn(&Document, NodeId) -> bool,
    {
        for id in nodes.into_iter().rev() {
            if self.doc.parent(id).is_none() {
                continue;
            }
            let should_remove = match &filter {
                None => true,
                Some(f) => f(&self.doc, id),
            };
            if should_remove {
                self.doc.remove(id);
            }
        }
    }

    /// Port of `replaceNodeTags` — rename all nodes in the list to `new_tag`.
    fn replace_node_tags(&mut self, nodes: Vec<NodeId>, new_tag: &str) {
        for id in nodes.into_iter().rev() {
            self.doc.rename_tag(id, new_tag);
        }
    }

    // ── DOM traversal helpers ─────────────────────────────────────────────

    /// Port of `getElementByTagName` — first descendant element with the given tag (DFS).
    fn get_element_by_tag_name(&self, id: NodeId, tag: &str) -> Option<NodeId> {
        // get_elements_by_tag_name returns all matches; take the first.
        self.doc.get_elements_by_tag_name(id, tag).into_iter().next()
    }

    /// Port of `getInnerText` — text content, optionally whitespace-normalized.
    fn get_inner_text(&self, id: NodeId, normalize: bool) -> String {
        let text = self.doc.text_content(id);
        if normalize {
            normalize_spaces(text.trim())
        } else {
            text.trim().to_string()
        }
    }

    /// Port of `isWhitespace` — true if the node is purely whitespace.
    fn is_whitespace(&self, id: NodeId) -> bool {
        match self.doc.html.tree.get(id).map(|n| n.value()) {
            Some(Node::Text(text)) => !has_text_content(&self.doc, id) && text.text.trim().is_empty(),
            Some(Node::Element(_)) => self.doc.tag_name(id) == "br",
            _ => false,
        }
    }

    /// Port of `isPhrasingContent` — true if the node qualifies as phrasing content.
    fn is_phrasing_content(&self, id: NodeId) -> bool {
        let tag = self.doc.tag_name(id);
        if self.doc.is_text_node(id) {
            return true;
        }
        if PHRASING_ELEMS.contains(&tag) {
            return true;
        }
        if (tag == "a" || tag == "del" || tag == "ins")
            && self.doc.child_nodes(id).iter().all(|&c| self.is_phrasing_content(c))
        {
            return true;
        }
        false
    }

    #[allow(dead_code)] // used in Phase 6 (replaceBrs inner traversal)
    /// Port of `nextNode` — advance past whitespace-only nodes.
    ///
    /// Starting at `id`, returns the first sibling (or `id` itself) that is either
    /// an element node or a non-whitespace text node.
    fn next_node(&self, id: NodeId) -> Option<NodeId> {
        let mut cur = Some(id);
        while let Some(n) = cur {
            let is_element = self.doc.is_element(n);
            let has_text = has_text_content(&self.doc, n);
            if is_element || has_text {
                return Some(n);
            }
            cur = self.doc.next_sibling(n);
        }
        None
    }

    /// Port of `getNextNode` — depth-first traversal step.
    ///
    /// If `ignore_self_and_kids` is true, skip this node's children (used when removing).
    fn get_next_node(&self, id: NodeId, ignore_self_and_kids: bool) -> Option<NodeId> {
        // Descend into first child unless we're skipping.
        if !ignore_self_and_kids {
            if let Some(child) = self.doc.first_element_child(id) {
                return Some(child);
            }
        }
        // Try next sibling.
        if let Some(sibling) = self.doc.next_element_sibling(id) {
            return Some(sibling);
        }
        // Walk up until we find a parent with a next sibling.
        let mut cur = id;
        loop {
            match self.doc.parent(cur) {
                None => return None,
                Some(p) => {
                    if let Some(sibling) = self.doc.next_element_sibling(p) {
                        return Some(sibling);
                    }
                    cur = p;
                }
            }
        }
    }

    /// Port of `removeAndGetNext` — remove a node and return its traversal successor.
    fn remove_and_get_next(&mut self, id: NodeId) -> Option<NodeId> {
        let next = self.get_next_node(id, true);
        self.doc.remove(id);
        next
    }

    /// Port of `isElementWithoutContent` — true if node is an element with no meaningful content.
    fn is_element_without_content(&self, id: NodeId) -> bool {
        if !self.doc.is_element(id) {
            return false;
        }
        for child_id in self.doc.child_nodes(id) {
            match self.doc.html.tree.get(child_id).map(|n| n.value()) {
                Some(Node::Text(t)) => {
                    if crate::utils::has_content(&t.text) {
                        return false;
                    }
                }
                Some(Node::Element(_)) => {
                    let tag = self.doc.tag_name(child_id);
                    if tag != "br" && tag != "hr" {
                        return false;
                    }
                }
                _ => {}
            }
        }
        true
    }

    /// Port of `hasSingleTagInsideElement` — true if element has exactly one element child
    /// with the given tag, and no non-whitespace text nodes.
    fn has_single_tag_inside_element(&self, id: NodeId, tag: &str) -> bool {
        let children = self.doc.children(id);
        if children.len() != 1 || self.doc.tag_name(children[0]) != tag {
            return false;
        }
        // Must have no non-whitespace text nodes among all child nodes.
        !self.doc.child_nodes(id).iter().any(|&c| {
            if let Some(Node::Text(t)) = self.doc.html.tree.get(c).map(|n| n.value()) {
                RX_HAS_CONTENT.is_match(&t.text)
            } else {
                false
            }
        })
    }

    #[allow(dead_code)] // used in Phase 6 (grabArticle)
    /// Port of `hasChildBlockElement` — true if any child is a block-level element.
    fn has_child_block_element(&self, id: NodeId) -> bool {
        self.doc.child_nodes(id).iter().any(|&c| {
            let tag = self.doc.tag_name(c);
            DIV_TO_P_ELEMS.contains(&tag) || self.has_child_block_element(c)
        })
    }

    #[allow(dead_code)] // used in Phase 6 (unwrapNoscriptImages delegation)
    /// Port of `isSingleImage` — true if the node is or contains exactly one image.
    fn is_single_image(&self, id: NodeId) -> bool {
        is_single_image_in(&self.doc, id)
    }

    // ── Scoring / classification helpers ────────────────────────────────

    #[allow(dead_code)] // used in Phase 6 (grabArticle)
    /// Port of `isProbablyVisible` — true if the node is not hidden.
    fn is_probably_visible(&self, id: NodeId) -> bool {
        let style = self.doc.attr(id, "style").unwrap_or("");
        let aria_hidden = self.doc.attr(id, "aria-hidden").unwrap_or("");
        let class = self.doc.attr(id, "class").unwrap_or("");

        (style.is_empty() || !RX_DISPLAY_NONE.is_match(style))
            && (style.is_empty() || !RX_VISIBILITY_HIDDEN.is_match(style))
            && !self.doc.has_attribute(id, "hidden")
            && (aria_hidden.is_empty()
                || aria_hidden != "true"
                || class.contains("fallback-image"))
    }

    #[allow(dead_code)] // used in Phase 6 (grabArticle)
    /// Port of `isValidByline` — true if the node looks like a byline.
    fn is_valid_byline(&self, id: NodeId, match_string: &str) -> bool {
        let rel = self.doc.attr(id, "rel").unwrap_or("");
        let itemprop = self.doc.attr(id, "itemprop").unwrap_or("");
        rel == "author" || itemprop.contains("author") || crate::regexp::is_byline(match_string)
    }

    #[allow(dead_code)] // used in Phase 6 (grabArticle)
    /// Port of `headerDuplicatesTitle` — true if the node is an h1/h2 whose text is
    /// very similar to the article title.
    fn header_duplicates_title(&self, id: NodeId) -> bool {
        let tag = self.doc.tag_name(id);
        if tag != "h1" && tag != "h2" {
            return false;
        }
        let heading = self.get_inner_text(id, false);
        self.text_similarity(&self.article_title.clone(), &heading) > 0.75
    }

    #[allow(dead_code)] // used in Phase 6 (grabArticle, cleanHeaders, cleanConditionally)
    /// Port of `getClassWeight` — score bonus/penalty from class/id names.
    ///
    /// Returns 0 when `use_weight_classes` is false.
    fn get_class_weight(&self, id: NodeId) -> i32 {
        if !self.flags.use_weight_classes {
            return 0;
        }
        let mut weight = 0i32;
        if let Some(cls) = self.doc.attr(id, "class") {
            if crate::regexp::is_negative_class(cls) {
                weight -= 25;
            }
            if crate::regexp::is_positive_class(cls) {
                weight += 25;
            }
        }
        if let Some(id_attr) = self.doc.attr(id, "id") {
            if crate::regexp::is_negative_class(id_attr) {
                weight -= 25;
            }
            if crate::regexp::is_positive_class(id_attr) {
                weight += 25;
            }
        }
        weight
    }

    #[allow(dead_code)] // used in Phase 6 (getLinkDensity)
    /// Port of `getLinkDensityCoefficient` — hash-only links are weighted lower.
    fn get_link_density_coefficient(doc: &Document, a: NodeId) -> f64 {
        let href = doc.attr(a, "href").unwrap_or("").trim().to_string();
        if href.len() > 1 && href.starts_with('#') { 0.3 } else { 1.0 }
    }

    #[allow(dead_code)] // used in Phase 6 (grabArticle, cleanConditionally)
    /// Port of `getLinkDensity` — ratio of link chars to total chars in the node.
    fn get_link_density(&self, id: NodeId) -> f64 {
        let mut total: usize = 0;
        let mut link_weighted: f64 = 0.0;

        fn walk(
            doc: &Document,
            n: NodeId,
            link_counter: &mut Option<(usize, f64)>, // (count, coefficient)
            total: &mut usize,
            link_weighted: &mut f64,
        ) {
            if let Some(Node::Text(text)) = doc.html.tree.get(n).map(|x| x.value()) {
                let count = crate::utils::char_count(&text.text);
                *total += count;
                if let Some((ref mut lc, _coeff)) = link_counter {
                    *lc += count;
                }
                return;
            }
            let tag = doc.tag_name(n);
            if tag == "a" {
                let coeff = Parser::get_link_density_coefficient(doc, n);
                let mut my_counter: Option<(usize, f64)> = Some((0, coeff));
                for child in doc.child_nodes(n) {
                    walk(doc, child, &mut my_counter, total, link_weighted);
                }
                if let Some((lc, c)) = my_counter {
                    *link_weighted += lc as f64 * c;
                }
            } else {
                for child in doc.child_nodes(n) {
                    walk(doc, child, link_counter, total, link_weighted);
                }
            }
        }

        walk(&self.doc, id, &mut None, &mut total, &mut link_weighted);

        if total == 0 { 0.0 } else { link_weighted / total as f64 }
    }

    #[allow(dead_code)] // used in Phase 6 (cleanConditionally, grabArticle)
    /// Port of `hasAncestorTag` — true if any ancestor (up to `max_depth`) has the given tag.
    ///
    /// `max_depth <= 0` means no limit.
    fn has_ancestor_tag<F>(&self, id: NodeId, tag: &str, max_depth: i32, filter: Option<F>) -> bool
    where
        F: Fn(&Document, NodeId) -> bool,
    {
        let mut depth = 0;
        let mut cur = id;
        while let Some(parent) = self.doc.parent(cur) {
            if max_depth > 0 && depth > max_depth {
                return false;
            }
            if self.doc.tag_name(parent) == tag
                && filter.as_ref().map(|f| f(&self.doc, parent)).unwrap_or(true)
            {
                return true;
            }
            cur = parent;
            depth += 1;
        }
        false
    }

    #[allow(dead_code)] // used in Phase 6 (grabArticle scoring)
    /// Port of `getNodeAncestors` — collect ancestors up to `max_depth` (0 = unlimited).
    fn get_node_ancestors(&self, id: NodeId, max_depth: usize) -> Vec<NodeId> {
        let mut result = Vec::new();
        let mut cur = id;
        while let Some(parent) = self.doc.parent(cur) {
            result.push(parent);
            if max_depth > 0 && result.len() == max_depth {
                break;
            }
            cur = parent;
        }
        result
    }

    /// Port of `textSimilarity` (Parser method — different algorithm from utils::text_similarity).
    ///
    /// Returns `1 - (unique_B_chars / total_B_chars)`, lowercased, tokenized by `\W+`.
    fn text_similarity(&self, a: &str, b: &str) -> f64 {
        let a_lower = a.to_lowercase();
        let b_lower = b.to_lowercase();

        let tokens_a: HashSet<&str> = RX_TOKENIZE
            .split(&a_lower)
            .filter(|s| !s.is_empty())
            .collect();

        let tokens_b: Vec<&str> = RX_TOKENIZE
            .split(&b_lower)
            .filter(|s| !s.is_empty())
            .collect();

        let unique_b: Vec<&str> = tokens_b.iter().filter(|t| !tokens_a.contains(**t)).copied().collect();

        let merged_b = tokens_b.join(" ");
        let merged_unique_b = unique_b.join(" ");

        let total = char_count(&merged_b);
        if total == 0 {
            return 0.0;
        }
        1.0 - char_count(&merged_unique_b) as f64 / total as f64
    }

    // ── Document preparation ──────────────────────────────────────────────

    /// Port of `removeComments` — remove all HTML comment nodes.
    fn remove_comments(&mut self) {
        let root = self.doc.root();
        self.remove_comments_from(root);
    }

    fn remove_comments_from(&mut self, id: NodeId) {
        let children: Vec<NodeId> = self.doc.child_nodes(id);
        for child in children {
            if matches!(self.doc.html.tree.get(child).map(|n| n.value()), Some(Node::Comment(_))) {
                self.doc.remove(child);
            } else {
                self.remove_comments_from(child);
            }
        }
    }

    /// Port of `removeScripts` — remove all `<script>` and `<noscript>` elements.
    fn remove_scripts(&mut self) {
        let root = self.doc.root();
        let targets = self.doc.get_all_nodes_with_tag(root, &["script", "noscript"]);
        self.remove_nodes(targets, None::<fn(&Document, NodeId) -> bool>);
    }

    /// Port of `setNodeTag` — rename a node's tag in place (NodeId stays valid).
    fn set_node_tag(&mut self, id: NodeId, new_tag: &str) {
        self.doc.rename_tag(id, new_tag);
    }

    /// Port of `prepDocument` — remove comments, styles, replace `<br>` chains, `<font>` → `<span>`.
    fn prep_document(&mut self) {
        self.remove_comments();

        let root = self.doc.root();
        let styles = self.doc.get_elements_by_tag_name(root, "style");
        self.remove_nodes(styles, None::<fn(&Document, NodeId) -> bool>);

        if let Some(body) = self.doc.body() {
            self.replace_brs(body);
        }

        let root = self.doc.root();
        let fonts = self.doc.get_elements_by_tag_name(root, "font");
        self.replace_node_tags(fonts, "span");
    }

    /// Port of `replaceBrs` — replace runs of 2+ `<br>` with `<p>` elements.
    fn replace_brs(&mut self, elem: NodeId) {
        self.replace_brs_finder(elem);
    }

    fn replace_brs_finder(&mut self, n: NodeId) {
        // Get the first child (any type) by using ego_tree's native traversal.
        let first = self.doc.html.tree.get(n).and_then(|x| x.first_child().map(|c| c.id()));
        let mut cur = first;

        while let Some(child) = cur {
            // Capture next sibling before any mutations.
            let next_sib = self.doc.html.tree.get(child).and_then(|x| x.next_sibling().map(|s| s.id()));
            let tag = self.doc.tag_name(child).to_string();

            if tag == "pre" {
                cur = next_sib;
                continue;
            }
            if tag == "br" {
                let new_node = self.replace_br(child);
                // Continue from after the new node.
                cur = self.doc.html.tree.get(new_node).and_then(|x| x.next_sibling().map(|s| s.id()));
                continue;
            }
            if !tag.is_empty() {
                // Element (not pre/br): recurse.
                self.replace_brs_finder(child);
            }
            cur = next_sib;
        }
    }

    /// Replace a single `<br>` with a `<p>` if it's part of a chain of 2+ `<br>`s.
    ///
    /// Returns the original `br` NodeId if no replacement happened, or the new `<p>` NodeId.
    fn replace_br(&mut self, br: NodeId) -> NodeId {
        // Collect the chain: skip whitespace-only nodes; stop at non-<br> elements.
        let mut next = self.doc.html.tree.get(br).and_then(|x| x.next_sibling().map(|s| s.id()));
        let mut replaced = false;

        loop {
            // Skip whitespace-only nodes.
            let advanced = self.advance_past_whitespace_siblings(next);
            next = advanced;
            let Some(n) = next else { break };
            if self.doc.tag_name(n) != "br" {
                break;
            }
            replaced = true;
            let after = self.doc.html.tree.get(n).and_then(|x| x.next_sibling().map(|s| s.id()));
            self.doc.remove(n);
            next = after;
        }

        if !replaced {
            return br;
        }

        // Replace the first `<br>` with a new `<p>`.
        let p = self.doc.create_element("p");
        let br_parent = self.doc.parent(br).unwrap_or_else(|| self.doc.root());
        self.doc.insert_before(br, p);
        self.doc.remove(br);

        // Absorb phrasing-content siblings into the new `<p>`.
        let mut sib = self.doc.html.tree.get(p).and_then(|x| x.next_sibling().map(|s| s.id()));
        while let Some(s) = sib {
            // Stop at a second `<br>` run.
            if self.doc.tag_name(s) == "br" {
                let nxt = self.doc.html.tree.get(s).and_then(|x| x.next_sibling().map(|s2| s2.id()));
                let next_elem = nxt.and_then(|n| self.advance_past_whitespace_siblings(Some(n)));
                if next_elem.map(|ne| self.doc.tag_name(ne) == "br").unwrap_or(false) {
                    break;
                }
            }
            if !self.is_phrasing_content(s) {
                break;
            }
            let after = self.doc.html.tree.get(s).and_then(|x| x.next_sibling().map(|s2| s2.id()));
            self.doc.append_child(p, s);
            sib = after;
        }

        // Trim trailing whitespace from the new `<p>`.
        loop {
            let last = self.doc.html.tree.get(p).and_then(|x| x.last_child().map(|c| c.id()));
            match last {
                None => break,
                Some(l) if self.is_whitespace(l) => { self.doc.remove(l); }
                _ => break,
            }
        }

        // If `<p>` ended up inside another `<p>`, promote parent to `<div>`.
        if self.doc.parent(p).map(|par| self.doc.tag_name(par) == "p").unwrap_or(false) {
            let parent_p = self.doc.parent(p).unwrap();
            self.set_node_tag(parent_p, "div");
        }

        let _ = br_parent; // used earlier for context
        p
    }

    /// Advance past text nodes that are whitespace-only; return the next non-whitespace-text or element node.
    fn advance_past_whitespace_siblings(&self, start: Option<NodeId>) -> Option<NodeId> {
        let mut cur = start;
        while let Some(n) = cur {
            let is_elem = self.doc.is_element(n);
            if is_elem {
                return Some(n);
            }
            if has_text_content(&self.doc, n) {
                return Some(n);
            }
            cur = self.doc.html.tree.get(n).and_then(|x| x.next_sibling().map(|s| s.id()));
        }
        None
    }

    // ── Image handling ────────────────────────────────────────────────────

    #[allow(dead_code)] // used in Phase 6 (prepArticle)
    /// Port of `fixLazyImages` — convert data-src / lazy-loaded images to real src attrs.
    fn fix_lazy_images(&mut self, root: NodeId) {
        let nodes = self.doc.get_all_nodes_with_tag(root, &["img", "picture", "figure"]);
        for elem in nodes {
            let src = self.doc.attr(elem, "src").unwrap_or("").to_string();
            let tag = self.doc.tag_name(elem).to_string();
            let class = self.doc.attr(elem, "class").unwrap_or("").to_string();

            // Remove tiny base64 placeholders if another attribute has the real image.
            if !src.is_empty() && RX_B64_DATA_URL.is_match(&src) {
                let mime = RX_B64_DATA_URL
                    .captures(&src)
                    .and_then(|c| c.get(1))
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default();

                if mime != "image/svg+xml" {
                    // Check if another attribute has a real image URL.
                    let attrs = self.doc.get_all_attrs(elem);
                    let src_removable = attrs.iter().any(|(k, v)| {
                        k != "src" && RX_IMG_EXTENSIONS.is_match(v) && is_valid_url(v)
                    });
                    if src_removable {
                        let b64_start = src.find("base64").map(|i| i + 7).unwrap_or(src.len());
                        if src.len() - b64_start < 133 {
                            self.doc.remove_attr(elem, "src");
                        }
                    }
                }
            }

            // Re-read src/srcset after potential removal.
            let src = self.doc.attr(elem, "src").unwrap_or("").to_string();
            let srcset = self.doc.attr(elem, "srcset").unwrap_or("").to_string();
            if (!src.is_empty() || !srcset.is_empty()) && !class.to_lowercase().contains("lazy") {
                continue;
            }

            // Copy lazy-load attributes to src/srcset.
            let attrs = self.doc.get_all_attrs(elem);
            for (attr_key, attr_val) in attrs {
                if attr_key == "src" || attr_key == "srcset" || attr_key == "alt" {
                    continue;
                }
                let copy_to = if RX_LAZY_IMAGE_SRCSET.is_match(&attr_val) {
                    "srcset"
                } else if RX_LAZY_IMAGE_SRC.is_match(&attr_val) {
                    "src"
                } else {
                    continue;
                };
                if !is_valid_url(&attr_val) {
                    continue;
                }
                if tag == "img" || tag == "picture" {
                    self.doc.set_attr(elem, copy_to, &attr_val);
                } else if tag == "figure" {
                    let has_img = !self.doc.get_all_nodes_with_tag(elem, &["img", "picture"]).is_empty();
                    if !has_img {
                        let img = self.doc.create_element("img");
                        self.doc.set_attr(img, copy_to, &attr_val);
                        self.doc.append_child(elem, img);
                    }
                }
            }
        }
    }

    /// Port of `unwrapNoscriptImages` — replace lazy-load `<img>` placeholders with the
    /// real image from the adjacent `<noscript>` tag.
    fn unwrap_noscript_images(&mut self) {
        // Step 1: Remove <img> elements that have no source-like attributes.
        let root = self.doc.root();
        let imgs = self.doc.get_elements_by_tag_name(root, "img");
        let to_remove: Vec<NodeId> = imgs
            .into_iter()
            .filter(|&img| {
                let attrs = self.doc.get_all_attrs(img);
                !attrs.iter().any(|(k, v)| {
                    matches!(k.as_str(), "src" | "data-src" | "srcset" | "data-srcset")
                        || RX_IMG_EXTENSIONS.is_match(v)
                })
            })
            .collect();
        for img in to_remove {
            self.doc.remove(img);
        }

        // Step 2: Replace <noscript> with its contained image when preceded by a single-image element.
        let root = self.doc.root();
        let noscripts = self.doc.get_elements_by_tag_name(root, "noscript");
        for noscript in noscripts {
            if self.doc.parent(noscript).is_none() {
                continue; // already removed
            }

            let content = self.doc.text_content(noscript);
            let fragment = Document::parse(&content);
            let Some(frag_body) = fragment.body() else { continue };
            if !is_single_image_in(&fragment, frag_body) {
                continue;
            }

            let prev = self.doc.prev_element_sibling(noscript);

            if let Some(prev_elem) = prev {
                if is_single_image_in(&self.doc, prev_elem) {
                    // Find the prev img element.
                    let prev_img = if self.doc.tag_name(prev_elem) == "img" {
                        prev_elem
                    } else if let Some(i) = self.doc.get_elements_by_tag_name(prev_elem, "img").into_iter().next() {
                        i
                    } else {
                        continue;
                    };

                    // Get fragment img attrs.
                    let frag_img = match fragment.get_elements_by_tag_name(frag_body, "img").into_iter().next() {
                        Some(i) => i,
                        None => continue,
                    };
                    let frag_attrs = fragment.get_all_attrs(frag_img);

                    // Create replacement img in main tree.
                    let new_img = self.doc.create_element("img");
                    // Copy fragment img attrs first.
                    for (k, v) in &frag_attrs {
                        self.doc.set_attr(new_img, k, v);
                    }
                    // Copy image-relevant attrs from prev_img.
                    let prev_attrs = self.doc.get_all_attrs(prev_img);
                    for (k, v) in prev_attrs {
                        if v.is_empty() {
                            continue;
                        }
                        if k == "src" || k == "srcset" || RX_IMG_EXTENSIONS.is_match(&v) {
                            let existing = self.doc.attr(new_img, &k).map(|s| s.to_string()).unwrap_or_default();
                            if existing == v {
                                continue;
                            }
                            let dest_key = if self.doc.has_attribute(new_img, &k) {
                                format!("data-old-{k}")
                            } else {
                                k
                            };
                            self.doc.set_attr(new_img, &dest_key, &v);
                        }
                    }

                    // Replace prev_elem with the new img.
                    self.doc.insert_before(prev_elem, new_img);
                    self.doc.remove(prev_elem);
                    self.doc.remove(noscript);
                    continue;
                }
            }

            // No prev single-image element: replace noscript with the fragment's first element.
            let frag_first = match fragment.first_element_child(frag_body) {
                Some(f) => f,
                None => continue,
            };
            let actual_img = if fragment.tag_name(frag_first) == "img" {
                frag_first
            } else {
                match fragment.get_elements_by_tag_name(frag_first, "img").into_iter().next() {
                    Some(i) => i,
                    None => continue,
                }
            };

            // Skip 1×1 pixel images.
            let w = fragment.attr(actual_img, "width").unwrap_or("");
            let h = fragment.attr(actual_img, "height").unwrap_or("");
            if w == "1" && h == "1" {
                continue;
            }

            // Create a copy of the img in the main tree.
            let new_img = self.doc.create_element("img");
            for (k, v) in fragment.get_all_attrs(actual_img) {
                self.doc.set_attr(new_img, &k, &v);
            }
            self.doc.insert_before(noscript, new_img);
            self.doc.remove(noscript);
        }
    }

    // ── Post-processing ───────────────────────────────────────────────────

    /// Port of `postProcessContent` — fix URIs, simplify nesting, strip classes.
    fn post_process_content(&mut self, article_content: NodeId) {
        self.fix_relative_uris(article_content);
        self.simplify_nested_elements(article_content);
        if !self.keep_classes {
            self.clean_classes(article_content);
        }
        self.clear_readability_attr(article_content);
    }

    /// Port of `cleanClasses` — strip class attributes, preserving `classes_to_preserve`.
    fn clean_classes(&mut self, id: NodeId) {
        let preserve: HashSet<String> = self.classes_to_preserve.iter().cloned().collect();
        self.clean_classes_impl(id, &preserve);
    }

    fn clean_classes_impl(&mut self, id: NodeId, preserve: &HashSet<String>) {
        if self.doc.is_element(id) {
            if let Some(cls) = self.doc.attr(id, "class") {
                let kept: Vec<&str> = cls.split_whitespace()
                    .filter(|c| preserve.contains(*c))
                    .collect();
                if kept.is_empty() {
                    self.doc.remove_attr(id, "class");
                } else {
                    let new_cls = kept.join(" ");
                    self.doc.set_attr(id, "class", &new_cls);
                }
            }
        }
        for child in self.doc.child_nodes(id) {
            self.clean_classes_impl(child, preserve);
        }
    }

    /// Port of `fixRelativeURIs` — convert relative links and media URLs to absolute.
    fn fix_relative_uris(&mut self, article_content: NodeId) {
        let base_uri = self.document_uri.clone();

        // Fix <a href> links.
        let links = self.doc.get_elements_by_tag_name(article_content, "a");
        for link in links {
            let href = self.doc.attr(link, "href").unwrap_or("").to_string();
            if href.is_empty() {
                continue;
            }

            if href.starts_with("javascript:") {
                let children = self.doc.child_nodes(link);
                if children.len() == 1 {
                    if let Some(Node::Text(t)) = self.doc.html.tree.get(children[0]).map(|n| n.value()) {
                        let text_content = t.text.as_ref().to_string();
                        let text_node = self.doc.create_text_node(&text_content);
                        self.doc.insert_before(link, text_node);
                        self.doc.remove(link);
                    } else {
                        let span = self.doc.create_element("span");
                        let kids: Vec<NodeId> = self.doc.child_nodes(link);
                        for kid in kids {
                            self.doc.append_child(span, kid);
                        }
                        self.doc.insert_before(link, span);
                        self.doc.remove(link);
                    }
                } else {
                    let span = self.doc.create_element("span");
                    let kids: Vec<NodeId> = self.doc.child_nodes(link);
                    for kid in kids {
                        self.doc.append_child(span, kid);
                    }
                    self.doc.insert_before(link, span);
                    self.doc.remove(link);
                }
                continue;
            }

            if let Some(base) = &base_uri {
                let new_href = to_absolute_uri(&href, base);
                if new_href.is_empty() {
                    self.doc.remove_attr(link, "href");
                } else {
                    self.doc.set_attr(link, "href", &new_href);
                }
            }
        }

        // Fix media elements (src, poster, srcset).
        let medias = self.doc.get_all_nodes_with_tag(
            article_content,
            &["img", "picture", "figure", "video", "audio", "source"],
        );
        for media in medias {
            if let Some(base) = &base_uri.clone() {
                if let Some(src) = self.doc.attr(media, "src").map(|s| s.to_string()) {
                    if !src.is_empty() {
                        self.doc.set_attr(media, "src", &to_absolute_uri(&src, base));
                    }
                }
                if let Some(poster) = self.doc.attr(media, "poster").map(|s| s.to_string()) {
                    if !poster.is_empty() {
                        self.doc.set_attr(media, "poster", &to_absolute_uri(&poster, base));
                    }
                }
                if let Some(srcset) = self.doc.attr(media, "srcset").map(|s| s.to_string()) {
                    if !srcset.is_empty() {
                        let base_clone = base.clone();
                        let new_srcset = RX_SRCSET_URL
                            .replace_all(&srcset, |caps: &regex::Captures| {
                                let url = caps.get(1).map_or("", |m| m.as_str());
                                let size = caps.get(2).map_or("", |m| m.as_str());
                                let sep = caps.get(3).map_or("", |m| m.as_str());
                                format!("{}{}{}", to_absolute_uri(url, &base_clone), size, sep)
                            })
                            .into_owned();
                        self.doc.set_attr(media, "srcset", &new_srcset);
                    }
                }
            }
        }
    }

    /// Port of `simplifyNestedElements` — collapse empty or redundant div/section wrappers.
    fn simplify_nested_elements(&mut self, article_content: NodeId) {
        let mut node = self.doc.first_element_child(article_content);

        while let Some(n) = node {
            let parent = self.doc.parent(n);
            let tag = self.doc.tag_name(n).to_string();
            let node_id_attr = self.doc.attr(n, "id").unwrap_or("").to_string();

            if parent.is_some()
                && (tag == "div" || tag == "section")
                && !node_id_attr.starts_with("readability")
            {
                if self.is_element_without_content(n) {
                    node = self.remove_and_get_next(n);
                    continue;
                }
                if self.has_single_tag_inside_element(n, "div")
                    || self.has_single_tag_inside_element(n, "section")
                {
                    let child = self.doc.first_element_child(n).unwrap();
                    // Copy parent attrs to child.
                    let parent_attrs = self.doc.get_all_attrs(n);
                    for (k, v) in parent_attrs {
                        self.doc.set_attr(child, &k, &v);
                    }
                    // Replace n with child.
                    self.doc.insert_before(n, child);
                    self.doc.remove(n);
                    node = Some(child);
                    continue;
                }
            }
            node = self.get_next_node(n, false);
        }
    }

    /// Port of `clearReadabilityAttr` — remove `data-readability-*` attributes.
    ///
    /// In this Rust port we use side tables instead of DOM attributes, so these
    /// attributes are never set. This is a no-op kept for structural completeness.
    fn clear_readability_attr(&mut self, _id: NodeId) {
        // Side tables are dropped at end of parse_and_mutate; nothing to clean.
    }

    #[allow(dead_code)] // used in Phase 6 (prepArticle)
    /// Port of `cleanStyles` — remove presentational attributes from all elements.
    fn clean_styles(&mut self, id: NodeId) {
        let tag = self.doc.tag_name(id).to_string();
        if tag == "svg" {
            return;
        }

        let is_size_elem = DEPRECATED_SIZE_ATTR_ELEMS.contains(&tag.as_str());

        let attrs_to_remove: Vec<String> = {
            self.doc.get_all_attrs(id).into_iter()
                .filter_map(|(k, _)| {
                    if (k == "width" || k == "height") && !is_size_elem {
                        return Some(k);
                    }
                    if PRESENTATIONAL_ATTRS.contains(&k.as_str()) {
                        return Some(k);
                    }
                    None
                })
                .collect()
        };

        for attr in attrs_to_remove {
            self.doc.remove_attr(id, &attr);
        }

        for child in self.doc.children(id) {
            self.clean_styles(child);
        }
    }

    // ── Metadata extraction ───────────────────────────────────────────────

    /// Port of `getArticleTitle` — extract and clean the page title.
    fn get_article_title(&self) -> String {
        let title_node = self.get_element_by_tag_name(self.doc.root(), "title");
        let orig_title = title_node
            .map(|t| self.get_inner_text(t, true))
            .unwrap_or_default();
        let mut cur_title = orig_title.clone();
        let mut had_hierarchical_sep = false;

        if RX_TITLE_SEPARATOR.is_match(&cur_title) {
            had_hierarchical_sep = RX_TITLE_HIERARCHY_SEP.is_match(&cur_title);
            cur_title = RX_TITLE_REMOVE_FINAL_PART
                .replace(&orig_title, "$1")
                .into_owned();

            if word_count(&cur_title) < 3 {
                cur_title = RX_TITLE_REMOVE_1ST_PART
                    .replace(&orig_title, "$1")
                    .into_owned();
            }
        } else if cur_title.contains(": ") {
            let root = self.doc.root();
            let headings = self.doc.get_all_nodes_with_tag(root, &["h1", "h2"]);
            let trimmed = cur_title.trim().to_string();
            let match_found = headings.iter().any(|&h| {
                self.doc.text_content(h).trim() == trimmed.as_str()
            });

            if !match_found {
                // Port of strings.LastIndex(origTitle, ":") + 1 — match Go exactly.
                // Leading space after ':' is stripped by normalize_spaces(trim()) below.
                let last_colon = orig_title.rfind(':').map(|i| i + 1).unwrap_or(0);
                cur_title = orig_title[last_colon..].to_string();

                if word_count(&cur_title) < 3 {
                    let first_colon = orig_title.find(':').map(|i| i + 1).unwrap_or(0);
                    cur_title = orig_title[first_colon..].to_string();
                } else {
                    let pre_colon_words = orig_title
                        .find(": ")
                        .map(|i| word_count(&orig_title[..i]))
                        .unwrap_or(0);
                    if pre_colon_words > 5 {
                        cur_title = orig_title.clone();
                    }
                }
            }
        } else if char_count(&cur_title) > 150 || char_count(&cur_title) < 15 {
            let root = self.doc.root();
            let h1s = self.doc.get_elements_by_tag_name(root, "h1");
            if h1s.len() == 1 {
                cur_title = self.get_inner_text(h1s[0], true);
            }
        }

        cur_title = normalize_spaces(cur_title.trim());

        let cur_word_count = word_count(&cur_title);
        let tmp_orig = RX_TITLE_ANY_SEPARATOR.replace_all(&orig_title, "").into_owned();

        if cur_word_count <= 4
            && (!had_hierarchical_sep || cur_word_count != word_count(&tmp_orig).saturating_sub(1))
        {
            cur_title = orig_title;
        }

        cur_title
    }

    /// Port of `getJSONLD` — extract Schema.org metadata from `<script type="application/ld+json">`.
    fn get_jsonld(&self) -> HashMap<String, String> {
        let mut metadata: Option<HashMap<String, String>> = None;

        let root = self.doc.root();
        let scripts = self.doc.query_selector_all(root, r#"script[type="application/ld+json"]"#);

        for script in scripts {
            if metadata.is_some() {
                break;
            }

            let content = self.doc.text_content(script);
            let content = RX_CDATA.replace_all(&content, "").into_owned();

            let parsed: serde_json::Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Find the right object (may be an array of items, or a @graph, or a direct object).
            // Go only validates @context for the top-level Object case, not for Array items.
            let (obj, validate_context) = match parsed {
                serde_json::Value::Array(ref arr) => {
                    let found = arr.iter()
                        .find(|item| {
                            item.get("@type")
                                .and_then(|t| t.as_str())
                                .map(|t| RX_JSON_LD_ARTICLE_TYPES.is_match(t))
                                .unwrap_or(false)
                        })
                        .and_then(|v| v.as_object());
                    (found, false) // Go skips @context check for array items
                }
                serde_json::Value::Object(ref m) => (Some(m), true),
                _ => continue,
            };

            let obj = match obj {
                Some(o) => o,
                None => continue,
            };

            // Validate @context is schema.org (only for top-level Object, not array items).
            if validate_context {
                let context_ok = match obj.get("@context") {
                    Some(serde_json::Value::String(s)) => RX_SCHEMA_ORG.is_match(s),
                    Some(serde_json::Value::Object(m)) => {
                        m.get("@vocab")
                            .and_then(|v| v.as_str())
                            .map(|s| RX_SCHEMA_ORG.is_match(s))
                            .unwrap_or(false)
                    }
                    _ => false,
                };
                if !context_ok {
                    continue;
                }
            }

            // If no @type, look in @graph for an article type.
            let final_obj: &serde_json::Map<String, serde_json::Value>;
            let graph_obj; // storage for borrowed value
            if !obj.contains_key("@type") {
                let graph = match obj.get("@graph").and_then(|g| g.as_array()) {
                    Some(g) => g,
                    None => continue,
                };
                graph_obj = graph
                    .iter()
                    .find(|item| {
                        item.get("@type")
                            .and_then(|t| t.as_str())
                            .map(|t| RX_JSON_LD_ARTICLE_TYPES.is_match(t))
                            .unwrap_or(false)
                    })
                    .and_then(|v| v.as_object());
                match graph_obj {
                    Some(go) => final_obj = go,
                    None => continue,
                }
            } else {
                final_obj = obj;
            }

            // Validate @type.
            let type_ok = final_obj
                .get("@type")
                .and_then(|t| t.as_str())
                .map(|t| RX_JSON_LD_ARTICLE_TYPES.is_match(t))
                .unwrap_or(false);
            if !type_ok {
                continue;
            }

            let mut meta = HashMap::new();

            // Title: prefer name/headline whichever better matches HTML title.
            let name = final_obj.get("name").and_then(|v| v.as_str()).map(str::trim);
            let headline = final_obj.get("headline").and_then(|v| v.as_str()).map(str::trim);
            match (name, headline) {
                (Some(n), Some(h)) if n != h => {
                    let title = self.get_article_title();
                    let name_matches = self.text_similarity(n, &title) > 0.75;
                    let headline_matches = self.text_similarity(h, &title) > 0.75;
                    if headline_matches && !name_matches {
                        meta.insert("title".to_string(), h.to_string());
                    } else {
                        meta.insert("title".to_string(), n.to_string());
                    }
                }
                (Some(n), _) => { meta.insert("title".to_string(), n.to_string()); }
                (_, Some(h)) => { meta.insert("title".to_string(), h.to_string()); }
                _ => {}
            }

            // Author.
            match final_obj.get("author") {
                Some(serde_json::Value::Object(a)) => {
                    if let Some(n) = a.get("name").and_then(|v| v.as_str()) {
                        meta.insert("byline".to_string(), n.trim().to_string());
                    }
                }
                Some(serde_json::Value::Array(arr)) => {
                    let authors: Vec<&str> = arr
                        .iter()
                        .filter_map(|a| a.get("name")?.as_str())
                        .collect();
                    meta.insert("byline".to_string(), authors.join(", "));
                }
                _ => {}
            }

            // Description / excerpt.
            if let Some(desc) = final_obj.get("description").and_then(|v| v.as_str()) {
                meta.insert("excerpt".to_string(), desc.trim().to_string());
            }

            // Publisher / site name.
            if let Some(pub_name) = final_obj
                .get("publisher")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
            {
                meta.insert("siteName".to_string(), pub_name.trim().to_string());
            }

            // Date published.
            if let Some(dp) = final_obj.get("datePublished").and_then(|v| v.as_str()) {
                meta.insert("datePublished".to_string(), dp.to_string());
            }

            metadata = Some(meta);
        }

        metadata.unwrap_or_default()
    }

    /// Port of `getArticleFavicon` — find the best PNG favicon from `<link>` elements.
    fn get_article_favicon(&self) -> String {
        let root = self.doc.root();
        let links = self.doc.get_elements_by_tag_name(root, "link");

        let mut favicon = String::new();
        let mut favicon_size: i32 = -1;

        for link in links {
            let rel = self.doc.attr(link, "rel").unwrap_or("").trim().to_string();
            let link_type = self.doc.attr(link, "type").unwrap_or("").trim().to_string();
            let href = self.doc.attr(link, "href").unwrap_or("").trim().to_string();
            let sizes = self.doc.attr(link, "sizes").unwrap_or("").trim().to_string();

            if href.is_empty() || !rel.contains("icon") {
                continue;
            }
            if link_type != "image/png" && !href.contains(".png") {
                continue;
            }

            let mut size = 0i32;
            for loc in &[sizes.as_str(), href.as_str()] {
                if let Some(caps) = RX_FAVICON_SIZE.captures(loc) {
                    let w = caps.get(1).map_or("", |m| m.as_str());
                    let h = caps.get(2).map_or("", |m| m.as_str());
                    if w == h {
                        size = w.parse().unwrap_or(0);
                        break;
                    }
                }
            }

            if size > favicon_size {
                favicon_size = size;
                favicon = href;
            }
        }

        if let Some(base) = &self.document_uri {
            to_absolute_uri(&favicon, base)
        } else {
            favicon
        }
    }

    /// Port of `getArticleMetadata` — collect metadata from `<meta>` tags and JSON-LD.
    fn get_article_metadata(&self, json_ld: &HashMap<String, String>) -> HashMap<String, String> {
        let root = self.doc.root();
        let metas = self.doc.get_elements_by_tag_name(root, "meta");
        let mut values: HashMap<String, String> = HashMap::new();

        for meta in metas {
            let element_property = self.doc.attr(meta, "property").unwrap_or("").to_string();
            let element_name = self.doc.attr(meta, "name").unwrap_or("").to_string();
            let content = self.doc.attr(meta, "content").unwrap_or("").to_string();

            if content.is_empty() {
                continue;
            }

            let mut matches: Vec<String> = Vec::new();

            if !element_property.is_empty() {
                // Go processes matches in reverse order, so first match wins.
                let all_matches: Vec<_> = RX_PROPERTY_PATTERN.find_iter(&element_property).collect();
                for m in all_matches.into_iter().rev() {
                    let name = m.as_str().to_lowercase();
                    let name: String = name.split_whitespace().collect();
                    matches.push(name.clone());
                    values.insert(name, content.trim().to_string());
                }
            }

            if matches.is_empty() && !element_name.is_empty() && RX_NAME_PATTERN.is_match(&element_name) {
                let name = element_name.to_lowercase();
                let name: String = name.split_whitespace().collect();
                let name = name.replace('.', ":");
                values.insert(name, content.trim().to_string());
            }
        }

        let empty = String::new();
        let jl = json_ld;

        // Build a helper to look up in values map with fallback to empty.
        let v = |key: &str| values.get(key).unwrap_or(&empty).as_str();
        let j = |key: &str| jl.get(key).map(|s| s.as_str()).unwrap_or("");

        let metadata_title = {
            let t = str_or(&[
                j("title"),
                v("dc:title"), v("dcterm:title"), v("og:title"),
                v("weibo:article:title"), v("weibo:webpage:title"),
                v("title"), v("twitter:title"), v("parsely-title"),
            ]);
            if t.is_empty() { self.get_article_title() } else { t.to_string() }
        };

        let metadata_byline = {
            let b = str_or(&[
                j("byline"),
                v("dc:creator"), v("dcterm:creator"), v("author"), v("parsely-author"),
            ]);
            if b.is_empty() {
                let article_author = v("article:author");
                if !article_author.is_empty() && !is_valid_url(article_author) {
                    article_author.to_string()
                } else {
                    b.to_string()
                }
            } else {
                b.to_string()
            }
        };

        let metadata_excerpt = str_or(&[
            j("excerpt"),
            v("dc:description"), v("dcterm:description"), v("og:description"),
            v("weibo:article:description"), v("weibo:webpage:description"),
            v("description"), v("twitter:description"),
        ]).to_string();

        let metadata_site_name = str_or(&[j("siteName"), v("og:site_name")]).to_string();

        let metadata_image = str_or(&[v("og:image"), v("image"), v("twitter:image")]).to_string();

        let metadata_favicon = self.get_article_favicon();

        let metadata_published_time = str_or(&[
            j("datePublished"),
            v("article:published_time"), v("dcterms.available"),
            v("dcterms.created"), v("dcterms.issued"),
            v("weibo:article:create_at"), v("parsely-pub-date"),
        ]).to_string();

        let metadata_modified_time = str_or(&[
            j("dateModified"),
            v("article:modified_time"), v("dcterms.modified"),
        ]).to_string();

        // HTML-unescape field values (in case of double-encoded entities in meta tags).
        let mut result = HashMap::new();
        result.insert("title".to_string(), html_unescape(&metadata_title));
        result.insert("byline".to_string(), html_unescape(&metadata_byline));
        result.insert("excerpt".to_string(), html_unescape(&metadata_excerpt));
        result.insert("siteName".to_string(), html_unescape(&metadata_site_name));
        result.insert("image".to_string(), metadata_image);
        result.insert("favicon".to_string(), metadata_favicon);
        result.insert("publishedTime".to_string(), html_unescape(&metadata_published_time));
        result.insert("modifiedTime".to_string(), html_unescape(&metadata_modified_time));
        result
    }

    // ── Phase 6 stubs (grabArticle and helpers) ───────────────────────────

    /// Port of `grabArticle` — score and select article content.
    ///
    /// Phase 6 implementation — returns `None` until scoring is ported.
    #[allow(dead_code)]
    fn grab_article(&mut self) -> Option<NodeId> {
        None // TODO: Phase 6
    }
}

// ── Default ───────────────────────────────────────────────────────────────────

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

// ── Free helpers (not methods — may operate on a foreign Document) ────────────

/// Port of `isSingleImage` as a free function operating on any Document.
fn is_single_image_in(doc: &Document, id: NodeId) -> bool {
    if doc.tag_name(id) == "img" {
        return true;
    }
    let children = doc.children(id);
    if children.len() != 1 || has_text_content(doc, id) {
        return false;
    }
    is_single_image_in(doc, children[0])
}

/// Minimal HTML entity decoder.
///
/// Handles the most common named entities and numeric character references.
/// Attribute values parsed by html5ever are already decoded; this is a safety pass
/// for occasionally double-encoded metadata fields.
fn html_unescape(s: &str) -> String {
    use std::sync::LazyLock;
    static RX_ENTITY: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"&(?:#x([0-9a-fA-F]+)|#([0-9]+)|([a-zA-Z][a-zA-Z0-9]*));").unwrap()
    });

    if !s.contains('&') {
        return s.to_string();
    }

    RX_ENTITY
        .replace_all(s, |caps: &regex::Captures| {
            if let Some(hex) = caps.get(1) {
                let code = u32::from_str_radix(hex.as_str(), 16).unwrap_or(0xFFFD);
                char::from_u32(code).map(|c| c.to_string()).unwrap_or_else(|| "\u{FFFD}".to_string())
            } else if let Some(dec) = caps.get(2) {
                let code: u32 = dec.as_str().parse().unwrap_or(0xFFFD);
                char::from_u32(code).map(|c| c.to_string()).unwrap_or_else(|| "\u{FFFD}".to_string())
            } else if let Some(name) = caps.get(3) {
                named_html_entity(name.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| caps[0].to_string()) // keep unknown entities
            } else {
                caps[0].to_string()
            }
        })
        .into_owned()
}

fn named_html_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&", "lt" => "<", "gt" => ">",
        "quot" => "\"", "apos" => "'",
        "nbsp" => "\u{00A0}", "shy" => "\u{00AD}",
        "mdash" => "\u{2014}", "ndash" => "\u{2013}",
        "lsquo" => "\u{2018}", "rsquo" => "\u{2019}",
        "ldquo" => "\u{201C}", "rdquo" => "\u{201D}",
        "hellip" => "\u{2026}", "bull" => "\u{2022}",
        "copy" => "\u{00A9}", "reg" => "\u{00AE}",
        "trade" => "\u{2122}", "euro" => "\u{20AC}",
        "pound" => "\u{00A3}", "yen" => "\u{00A5}",
        "cent" => "\u{00A2}",
        _ => return None,
    })
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::Document;

    fn make_parser() -> Parser {
        Parser::new()
    }

    // ── html_unescape ──────────────────────────────────────────────────────

    #[test]
    fn html_unescape_named_entities() {
        assert_eq!(html_unescape("foo &amp; bar"), "foo & bar");
        assert_eq!(html_unescape("&lt;p&gt;"), "<p>");
        assert_eq!(html_unescape("he said &quot;hi&quot;"), "he said \"hi\"");
    }

    #[test]
    fn html_unescape_numeric_entities() {
        assert_eq!(html_unescape("&#65;"), "A"); // decimal
        assert_eq!(html_unescape("&#x41;"), "A"); // hex
    }

    #[test]
    fn html_unescape_unknown_entity_preserved() {
        assert_eq!(html_unescape("&unknown;"), "&unknown;");
    }

    #[test]
    fn html_unescape_no_ampersand_passthrough() {
        let s = "no entities here";
        assert_eq!(html_unescape(s), s);
    }

    // ── text_similarity ────────────────────────────────────────────────────

    #[test]
    fn text_similarity_identical_is_one() {
        let p = make_parser();
        assert!((p.text_similarity("hello world", "hello world") - 1.0).abs() < 1e-9);
    }

    #[test]
    fn text_similarity_disjoint_is_zero() {
        let p = make_parser();
        assert!((p.text_similarity("foo bar", "baz qux")).abs() < 1e-9);
    }

    // ── get_article_title ──────────────────────────────────────────────────

    #[test]
    fn get_article_title_simple() {
        let mut p = make_parser();
        let doc = Document::parse("<html><head><title>Hello World</title></head><body></body></html>");
        p.doc = doc;
        assert_eq!(p.get_article_title(), "Hello World");
    }

    #[test]
    fn get_article_title_with_separator() {
        let mut p = make_parser();
        // Pre-separator part must be >4 words for the fallback NOT to restore the original title.
        let doc = Document::parse(
            "<html><head><title>A Five Word Long Article Title | Site Name</title></head><body></body></html>",
        );
        p.doc = doc;
        let title = p.get_article_title();
        // Should strip the site name part (more than 4 words before separator → no fallback).
        assert_eq!(title, "A Five Word Long Article Title");
    }

    // ── remove_comments ────────────────────────────────────────────────────

    #[test]
    fn remove_comments_clears_html_comments() {
        let mut p = make_parser();
        p.doc = Document::parse("<html><body><!-- hidden -->visible</body></html>");
        p.remove_comments();
        let body = p.doc.body().unwrap();
        let html = p.doc.inner_html(body);
        assert!(!html.contains("hidden"), "comment should be removed: {html}");
        assert!(html.contains("visible"), "text should remain: {html}");
    }

    // ── is_element_without_content ─────────────────────────────────────────

    #[test]
    fn empty_div_is_without_content() {
        let mut p = make_parser();
        p.doc = Document::parse("<html><body><div></div></body></html>");
        let body = p.doc.body().unwrap();
        let div = p.doc.first_element_child(body).unwrap();
        assert!(p.is_element_without_content(div));
    }

    #[test]
    fn div_with_text_is_not_empty() {
        let mut p = make_parser();
        p.doc = Document::parse("<html><body><div>hello</div></body></html>");
        let body = p.doc.body().unwrap();
        let div = p.doc.first_element_child(body).unwrap();
        assert!(!p.is_element_without_content(div));
    }

    // ── clean_classes ──────────────────────────────────────────────────────

    #[test]
    fn clean_classes_removes_non_preserved() {
        let mut p = make_parser();
        p.doc = Document::parse(r#"<html><body><div class="foo page bar">text</div></body></html>"#);
        let body = p.doc.body().unwrap();
        let div = p.doc.first_element_child(body).unwrap();
        p.clean_classes(div);
        let cls = p.doc.attr(div, "class").unwrap_or("");
        assert_eq!(cls, "page", "only 'page' should survive: {cls:?}");
    }

    // ── fix_relative_uris ──────────────────────────────────────────────────

    #[test]
    fn fix_relative_uris_resolves_href() {
        let base = Url::parse("https://example.com/articles/").unwrap();
        let mut p = make_parser();
        p.document_uri = Some(base);
        p.doc = Document::parse(r#"<html><body><a href="/page">link</a></body></html>"#);
        let body = p.doc.body().unwrap();
        p.fix_relative_uris(body);
        let a = p.doc.get_elements_by_tag_name(body, "a").into_iter().next().unwrap();
        assert_eq!(p.doc.attr(a, "href"), Some("https://example.com/page"));
    }

    #[test]
    fn fix_relative_uris_removes_javascript_links() {
        let base = Url::parse("https://example.com/").unwrap();
        let mut p = make_parser();
        p.document_uri = Some(base);
        p.doc = Document::parse(r#"<html><body><a href="javascript:void(0)">click</a></body></html>"#);
        let body = p.doc.body().unwrap();
        p.fix_relative_uris(body);
        // The <a> should be replaced with a text node or <span>
        let links = p.doc.get_elements_by_tag_name(body, "a");
        assert!(links.is_empty(), "javascript link should be removed");
    }

    // ── get_article_favicon ────────────────────────────────────────────────

    #[test]
    fn get_article_favicon_finds_png_icon() {
        let mut p = make_parser();
        p.document_uri = Some(Url::parse("https://example.com/").unwrap());
        p.doc = Document::parse(
            r#"<html><head>
                <link rel="icon" type="image/png" href="/favicon-32x32.png" sizes="32x32">
                <link rel="icon" type="image/png" href="/favicon-16x16.png" sizes="16x16">
            </head><body></body></html>"#,
        );
        let fav = p.get_article_favicon();
        // Should prefer larger size (32x32)
        assert!(fav.contains("32x32"), "got: {fav}");
    }

    // ── parse ──────────────────────────────────────────────────────────────

    #[test]
    fn parse_returns_article_with_metadata() {
        let html = r#"<html>
            <head>
                <title>Test Article</title>
                <meta property="og:description" content="A test description"/>
                <meta property="og:site_name" content="Test Site"/>
            </head>
            <body><p>Hello world</p></body>
        </html>"#;

        let mut p = make_parser();
        let article = p.parse(html, None).unwrap();
        assert_eq!(article.title, "Test Article");
        assert_eq!(article.excerpt, "A test description");
        assert_eq!(article.site_name, "Test Site");
    }
}
