// Port of go-readability/parser.go + parser-parse.go

use std::collections::{HashMap, HashSet};

use ego_tree::NodeId;
use scraper::Node;
use url::Url;

use crate::article::Article;
use crate::dom::Document;
use crate::traverse::{is_comma, CharCounter};
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
    strip_unlikelys: bool,
    use_weight_classes: bool,
    clean_conditionally: bool,
}

/// Saved state from a failed grab_article pass.
struct ParseAttempt {
    article_content: NodeId,
    doc_snapshot: Document,
    text_length: usize,
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

    // ── Per-pass side tables (reset at start of each grab_article pass) ──
    score_map: HashMap<NodeId, f64>,
    data_tables: HashSet<NodeId>,
    attempts: Vec<ParseAttempt>,
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
            score_map: HashMap::new(),
            data_tables: HashSet::new(),
            attempts: Vec::new(),
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
        self.score_map.clear();
        self.data_tables.clear();
        self.attempts.clear();

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

    // ── Score / table side-table accessors ───────────────────────────────

    /// Store a content score for `id` in the per-pass side table.
    fn set_content_score(&mut self, id: NodeId, score: f64) {
        self.score_map.insert(id, score);
    }

    /// Get the content score for `id` (0.0 if not scored).
    fn get_content_score(&self, id: NodeId) -> f64 {
        self.score_map.get(&id).copied().unwrap_or(0.0)
    }

    /// True if `id` has been scored.
    fn has_content_score(&self, id: NodeId) -> bool {
        self.score_map.contains_key(&id)
    }

    /// Mark or unmark `id` as a data (non-layout) table.
    fn set_readability_data_table(&mut self, id: NodeId, is_data: bool) {
        if is_data {
            self.data_tables.insert(id);
        } else {
            self.data_tables.remove(&id);
        }
    }

    /// True if `id` has been marked as a data table.
    fn is_readability_data_table(&self, id: NodeId) -> bool {
        self.data_tables.contains(&id)
    }

    // ── Node initialization ───────────────────────────────────────────────

    /// Port of `initializeNode` — set initial content score from tag name and class weight.
    fn initialize_node(&mut self, id: NodeId) {
        let class_weight = self.get_class_weight(id) as f64;
        let tag_score: f64 = match self.doc.tag_name(id) {
            "div" => 5.0,
            "pre" | "td" | "blockquote" => 3.0,
            "address" | "ol" | "ul" | "dl" | "dd" | "dt" | "li" | "form" => -3.0,
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "th" => -5.0,
            _ => 0.0,
        };
        self.set_content_score(id, class_weight + tag_score);
    }

    // ── Video embed detection ─────────────────────────────────────────────

    /// Port of `isVideoEmbed` — true if the element is an embedded video.
    fn is_video_embed(&self, id: NodeId) -> bool {
        let tag = self.doc.tag_name(id);
        if tag != "object" && tag != "embed" && tag != "iframe" {
            return false;
        }
        let rx = self.allowed_video_regex.as_ref().unwrap_or(&*RX_VIDEOS);
        for (_, val) in self.doc.get_all_attrs(id) {
            if rx.is_match(&val) {
                return true;
            }
        }
        if tag == "object" {
            let inner = self.doc.inner_html(id);
            if rx.is_match(&inner) {
                return true;
            }
        }
        false
    }

    // ── Table analysis ────────────────────────────────────────────────────

    /// Port of `getRowAndColumnCount` — count rows and max columns in a table.
    fn get_row_and_column_count(&self, table: NodeId) -> (usize, usize) {
        let mut rows: usize = 0;
        let mut columns: usize = 0;
        let trs = self.doc.get_elements_by_tag_name(table, "tr");
        for tr in &trs {
            let row_span: usize = self
                .doc
                .attr(*tr, "rowspan")
                .and_then(|s| s.parse().ok())
                .filter(|&v| v > 0)
                .unwrap_or(1);
            rows += row_span;

            let mut cols_in_row: usize = 0;
            let cells = self.doc.get_elements_by_tag_name(*tr, "td");
            for cell in &cells {
                let col_span: usize = self
                    .doc
                    .attr(*cell, "colspan")
                    .and_then(|s| s.parse().ok())
                    .filter(|&v| v > 0)
                    .unwrap_or(1);
                cols_in_row += col_span;
            }
            if cols_in_row > columns {
                columns = cols_in_row;
            }
        }
        (rows, columns)
    }

    /// Port of `markDataTables` — classify each `<table>` as data or layout.
    fn mark_data_tables(&mut self, root: NodeId) {
        let tables = self.doc.get_elements_by_tag_name(root, "table");
        for table in tables {
            // If parent was removed (e.g. nested within a removed table), skip.
            if self.doc.parent(table).is_none() {
                continue;
            }

            // role="presentation" → layout table.
            if self.doc.attr(table, "role").unwrap_or("") == "presentation" {
                self.set_readability_data_table(table, false);
                continue;
            }

            // datatable="0" → layout table.
            if self.doc.attr(table, "datatable").unwrap_or("") == "0" {
                self.set_readability_data_table(table, false);
                continue;
            }

            // summary attribute → data table.
            if self.doc.has_attribute(table, "summary") {
                self.set_readability_data_table(table, true);
                continue;
            }

            // Scan children for structural indicators.
            let (is_data, conclusive) = scan_for_data_table_signals(&self.doc, table);
            if conclusive {
                self.set_readability_data_table(table, is_data);
                continue;
            }

            // Fall back to row/column heuristics.
            let (rows, cols) = self.get_row_and_column_count(table);
            if rows == 1 || cols == 1 {
                self.set_readability_data_table(table, false);
                continue;
            }
            if rows >= 10 || cols > 4 {
                self.set_readability_data_table(table, true);
                continue;
            }
            if rows * cols > 10 {
                self.set_readability_data_table(table, true);
            }
        }
    }

    // ── Cleaning helpers ──────────────────────────────────────────────────

    /// Port of `clean` — remove all elements with `tag` unless they are video embeds.
    fn clean(&mut self, root: NodeId, tag: &str) {
        let nodes = self.doc.get_elements_by_tag_name(root, tag);
        let to_remove: Vec<NodeId> = nodes
            .into_iter()
            .filter(|&id| !self.is_video_embed(id))
            .collect();
        for id in to_remove.into_iter().rev() {
            if self.doc.parent(id).is_some() {
                self.doc.remove(id);
            }
        }
    }

    /// Port of `cleanHeaders` — remove h1/h2 with negative class weight.
    fn clean_headers(&mut self, root: NodeId) {
        let nodes = self.doc.get_all_nodes_with_tag(root, &["h1", "h2"]);
        let to_remove: Vec<NodeId> = nodes
            .into_iter()
            .filter(|&id| self.get_class_weight(id) < 0)
            .collect();
        for id in to_remove.into_iter().rev() {
            if self.doc.parent(id).is_some() {
                self.doc.remove(id);
            }
        }
    }

    /// Port of `cleanConditionally` — remove elements that look like non-content.
    fn clean_conditionally(&mut self, root: NodeId, tag: &str) {
        if !self.flags.clean_conditionally {
            return;
        }
        let nodes = self.doc.get_elements_by_tag_name(root, tag);
        let to_remove: Vec<NodeId> = nodes
            .into_iter()
            .filter(|&node| self.should_clean_conditionally(node, tag))
            .collect();
        for id in to_remove.into_iter().rev() {
            if self.doc.parent(id).is_some() {
                self.doc.remove(id);
            }
        }
    }

    /// Determine whether a single node should be removed by `clean_conditionally`.
    fn should_clean_conditionally(&self, node: NodeId, tag: &str) -> bool {
        // Data tables are never removed.
        if tag == "table" && self.is_readability_data_table(node) {
            return false;
        }

        // Nodes inside data tables are never removed.
        let data_tables = &self.data_tables;
        if self.has_ancestor_tag(node, "table", -1, Some(|_doc: &Document, id: NodeId| {
            data_tables.contains(&id)
        })) {
            return false;
        }

        // Nodes inside <code> blocks are never removed.
        if self.has_ancestor_tag::<fn(&Document, NodeId) -> bool>(node, "code", 3, None) {
            return false;
        }

        let weight = self.get_class_weight(node);
        if weight < 0 {
            return true;
        }

        // Walk the subtree to collect content statistics.
        let mut stats = CondStats::new();
        let is_video_fn = |id: NodeId| self.is_video_embed(id);
        let mut dummy_link_acc = CharCounter::new();
        for child in self.doc.child_nodes(node) {
            walk_cond(
                &self.doc,
                child,
                &mut stats,
                false,
                false,
                false,
                0.0,
                &mut dummy_link_acc,
                &is_video_fn,
            );
        }

        if stats.has_video_embed {
            return false;
        }

        let is_list = tag == "ul"
            || tag == "ol"
            || (stats.chars.total() > 0
                && stats.list_chars.total() as f64 / stats.chars.total() as f64 > 0.9);

        if stats.commas < 10 {
            // Single-text-node ad / loading word check.
            if !stats.inner_text_single.is_empty() {
                let trimmed = stats.inner_text_single.trim();
                if RX_AD_WORDS.is_match(trimmed) || RX_LOADING_WORDS.is_match(trimmed) {
                    return true;
                }
            }

            let total = stats.chars.total() as f64;
            let (text_density, link_density, heading_density) = if total > 0.0 {
                (
                    stats.text_chars.total() as f64 / total,
                    stats.link_chars_weighted / total,
                    stats.heading_chars.total() as f64 / total,
                )
            } else {
                (0.0, 0.0, 0.0)
            };

            const LI_COUNT_OFFSET: i64 = -100;

            let have_to_remove = (stats.img_count > 1
                && (stats.p_count as f64 / stats.img_count as f64) < 0.5
                && !self.has_ancestor_tag::<fn(&Document, NodeId) -> bool>(
                    node, "figure", 3, None,
                ))
                || (!is_list
                    && (stats.li_count as i64 + LI_COUNT_OFFSET) > stats.p_count as i64)
                || ((stats.input_count as f64) > (stats.p_count as f64 / 3.0).floor())
                || (!is_list
                    && heading_density < 0.9
                    && stats.chars.total() < 25
                    && (stats.img_count == 0 || stats.img_count > 2)
                    && link_density > 0.0
                    && !self.has_ancestor_tag::<fn(&Document, NodeId) -> bool>(
                        node, "figure", 3, None,
                    ))
                || (!is_list && weight < 25 && link_density > 0.2)
                || (weight >= 25 && link_density > 0.5)
                || ((stats.embed_count == 1 && stats.chars.total() < 75)
                    || stats.embed_count > 1)
                || (stats.img_count == 0 && text_density == 0.0);

            // Allow simple lists of images to remain.
            if is_list && have_to_remove {
                for child in self.doc.children(node) {
                    if self.doc.children(child).len() > 1 {
                        return have_to_remove;
                    }
                }
                if stats.img_count == stats.li_count {
                    return false;
                }
            }

            return have_to_remove;
        }

        false
    }

    // ── Article preparation ───────────────────────────────────────────────

    /// Port of `prepArticle` — clean article content for display.
    fn prep_article(&mut self, article_content: NodeId) {
        self.mark_data_tables(article_content);
        self.fix_lazy_images(article_content);

        self.clean_conditionally(article_content, "form");
        self.clean_conditionally(article_content, "fieldset");
        self.clean(article_content, "object");
        self.clean(article_content, "embed");
        self.clean(article_content, "footer");
        self.clean(article_content, "link");
        self.clean(article_content, "aside");

        // Remove elements that have "share" in their class/id and are small.
        let share_threshold = self.char_thresholds;
        self.remove_share_elements(article_content, share_threshold);

        self.clean(article_content, "iframe");
        self.clean(article_content, "input");
        self.clean(article_content, "textarea");
        self.clean(article_content, "select");
        self.clean(article_content, "button");
        self.clean_headers(article_content);

        // These last since prior cleaning may affect them.
        self.clean_conditionally(article_content, "table");
        self.clean_conditionally(article_content, "ul");
        self.clean_conditionally(article_content, "div");

        // Replace h1 with h2 — h1 should only appear as the title.
        let h1s = self.doc.get_elements_by_tag_name(article_content, "h1");
        for id in h1s {
            self.doc.rename_tag(id, "h2");
        }

        // Remove empty paragraphs (no meaningful content).
        let ps = self.doc.get_elements_by_tag_name(article_content, "p");
        let to_remove: Vec<NodeId> = ps
            .into_iter()
            .filter(|&p_id| !find_content_in_node(&self.doc, p_id))
            .collect();
        for id in to_remove.into_iter().rev() {
            if self.doc.parent(id).is_some() {
                self.doc.remove(id);
            }
        }

        // Remove <br> immediately before a <p>.
        let brs = self.doc.get_elements_by_tag_name(article_content, "br");
        let to_remove_brs: Vec<NodeId> = brs
            .into_iter()
            .filter(|&br_id| {
                let next_sib = self.doc.next_sibling(br_id);
                next_sib
                    .and_then(|n| self.next_node(n))
                    .map(|n| self.doc.tag_name(n) == "p")
                    .unwrap_or(false)
            })
            .collect();
        for id in to_remove_brs.into_iter().rev() {
            if self.doc.parent(id).is_some() {
                self.doc.remove(id);
            }
        }

        self.clean_styles(article_content);

        // Flatten single-cell tables.
        let tables = self.doc.get_elements_by_tag_name(article_content, "table");
        for table_id in tables {
            if self.doc.parent(table_id).is_none() {
                continue;
            }

            let tbody = if self.has_single_tag_inside_element(table_id, "tbody") {
                self.doc.first_element_child(table_id)
            } else {
                Some(table_id)
            };
            let tbody = match tbody {
                Some(t) => t,
                None => continue,
            };

            if !self.has_single_tag_inside_element(tbody, "tr") {
                continue;
            }
            let row = match self.doc.first_element_child(tbody) {
                Some(r) => r,
                None => continue,
            };

            if !self.has_single_tag_inside_element(row, "td") {
                continue;
            }
            let cell = match self.doc.first_element_child(row) {
                Some(c) => c,
                None => continue,
            };

            let new_tag =
                if self.doc.child_nodes(cell).iter().all(|&c| self.is_phrasing_content(c)) {
                    "p"
                } else {
                    "div"
                };

            self.doc.rename_tag(cell, new_tag);

            // Replace the table with the cell in table's parent.
            self.doc.insert_before(table_id, cell);
            self.doc.remove(table_id);
        }
    }

    /// Remove share-element divs (elements whose class+id contains "share" and whose text is short).
    fn remove_share_elements(&mut self, node: NodeId, share_threshold: usize) {
        // Collect candidates first to avoid borrow issues.
        let children: Vec<NodeId> = self.doc.child_nodes(node);
        for child in children {
            if !self.doc.is_element(child) {
                continue;
            }
            let class = self.doc.attr(child, "class").unwrap_or("").to_string();
            let id_attr = self.doc.attr(child, "id").unwrap_or("").to_string();
            let match_string = format!("{class} {id_attr}");
            if match_string.len() > 1
                && RX_SHARE_ELEMENTS.is_match(&match_string)
                && crate::utils::char_count(&self.doc.text_content(child)) < share_threshold
            {
                if self.doc.parent(child).is_some() {
                    self.doc.remove(child);
                }
            } else {
                self.remove_share_elements(child, share_threshold);
            }
        }
    }

    // ── Main article extraction ───────────────────────────────────────────

    /// Port of `grabArticle` — score and select article content.
    fn grab_article(&mut self) -> Option<NodeId> {
        // Save a pristine snapshot to restore at the start of each pass.
        let base_doc = self.doc.clone();

        loop {
            // Restore the document to a clean state for this pass.
            self.doc = base_doc.clone();
            self.score_map.clear();
            self.data_tables.clear();

            let page = self.doc.body()?;

            // ── Node prepping ─────────────────────────────────────────────
            let mut elements_to_score: Vec<NodeId> = Vec::new();
            let mut node = self.doc.document_element();
            let mut should_remove_title_header = true;

            'grab_loop: while let Some(n) = node {
                let tag = self.doc.tag_name(n).to_string();
                let class = self.doc.attr(n, "class").unwrap_or("").to_string();
                let id_attr = self.doc.attr(n, "id").unwrap_or("").to_string();
                let match_string = format!("{class} {id_attr}");

                if tag == "html" {
                    self.article_lang =
                        self.doc.attr(n, "lang").unwrap_or("").to_string();
                }

                if !self.is_probably_visible(n) {
                    node = self.remove_and_get_next(n);
                    continue;
                }

                // Remove aria-modal="true" role="dialog" elements.
                if self.doc.attr(n, "aria-modal").unwrap_or("") == "true"
                    && self.doc.attr(n, "role").unwrap_or("") == "dialog"
                {
                    node = self.remove_and_get_next(n);
                    continue;
                }

                // Byline detection and removal.
                if self.article_byline.is_empty() && self.is_valid_byline(n, &match_string) {
                    // Look for [itemprop="name"] child for a more accurate byline.
                    let end_marker = self.get_next_node(n, true);
                    let mut next = self.get_next_node(n, false);
                    let mut found_name = false;
                    while let Some(nx) = next {
                        if end_marker.map(|e| e == nx).unwrap_or(false) {
                            break;
                        }
                        let itemprop = self.doc.attr(nx, "itemprop").unwrap_or("").to_string();
                        if itemprop.contains("name") {
                            self.article_byline = self.get_inner_text(nx, false);
                            node = self.remove_and_get_next(n);
                            found_name = true;
                            break;
                        }
                        next = self.get_next_node(nx, false);
                    }
                    if found_name {
                        continue 'grab_loop;
                    }
                    let byline_text = self.get_inner_text(n, false);
                    let n_char = crate::utils::char_count(&byline_text);
                    if n_char > 0 && n_char < 100 {
                        self.article_byline = normalize_spaces(byline_text.trim());
                        node = self.remove_and_get_next(n);
                        continue;
                    }
                }

                if should_remove_title_header && self.header_duplicates_title(n) {
                    should_remove_title_header = false;
                    node = self.remove_and_get_next(n);
                    continue;
                }

                // Remove unlikely candidates.
                if self.flags.strip_unlikelys {
                    if tag != "body"
                        && tag != "a"
                        && crate::regexp::is_unlikely_candidate(&match_string)
                        && !crate::regexp::maybe_its_a_candidate(&match_string)
                        && !self.has_ancestor_tag::<fn(&Document, NodeId) -> bool>(
                            n, "table", 3, None,
                        )
                        && !self.has_ancestor_tag::<fn(&Document, NodeId) -> bool>(
                            n, "code", 3, None,
                        )
                    {
                        node = self.remove_and_get_next(n);
                        continue;
                    }

                    let role = self.doc.attr(n, "role").unwrap_or("").to_string();
                    if UNLIKELY_ROLES.contains(&role.as_str()) {
                        node = self.remove_and_get_next(n);
                        continue;
                    }
                }

                // Remove empty structural elements.
                match tag.as_str() {
                    "div" | "section" | "header" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        if self.is_element_without_content(n) {
                            node = self.remove_and_get_next(n);
                            continue;
                        }
                    }
                    _ => {}
                }

                if self.tags_to_score.contains(&tag) {
                    elements_to_score.push(n);
                }

                // Convert divs to p where appropriate.
                if tag == "div" {
                    // Wrap inline phrasing content into <p> elements.
                    let child_nodes_snap = self.doc.child_nodes(n);
                    let mut p: Option<NodeId> = None;
                    for &child in &child_nodes_snap {
                        if self.doc.parent(child).is_none() {
                            // Already moved.
                            p = None;
                            continue;
                        }
                        if self.is_phrasing_content(child) {
                            if let Some(p_id) = p {
                                self.doc.append_child(p_id, child);
                            } else if !self.is_whitespace(child) {
                                let new_p = self.doc.create_element("p");
                                // Clone child, put clone into new_p, replace child with new_p.
                                let child_clone = clone_node(&mut self.doc, child);
                                self.doc.insert_before(child, new_p);
                                self.doc.append_child(new_p, child_clone);
                                self.doc.remove(child);
                                p = Some(new_p);
                            }
                        } else if p.is_some() {
                            // Trim trailing whitespace from p, then close it.
                            // Port of Go: if p has a next element sibling, move the
                            // whitespace node to after p rather than removing it entirely.
                            // This prevents missing space characters in article.text_content.
                            if let Some(p_id) = p {
                                loop {
                                    let last = last_child_node(&self.doc, p_id);
                                    match last {
                                        Some(l) if self.is_whitespace(l) => {
                                            if let Some(next_elem) = self.doc.next_element_sibling(p_id) {
                                                // Detach from p and reinsert before next sibling.
                                                self.doc.remove(l);
                                                self.doc.insert_before(next_elem, l);
                                            } else {
                                                self.doc.remove(l);
                                            }
                                        }
                                        _ => break,
                                    }
                                }
                            }
                            p = None;
                        }
                    }

                    // div with single p child → promote the p.
                    if self.has_single_tag_inside_element(n, "p")
                        && self.get_link_density(n) < 0.25
                    {
                        let div_id = self.doc.attr(n, "id").unwrap_or("").to_string();
                        let div_class = self.doc.attr(n, "class").unwrap_or("").to_string();
                        let new_node = self.doc.children(n)[0];
                        // Replace div with its child p.
                        self.doc.insert_before(n, new_node);
                        self.doc.remove(n);
                        // Inherit id/class if the promoted node lacks them.
                        if !div_id.is_empty()
                            && self.doc.attr(new_node, "id").unwrap_or("").is_empty()
                        {
                            self.doc.set_attr(new_node, "id", &div_id);
                        }
                        if !div_class.is_empty()
                            && self.doc.attr(new_node, "class").unwrap_or("").is_empty()
                        {
                            self.doc.set_attr(new_node, "class", &div_class);
                        }
                        elements_to_score.push(new_node);
                        node = self.get_next_node(new_node, false);
                        continue;
                    } else if !self.has_child_block_element(n) {
                        self.set_node_tag(n, "p");
                        elements_to_score.push(n);
                    }
                }

                node = self.get_next_node(n, false);
            }

            // ── Scoring loop ──────────────────────────────────────────────
            let mut candidates: Vec<NodeId> = Vec::new();

            for &elem in &elements_to_score {
                if self.doc.parent(elem).is_none() {
                    continue;
                }
                let parent_tag = self
                    .doc
                    .parent(elem)
                    .map(|p| self.doc.tag_name(p).to_string())
                    .unwrap_or_default();
                if parent_tag.is_empty() {
                    continue;
                }

                let (num_chars, num_commas) =
                    crate::traverse::count_chars_and_commas(&self.doc, elem);
                if num_chars < 25 {
                    continue;
                }

                let ancestors = self.get_node_ancestors(elem, 5);
                if ancestors.is_empty() {
                    continue;
                }

                // Base score + commas + 1 + char bonus.
                let content_score = 1
                    + num_commas
                    + 1
                    + (((num_chars as f64) / 100.0).floor() as usize).min(3);

                for (level, &ancestor) in ancestors.iter().enumerate() {
                    let anc_tag = self.doc.tag_name(ancestor).to_string();
                    if anc_tag.is_empty() {
                        continue;
                    }
                    if self.doc.parent(ancestor).is_none() {
                        continue;
                    }
                    // Verify parent of ancestor is an element.
                    let anc_parent_is_elem = self
                        .doc
                        .parent(ancestor)
                        .map(|p| self.doc.is_element(p))
                        .unwrap_or(false);
                    if !anc_parent_is_elem {
                        continue;
                    }

                    if !self.has_content_score(ancestor) {
                        self.initialize_node(ancestor);
                        candidates.push(ancestor);
                    }

                    let score_divider: f64 = match level {
                        0 => 1.0,
                        1 => 2.0,
                        _ => (level as f64) * 3.0,
                    };

                    let ancestor_score = self.get_content_score(ancestor);
                    self.set_content_score(
                        ancestor,
                        ancestor_score + content_score as f64 / score_divider,
                    );
                }
            }

            // Scale scores by link density.
            for &candidate in &candidates {
                let score = self.get_content_score(candidate)
                    * (1.0 - self.get_link_density(candidate));
                self.set_content_score(candidate, score);
            }

            // Sort candidates descending by score.
            candidates.sort_by(|&a, &b| {
                self.get_content_score(b)
                    .partial_cmp(&self.get_content_score(a))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let top_candidates: Vec<NodeId> = candidates
                .iter()
                .copied()
                .take(self.n_top_candidates)
                .collect();

            // ── Top candidate selection ───────────────────────────────────
            let mut top_candidate: Option<NodeId> = top_candidates.first().copied();
            let mut needed_to_create_top_candidate = false;

            if top_candidate.is_none()
                || top_candidate
                    .map(|tc| self.doc.tag_name(tc) == "body")
                    .unwrap_or(false)
            {
                // Wrap all body children in a new div.
                let new_div = self.doc.create_element("div");
                needed_to_create_top_candidate = true;
                // Move all body children into new_div.
                loop {
                    let first = self
                        .doc
                        .html
                        .tree
                        .get(page)
                        .and_then(|n| n.first_child().map(|c| c.id()));
                    match first {
                        Some(child) => self.doc.append_child(new_div, child),
                        None => break,
                    }
                }
                self.doc.append_child(page, new_div);
                self.initialize_node(new_div);
                top_candidate = Some(new_div);
            } else {
                let tc = top_candidate.unwrap();
                let top_candidate_score = self.get_content_score(tc);

                // Check for alternative ancestors shared by multiple top candidates.
                let mut alternative_ancestors: Vec<Vec<NodeId>> = Vec::new();
                for &alt in top_candidates.iter().skip(1) {
                    if self.get_content_score(alt) / top_candidate_score >= 0.75 {
                        let ancs = self.get_node_ancestors(alt, 0);
                        alternative_ancestors.push(ancs);
                    }
                }

                const MINIMUM_TOP_CANDIDATES: usize = 3;
                if alternative_ancestors.len() >= MINIMUM_TOP_CANDIDATES {
                    let mut parent_of_tc = self.doc.parent(tc);
                    'walk_up: while let Some(pot) = parent_of_tc {
                        if self.doc.tag_name(pot) == "body" {
                            break;
                        }
                        let mut count = 0;
                        for anc_list in &alternative_ancestors {
                            if anc_list.contains(&pot) {
                                count += 1;
                            }
                            if count >= MINIMUM_TOP_CANDIDATES {
                                top_candidate = Some(pot);
                                break 'walk_up;
                            }
                        }
                        parent_of_tc = self.doc.parent(pot);
                    }
                }

                let tc = top_candidate.unwrap();
                if !self.has_content_score(tc) {
                    self.initialize_node(tc);
                }

                // Walk up the tree if score improves.
                let mut parent_of_tc = self.doc.parent(tc);
                let mut last_score = self.get_content_score(tc);
                let score_threshold = last_score / 3.0;
                while let Some(pot) = parent_of_tc {
                    if self.doc.tag_name(pot) == "body" {
                        break;
                    }
                    if !self.has_content_score(pot) {
                        parent_of_tc = self.doc.parent(pot);
                        continue;
                    }
                    let parent_score = self.get_content_score(pot);
                    if parent_score < score_threshold {
                        break;
                    }
                    if parent_score > last_score {
                        top_candidate = Some(pot);
                        break;
                    }
                    last_score = parent_score;
                    parent_of_tc = self.doc.parent(pot);
                }

                // If top candidate is the only child, use parent.
                let tc = top_candidate.unwrap();
                let mut parent_of_tc = self.doc.parent(tc);
                while let Some(pot) = parent_of_tc {
                    if self.doc.tag_name(pot) == "body" {
                        break;
                    }
                    if self.doc.children(pot).len() != 1 {
                        break;
                    }
                    top_candidate = Some(pot);
                    parent_of_tc = self.doc.parent(pot);
                }

                let tc = top_candidate.unwrap();
                if !self.has_content_score(tc) {
                    self.initialize_node(tc);
                }
            }

            let top_candidate = top_candidate.unwrap();

            // ── Sibling gathering ─────────────────────────────────────────
            let article_content = self.doc.create_element("div");
            let sibling_score_threshold =
                10.0_f64.max(self.get_content_score(top_candidate) * 0.2);
            let top_candidate_score = self.get_content_score(top_candidate);
            let top_candidate_class = self
                .doc
                .attr(top_candidate, "class")
                .unwrap_or("")
                .to_string();

            let parent_of_tc = match self.doc.parent(top_candidate) {
                Some(p) => p,
                None => {
                    // No parent — just wrap top_candidate alone.
                    self.doc.append_child(article_content, top_candidate);
                    self.prep_article(article_content);
                    return Some(article_content);
                }
            };

            let siblings = self.doc.children(parent_of_tc);
            for sibling in siblings {
                let mut append = false;

                if sibling == top_candidate {
                    append = true;
                } else {
                    let mut content_bonus = 0.0_f64;
                    let sib_class = self.doc.attr(sibling, "class").unwrap_or("").to_string();
                    if sib_class == top_candidate_class && !top_candidate_class.is_empty() {
                        content_bonus += top_candidate_score * 0.2;
                    }

                    if self.has_content_score(sibling)
                        && self.get_content_score(sibling) + content_bonus
                            >= sibling_score_threshold
                    {
                        append = true;
                    } else if self.doc.tag_name(sibling) == "p" {
                        let link_density = self.get_link_density(sibling);
                        let node_content = self.get_inner_text(sibling, true);
                        let node_length = crate::utils::char_count(&node_content);

                        append = (node_length > 80 && link_density < 0.25)
                            || (node_length < 80
                                && node_length > 0
                                && link_density == 0.0
                                && RX_SENTENCE_PERIOD.is_match(&node_content));
                    }
                }

                if append {
                    let sib_tag = self.doc.tag_name(sibling).to_string();
                    if !ALTER_TO_DIV_EXCEPTIONS.contains(&sib_tag.as_str()) {
                        self.doc.rename_tag(sibling, "div");
                    }
                    self.doc.append_child(article_content, sibling);
                }
            }

            // ── Prep and wrap ─────────────────────────────────────────────
            self.prep_article(article_content);

            if needed_to_create_top_candidate {
                // The fake div was already moved into article_content.
                // Find it (should be the first div child) and tag it.
                let first_child = self.doc.first_element_child(article_content);
                if let Some(fc) = first_child {
                    if self.doc.tag_name(fc) == "div" {
                        self.doc.set_attr(fc, "id", "readability-page-1");
                        self.doc.set_attr(fc, "class", "page");
                    }
                }
            } else {
                let page_div = self.doc.create_element("div");
                self.doc.set_attr(page_div, "id", "readability-page-1");
                self.doc.set_attr(page_div, "class", "page");
                // Move all children of article_content into page_div.
                loop {
                    let first = self
                        .doc
                        .html
                        .tree
                        .get(article_content)
                        .and_then(|n| n.first_child().map(|c| c.id()));
                    match first {
                        Some(child) => self.doc.append_child(page_div, child),
                        None => break,
                    }
                }
                self.doc.append_child(article_content, page_div);
            }

            // ── Length check and flag cycling ─────────────────────────────
            let (text_length, _) =
                crate::traverse::count_chars_and_commas(&self.doc, article_content);

            if text_length < self.char_thresholds {
                let doc_snap = self.doc.clone();
                self.attempts.push(ParseAttempt {
                    article_content,
                    doc_snapshot: doc_snap,
                    text_length,
                });

                if self.flags.strip_unlikelys {
                    self.flags.strip_unlikelys = false;
                } else if self.flags.use_weight_classes {
                    self.flags.use_weight_classes = false;
                } else if self.flags.clean_conditionally {
                    self.flags.clean_conditionally = false;
                } else {
                    // All flags exhausted — use the attempt with the most text.
                    self.attempts
                        .sort_by(|a, b| b.text_length.cmp(&a.text_length));
                    if self.attempts[0].text_length == 0 {
                        return None;
                    }
                    let best_content = self.attempts[0].article_content;
                    self.doc = self.attempts[0].doc_snapshot.clone();
                    return Some(best_content);
                }
                // Try next pass.
                continue;
            }

            return Some(article_content);
        }
    }
}

// ── Default ───────────────────────────────────────────────────────────────────

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

// ── Free helpers (not methods — may operate on a foreign Document) ────────────

// ── CondStats and walk_cond (used by clean_conditionally) ────────────────────

/// Accumulates statistics for `should_clean_conditionally`.
struct CondStats {
    chars: CharCounter,
    text_chars: CharCounter,
    list_chars: CharCounter,
    heading_chars: CharCounter,
    link_chars_weighted: f64,
    commas: usize,
    p_count: usize,
    img_count: usize,
    li_count: usize,
    input_count: usize,
    embed_count: usize,
    has_video_embed: bool,
    inner_text_single: String,
}

impl CondStats {
    fn new() -> Self {
        CondStats {
            chars: CharCounter::new(),
            text_chars: CharCounter::new(),
            list_chars: CharCounter::new(),
            heading_chars: CharCounter::new(),
            link_chars_weighted: 0.0,
            commas: 0,
            p_count: 0,
            img_count: 0,
            li_count: 0,
            input_count: 0,
            embed_count: 0,
            has_video_embed: false,
            inner_text_single: String::new(),
        }
    }
}

/// Recursive walker for `should_clean_conditionally`.
///
/// Mirrors Go's `walk` closure in `cleanConditionally`: accumulates char counts
/// per element class (text/list/heading) and per-`<a>` link counts.
///
/// `link_coeff` is non-zero only when we are inside an `<a>` element.
/// `link_acc` accumulates chars that belong to the current `<a>` (if any).
#[allow(clippy::too_many_arguments)]
fn walk_cond(
    doc: &Document,
    n: NodeId,
    stats: &mut CondStats,
    in_text: bool,
    in_list: bool,
    in_heading: bool,
    link_coeff: f64,
    link_acc: &mut CharCounter,
    is_video_fn: &dyn Fn(NodeId) -> bool,
) {
    match doc.html.tree.get(n).map(|x| x.value()) {
        Some(Node::Text(text)) => {
            let old_total = stats.chars.total();
            for r in text.text.chars() {
                stats.chars.count(r);
                if is_comma(r) {
                    stats.commas += 1;
                }
                if in_text {
                    stats.text_chars.count(r);
                }
                if in_list {
                    stats.list_chars.count(r);
                }
                if in_heading {
                    stats.heading_chars.count(r);
                }
                if link_coeff != 0.0 {
                    link_acc.count(r);
                }
            }
            if stats.chars.total() > old_total {
                stats.inner_text_single = text.text.to_string();
            }
        }
        Some(Node::Element(_)) => {
            let tag = doc.tag_name(n);
            match tag {
                "p" => stats.p_count += 1,
                "img" => stats.img_count += 1,
                "li" => stats.li_count += 1,
                "input" => stats.input_count += 1,
                "object" | "embed" | "iframe" => {
                    stats.embed_count += 1;
                    if is_video_fn(n) {
                        stats.has_video_embed = true;
                    }
                }
                _ => {}
            }

            let new_in_list = in_list || matches!(tag, "ul" | "ol");
            if !in_list && matches!(tag, "ul" | "ol") {
                stats.list_chars.reset_context();
            }

            let new_in_heading =
                in_heading || matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
            if !in_heading && matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                stats.heading_chars.reset_context();
            }

            let new_in_text = in_text
                || matches!(
                    tag,
                    "blockquote"
                        | "dl"
                        | "div"
                        | "img"
                        | "ol"
                        | "p"
                        | "pre"
                        | "table"
                        | "ul"
                        | "span"
                        | "li"
                        | "td"
                );
            if !in_text
                && matches!(
                    tag,
                    "blockquote"
                        | "dl"
                        | "div"
                        | "img"
                        | "ol"
                        | "p"
                        | "pre"
                        | "table"
                        | "ul"
                        | "span"
                        | "li"
                        | "td"
                )
            {
                stats.text_chars.reset_context();
            }

            if tag == "a" {
                // Each <a> gets its own fresh link counter (port of Go's per-a cc).
                // Mirror Go: coefficient is 0 when the <a>'s DIRECT parent is a figcaption.
                let parent_tag = doc.parent(n).map(|p| doc.tag_name(p)).unwrap_or("");
                let coeff = if parent_tag != "figcaption" {
                    let href = doc.attr(n, "href").unwrap_or("").trim().to_string();
                    if href.len() > 1 && href.starts_with('#') { 0.3 } else { 1.0 }
                } else {
                    0.0
                };
                let mut my_acc = CharCounter::new();
                for child in doc.child_nodes(n) {
                    walk_cond(
                        doc, child, stats, new_in_text, new_in_list, new_in_heading,
                        coeff, &mut my_acc, is_video_fn,
                    );
                }
                stats.link_chars_weighted += my_acc.total() as f64 * coeff;
            } else {
                for child in doc.child_nodes(n) {
                    walk_cond(
                        doc, child, stats, new_in_text, new_in_list, new_in_heading,
                        link_coeff, link_acc, is_video_fn,
                    );
                }
            }
        }
        _ => {}
    }
}

/// Scan direct children (and their descendants) for data-table structural signals.
///
/// Returns `(is_data_table, is_conclusive)`.
fn scan_for_data_table_signals(doc: &Document, n: NodeId) -> (bool, bool) {
    let Some(node) = doc.html.tree.get(n) else {
        return (false, false);
    };
    for child in node.children() {
        if let Node::Element(el) = child.value() {
            match el.name() {
                "col" | "colgroup" | "tfoot" | "thead" | "th" => {
                    return (true, true);
                }
                "caption" => {
                    if child.has_children() {
                        return (true, true);
                    }
                }
                "table" => {
                    return (false, true);
                }
                _ => {}
            }
        }
        let (result, conclusive) = scan_for_data_table_signals(doc, child.id());
        if conclusive {
            return (result, conclusive);
        }
    }
    (false, false)
}

/// Return the last child of any type (text, element, comment…).
fn last_child_node(doc: &Document, id: NodeId) -> Option<NodeId> {
    doc.html.tree.get(id)?.last_child().map(|n| n.id())
}

/// Deep-clone a node into the same tree and return the new NodeId.
///
/// Handles element nodes (recursively cloning children) and text nodes.
fn clone_node(doc: &mut Document, id: NodeId) -> NodeId {
    match doc.html.tree.get(id).map(|n| n.value().clone()) {
        Some(Node::Element(el)) => {
            let new_name = el.name.clone();
            let attrs: Vec<_> = el
                .attrs
                .iter()
                .map(|(k, v)| html5ever::Attribute {
                    name: k.clone(),
                    value: v.clone(),
                })
                .collect();
            let new_el =
                scraper::node::Element::new(new_name, attrs);
            let new_id = doc.html.tree.orphan(Node::Element(new_el)).id();
            // Clone children.
            let children: Vec<NodeId> = doc.child_nodes(id);
            for child in children {
                let child_clone = clone_node(doc, child);
                doc.append_child(new_id, child_clone);
            }
            new_id
        }
        Some(Node::Text(t)) => {
            let text_val = t.text.as_ref().to_string();
            doc.create_text_node(&text_val)
        }
        _ => {
            // For other node types (comments, etc.), create an empty text node as a fallback.
            doc.create_text_node("")
        }
    }
}

/// True if this node or any descendant has useful content (images, embeds, or text).
///
/// Port of the inline `findContent` closure in Go's `prepArticle`.
fn find_content_in_node(doc: &Document, id: NodeId) -> bool {
    let Some(node) = doc.html.tree.get(id) else {
        return false;
    };
    match node.value() {
        Node::Element(el) => {
            match el.name() {
                "img" | "picture" | "embed" | "object" | "iframe" => return true,
                _ => {}
            }
        }
        Node::Text(_) => {
            if has_text_content(doc, id) {
                return true;
            }
        }
        _ => {}
    }
    for child in node.children() {
        if find_content_in_node(doc, child.id()) {
            return true;
        }
    }
    false
}

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

    #[test]
    fn grab_article_extracts_content() {
        // A realistic article with enough text to exceed the 500-char threshold.
        let body = "The quick brown fox jumps over the lazy dog. ".repeat(15);
        let html = format!(
            r#"<html><head><title>Test</title></head>
            <body>
              <nav class="menu">Navigation links here and there and everywhere</nav>
              <article>
                <h1>Article Heading</h1>
                <p>{body}</p>
                <p>{body}</p>
              </article>
              <footer>Footer content</footer>
            </body></html>"#,
        );
        let mut p = make_parser();
        let article = p.parse(&html, None).unwrap();
        assert!(article.length > 0, "grab_article should produce content, got length=0");
        assert!(article.content.contains("quick brown fox"), "content should include article text");
    }
}
