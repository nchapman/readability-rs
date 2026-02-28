uniffi::setup_scaffolding!();

/// Errors returned by extraction functions.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ReadabilityError {
    #[error("{reason}")]
    Parse { reason: String },
    #[error("document is not readable")]
    NotReadable,
    #[error("no article content found")]
    NoContent,
}

impl From<libreadability::Error> for ReadabilityError {
    fn from(e: libreadability::Error) -> Self {
        match e {
            libreadability::Error::Parse(msg) => ReadabilityError::Parse { reason: msg },
            libreadability::Error::NotReadable => ReadabilityError::NotReadable,
            libreadability::Error::NoContent => ReadabilityError::NoContent,
            // non_exhaustive — future-proof
            _ => ReadabilityError::Parse {
                reason: e.to_string(),
            },
        }
    }
}

/// Extracted article content and metadata.
#[derive(uniffi::Record)]
pub struct Article {
    pub title: String,
    pub byline: String,
    pub excerpt: String,
    pub site_name: String,
    pub image: String,
    pub favicon: String,
    pub language: String,
    pub published_time: String,
    pub modified_time: String,
    /// Cleaned article HTML.
    pub content: String,
    /// Plain text via InnerText algorithm.
    pub text_content: String,
    /// Character count of text_content.
    pub length: u64,
    /// Text direction: "ltr", "rtl", or "".
    pub dir: String,
}

/// Parser configuration options.
#[derive(uniffi::Record)]
pub struct ParserConfig {
    /// Max DOM nodes to process. 0 = unlimited.
    pub max_elems_to_parse: u64,
    /// Number of top candidates to compare during scoring.
    pub n_top_candidates: u64,
    /// Minimum character count for accepted article content.
    pub char_threshold: u64,
    /// CSS class names to preserve.
    pub classes_to_preserve: Vec<String>,
    /// If true, keep all class attributes.
    pub keep_classes: bool,
    /// Tag names eligible for content scoring.
    pub tags_to_score: Vec<String>,
    /// Disable JSON-LD metadata extraction.
    pub disable_jsonld: bool,
}

/// Returns the default parser configuration.
#[uniffi::export]
pub fn default_parser_config() -> ParserConfig {
    let p = libreadability::Parser::new();
    ParserConfig {
        max_elems_to_parse: p.max_elems_to_parse as u64,
        n_top_candidates: p.n_top_candidates as u64,
        char_threshold: p.char_threshold as u64,
        classes_to_preserve: p.classes_to_preserve,
        keep_classes: p.keep_classes,
        tags_to_score: p.tags_to_score,
        disable_jsonld: p.disable_jsonld,
    }
}

/// Extract the main article content from HTML.
///
/// `url` is an optional page URL used to resolve relative links.
#[uniffi::export]
pub fn extract(html: String, url: Option<String>) -> Result<Article, ReadabilityError> {
    let article = libreadability::extract(&html, url.as_deref())?;
    Ok(to_ffi_article(article))
}

/// Extract the main article content from HTML with custom parser configuration.
#[uniffi::export]
pub fn extract_with(
    html: String,
    url: Option<String>,
    config: ParserConfig,
) -> Result<Article, ReadabilityError> {
    let page_url = match &url {
        Some(u) => Some(
            url::Url::parse(u)
                .map_err(|e| ReadabilityError::Parse {
                    reason: format!("invalid URL '{u}': {e}"),
                })?,
        ),
        None => None,
    };
    let mut parser = to_core_parser(&config);
    let article = parser.parse(&html, page_url.as_ref())?;
    Ok(to_ffi_article(article))
}

/// Check whether an HTML document is likely a readable article.
#[uniffi::export]
pub fn check_html(html: String) -> bool {
    libreadability::Parser::new().check_html(&html)
}

// --- Internal conversion helpers ---

fn to_ffi_article(a: libreadability::Article) -> Article {
    Article {
        title: a.title,
        byline: a.byline,
        excerpt: a.excerpt,
        site_name: a.site_name,
        image: a.image,
        favicon: a.favicon,
        language: a.language,
        published_time: a.published_time,
        modified_time: a.modified_time,
        content: a.content,
        text_content: a.text_content,
        length: a.length as u64,
        dir: a.dir,
    }
}

fn to_core_parser(c: &ParserConfig) -> libreadability::Parser {
    let cap = usize::MAX as u64;
    libreadability::Parser::new()
        .with_max_elems_to_parse(c.max_elems_to_parse.min(cap) as usize)
        .with_n_top_candidates(c.n_top_candidates.min(cap) as usize)
        .with_char_threshold(c.char_threshold.min(cap) as usize)
        .with_classes_to_preserve(c.classes_to_preserve.clone())
        .with_keep_classes(c.keep_classes)
        .with_tags_to_score(c.tags_to_score.clone())
        .with_disable_jsonld(c.disable_jsonld)
}
