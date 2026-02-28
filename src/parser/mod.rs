// Port of go-readability/parser.go + parser-parse.go
//
// Split into submodules for maintainability:
//   parse.rs    — public API (parse, check_html, check_document, parse_and_mutate)
//   helpers.rs  — node iteration/traversal helpers + free functions
//   prep.rs     — document preparation (comments, scripts, <br> chains, images)
//   metadata.rs — title, JSON-LD, favicon, metadata extraction
//   scoring.rs  — content scoring + grab_article
//   clean.rs    — post-processing, cleaning, prep_article

mod clean;
mod helpers;
mod metadata;
mod parse;
mod prep;
mod scoring;

use std::collections::{HashMap, HashSet};

use ego_tree::NodeId;
use url::Url;

use crate::article::Article;
use crate::dom::Document;
use crate::error::Error;

pub(crate) type Result<T = Article> = std::result::Result<T, Error>;

// ── Constants ─────────────────────────────────────────────────────────────────

pub(super) const DIV_TO_P_ELEMS: &[&str] = &[
    "blockquote",
    "dl",
    "div",
    "img",
    "ol",
    "p",
    "pre",
    "table",
    "ul",
    "select",
];

pub(super) const ALTER_TO_DIV_EXCEPTIONS: &[&str] = &["div", "article", "section", "p", "ol", "ul"];

pub(super) const PHRASING_ELEMS: &[&str] = &[
    "abbr", "audio", "b", "bdo", "br", "button", "cite", "code", "data", "datalist", "dfn", "em",
    "embed", "i", "img", "input", "kbd", "label", "mark", "math", "meter", "noscript", "object",
    "output", "progress", "q", "ruby", "samp", "script", "select", "small", "span", "strong",
    "sub", "sup", "textarea", "time", "var", "wbr",
];

pub(super) const UNLIKELY_ROLES: &[&str] = &[
    "menu",
    "menubar",
    "complementary",
    "navigation",
    "alert",
    "alertdialog",
    "dialog",
];

pub(super) const PRESENTATIONAL_ATTRS: &[&str] = &[
    "align",
    "background",
    "bgcolor",
    "border",
    "cellpadding",
    "cellspacing",
    "frame",
    "hspace",
    "rules",
    "style",
    "valign",
    "vspace",
];

pub(super) const DEPRECATED_SIZE_ATTR_ELEMS: &[&str] = &["table", "th", "td", "hr", "pre"];

/// Maximum recursion depth for tree-walking helpers to avoid stack overflow on
/// pathologically nested documents.
pub(super) const MAX_TREE_DEPTH: usize = 200;

// ── Internal structs ──────────────────────────────────────────────────────────

/// Port of `flags` — controls which phases of the algorithm are active.
#[derive(Clone, Debug)]
pub(super) struct Flags {
    pub(super) strip_unlikelys: bool,
    pub(super) use_weight_classes: bool,
    pub(super) clean_conditionally: bool,
}

/// Saved state from a failed grab_article pass.
pub(super) struct ParseAttempt {
    pub(super) article_content: NodeId,
    pub(super) doc_snapshot: Document,
    pub(super) text_length: usize,
}

impl Default for Flags {
    fn default() -> Self {
        Flags {
            strip_unlikelys: true,
            use_weight_classes: true,
            clean_conditionally: true,
        }
    }
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Port of `Parser` — the core readability extraction engine.
///
/// Create with `Parser::new()`, configure public fields as needed, then call
/// `parse()` to extract an article.
///
/// A single `Parser` can be reused for multiple documents — internal state is
/// fully reset at the start of each parse call. However, `Parser` is **not
/// thread-safe**: it requires `&mut self` for parsing, so it cannot be shared
/// across threads without external synchronization.
#[non_exhaustive]
pub struct Parser {
    // ── Public configuration ──────────────────────────────────────────────
    /// Max DOM nodes to process. 0 = unlimited. Port of `MaxElemsToParse`.
    pub max_elems_to_parse: usize,
    /// Number of top candidates to compare during scoring. Port of `NTopCandidates`.
    pub n_top_candidates: usize,
    /// Minimum character count for accepted article content. Port of `CharThreshold`.
    pub char_threshold: usize,
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
    pub(super) doc: Document,
    pub(super) document_uri: Option<Url>,
    pub(super) article_title: String,
    pub(super) article_byline: String,
    pub(super) article_dir: String,
    pub(super) article_lang: String,
    pub(super) flags: Flags,

    // ── Per-pass side tables (reset at start of each grab_article pass) ──
    pub(super) score_map: HashMap<NodeId, f64>,
    pub(super) data_tables: HashSet<NodeId>,
    pub(super) attempts: Vec<ParseAttempt>,
}

impl Parser {
    /// Port of `NewParser` — construct a parser with default settings.
    pub fn new() -> Self {
        Parser {
            max_elems_to_parse: 0,
            n_top_candidates: 5,
            char_threshold: 500,
            classes_to_preserve: vec!["page".to_string()],
            keep_classes: false,
            tags_to_score: vec![
                "section".to_string(),
                "h2".to_string(),
                "h3".to_string(),
                "h4".to_string(),
                "h5".to_string(),
                "h6".to_string(),
                "p".to_string(),
                "td".to_string(),
                "pre".to_string(),
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

    // ── Chainable builder methods ──────────────────────────────────────────

    /// Set the maximum number of DOM elements to parse. 0 = unlimited.
    pub fn with_max_elems_to_parse(mut self, n: usize) -> Self {
        self.max_elems_to_parse = n;
        self
    }

    /// Set the number of top candidates to compare during scoring.
    pub fn with_n_top_candidates(mut self, n: usize) -> Self {
        self.n_top_candidates = n;
        self
    }

    /// Set the minimum character count for accepted article content.
    pub fn with_char_threshold(mut self, n: usize) -> Self {
        self.char_threshold = n;
        self
    }

    /// Set CSS class names to preserve when `keep_classes` is false.
    pub fn with_classes_to_preserve(
        mut self,
        classes: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.classes_to_preserve = classes.into_iter().map(Into::into).collect();
        self
    }

    /// If true, keep all class attributes on extracted content.
    pub fn with_keep_classes(mut self, keep: bool) -> Self {
        self.keep_classes = keep;
        self
    }

    /// Set tag names eligible for content scoring.
    pub fn with_tags_to_score(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.tags_to_score = tags.into_iter().map(Into::into).collect();
        self
    }

    /// Disable JSON-LD metadata extraction.
    pub fn with_disable_jsonld(mut self, disable: bool) -> Self {
        self.disable_jsonld = disable;
        self
    }

    /// Set a regex for video URLs to allow during cleaning.
    pub fn with_allowed_video_regex(mut self, re: regex::Regex) -> Self {
        self.allowed_video_regex = Some(re);
        self
    }
}

// ── Default ───────────────────────────────────────────────────────────────────

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
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
        assert_eq!(metadata::html_unescape("foo &amp; bar"), "foo & bar");
        assert_eq!(metadata::html_unescape("&lt;p&gt;"), "<p>");
        assert_eq!(
            metadata::html_unescape("he said &quot;hi&quot;"),
            "he said \"hi\""
        );
    }

    #[test]
    fn html_unescape_numeric_entities() {
        assert_eq!(metadata::html_unescape("&#65;"), "A"); // decimal
        assert_eq!(metadata::html_unescape("&#x41;"), "A"); // hex
    }

    #[test]
    fn html_unescape_unknown_entity_preserved() {
        assert_eq!(metadata::html_unescape("&unknown;"), "&unknown;");
    }

    #[test]
    fn html_unescape_no_ampersand_passthrough() {
        let s = "no entities here";
        assert_eq!(metadata::html_unescape(s), s);
    }

    // ── get_article_title ──────────────────────────────────────────────────

    #[test]
    fn get_article_title_simple() {
        let mut p = make_parser();
        let doc =
            Document::parse("<html><head><title>Hello World</title></head><body></body></html>");
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
        assert!(
            !html.contains("hidden"),
            "comment should be removed: {html}"
        );
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
        p.doc =
            Document::parse(r#"<html><body><div class="foo page bar">text</div></body></html>"#);
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
        let a = p
            .doc
            .get_elements_by_tag_name(body, "a")
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(p.doc.attr(a, "href"), Some("https://example.com/page"));
    }

    #[test]
    fn fix_relative_uris_removes_javascript_links() {
        let base = Url::parse("https://example.com/").unwrap();
        let mut p = make_parser();
        p.document_uri = Some(base);
        p.doc =
            Document::parse(r#"<html><body><a href="javascript:void(0)">click</a></body></html>"#);
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
        assert!(
            article.length > 0,
            "grab_article should produce content, got length=0"
        );
        assert!(
            article.content.contains("quick brown fox"),
            "content should include article text"
        );
    }
}
