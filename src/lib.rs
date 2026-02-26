// Port of go-readability/readability.go

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
