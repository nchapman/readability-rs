// Error types for readability-rs

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("HTML parse error: {0}")]
    Parse(String),
    #[error("document is not readable")]
    NotReadable,
    #[error("no article content found")]
    NoContent,
}
