// Port of go-readability/readability.go

pub mod article;
pub(crate) mod dom;
pub mod error;
pub mod parser;
pub(crate) mod regexp;
pub(crate) mod render;
pub(crate) mod traverse;
pub(crate) mod utils;

pub use article::Article;
pub use error::Error;
pub use parser::Parser;
