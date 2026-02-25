// Port of go-readability/article.go

/// The extracted article content and metadata.
///
/// All fields are eagerly serialized to strings at construction time (unlike the Go
/// version which holds a `*html.Node` and lazily renders). The `excerpt` fallback
/// (inner text of the first `<p>` in the article) is applied at construction.
///
/// Port of `Article`.
#[derive(Debug, Clone, Default)]
pub struct Article {
    pub title: String,
    pub byline: String,
    /// Excerpt from metadata, or inner text of the first `<p>` in the article.
    pub excerpt: String,
    pub site_name: String,
    pub image: String,
    pub favicon: String,
    pub language: String,
    pub published_time: String,
    pub modified_time: String,
    /// Cleaned article HTML (outer_html of the article container node).
    pub content: String,
    /// Plain text via InnerText algorithm (port of render.InnerText).
    pub text_content: String,
    /// Character count of text_content.
    pub length: usize,
    /// Direction: "ltr", "rtl", or "".
    pub dir: String,
}
