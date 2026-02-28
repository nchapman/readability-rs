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
//! use libreadability::Parser;
//!
//! let html = r#"<html><body>
//!   <nav>Navigation links</nav>
//!   <article><p>This is the main article body with enough text to be extracted.</p>
//!   <p>The readability algorithm scores content density and identifies the
//!   primary article content, stripping navigation, ads, and other boilerplate.</p></article>
//!   <aside>Sidebar content</aside>
//! </body></html>"#;
//!
//! let mut parser = Parser::new();
//! let article = parser.parse(html, None).expect("valid HTML");
//! assert!(!article.content.is_empty());
//! assert!(!article.text_content.is_empty());
//! ```
//!
//! # Output
//!
//! [`Article`] contains both cleaned HTML ([`content`](Article::content)) and
//! plain text ([`text_content`](Article::text_content)), plus metadata like
//! title, byline, excerpt, published time, and text direction.

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
