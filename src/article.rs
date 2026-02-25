// Port of go-readability/article.go

/// The extracted article content and metadata.
///
/// Port of `Article`.
#[derive(Debug, Clone, Default)]
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
    /// Plain text content (port of TextContent via render.InnerText)
    pub text_content: String,
    /// Cleaned HTML content
    pub content: String,
}
