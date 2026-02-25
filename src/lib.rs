// Port of go-readability/readability.go

pub mod article;
pub mod dom;
pub mod error;
pub mod inspect;
pub mod parser;
pub mod regexp;
pub mod render;
pub mod traverse;
pub mod utils;

pub use article::Article;
