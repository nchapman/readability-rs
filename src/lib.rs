// Port of go-readability/readability.go

//! Readability article extraction library.
//!
//! `libreadability` extracts the main article content from web pages by analyzing
//! DOM structure, scoring content density, and removing boilerplate. It is a
//! Rust port of [readability by readeck](https://codeberg.org/readeck/readability),
//! itself a Go port of Mozilla's Readability.js.
//!
//! # Quick start
//!
//! ```rust
//! use libreadability::extract;
//!
//! let html = r#"<html><body>
//!   <nav>Navigation links</nav>
//!   <article><p>This is the main article body with enough text to be extracted.</p>
//!   <p>The readability algorithm scores content density and identifies the
//!   primary article content, stripping navigation, ads, and other boilerplate.</p></article>
//!   <aside>Sidebar content</aside>
//! </body></html>"#;
//!
//! let article = extract(html, None).expect("valid HTML");
//! assert!(!article.content.is_empty());
//! assert!(!article.text_content.is_empty());
//! ```
//!
//! # Output
//!
//! [`Article`] contains both cleaned HTML ([`content`](Article::content)) and
//! plain text ([`text_content`](Article::text_content)), plus metadata like
//! title, byline, excerpt, published time, and text direction.
//!
//! # Configuration
//!
//! For fine-grained control, use [`Parser`] directly:
//!
//! ```rust
//! use libreadability::Parser;
//!
//! let mut parser = Parser::new()
//!     .with_char_threshold(200)
//!     .with_keep_classes(true);
//! let article = parser.parse("<html><body><article><p>Content</p></article></body></html>", None);
//! ```
//!
//! # Related crates
//!
//! - [`trafilatura`](https://crates.io/crates/trafilatura) — full-featured web
//!   content extraction with metadata, comments, and fallback strategies.
//! - [`justext`](https://crates.io/crates/justext) — paragraph-level boilerplate
//!   removal using stopword density.
//! - [`html2markdown`](https://crates.io/crates/html2markdown) — converts HTML to
//!   Markdown via an intermediate AST.

pub(crate) mod article;
pub(crate) mod dom;
pub(crate) mod error;
pub(crate) mod parser;
pub(crate) mod regexp;
pub(crate) mod render;
pub(crate) mod traverse;
pub(crate) mod utils;

pub use article::Article;
pub use error::Error;
pub use parser::Parser;

/// Parse HTML and extract the main article content in one call.
///
/// This is a convenience wrapper around [`Parser::new`] + [`Parser::parse`].
/// For repeated use or fine-grained configuration, create a [`Parser`] directly.
///
/// `url` is an optional page URL used to resolve relative links (e.g.
/// `"https://example.com/article"`). Pass `None` if the HTML is self-contained
/// or you don't need absolute URLs.
///
/// # Example
///
/// ```rust
/// let article = libreadability::extract(
///     "<html><body><article><p>Hello world</p></article></body></html>",
///     None,
/// ).unwrap();
/// assert_eq!(article.text_content, "Hello world");
/// ```
pub fn extract(html: &str, url: Option<&str>) -> Result<Article, Error> {
    let page_url = match url {
        Some(u) => {
            Some(url::Url::parse(u).map_err(|e| Error::Parse(format!("invalid URL '{u}': {e}")))?)
        }
        None => None,
    };
    Parser::new().parse(html, page_url.as_ref())
}
