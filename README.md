# libreadability

[![Crates.io](https://img.shields.io/crates/v/libreadability.svg)](https://crates.io/crates/libreadability)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust: 1.80+](https://img.shields.io/badge/rust-1.80%2B-orange.svg)](https://www.rust-lang.org)

Extract the main article content from web pages.

A Rust port of [readability by readeck](https://codeberg.org/readeck/readability),
which is a Go port of Mozilla's [Readability.js](https://github.com/mozilla/readability).

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
libreadability = "0.1"
```

```rust
use libreadability::Parser;

let html = include_str!("article.html");

let mut parser = Parser::new();
let article = parser.parse(html, None).unwrap();

println!("Title: {}", article.title);
println!("Author: {}", article.byline);
println!("Text: {}", article.text_content);
println!("HTML: {}", article.content);
```

## What it returns

The `Article` struct contains:

| Field | Description |
|-------|-------------|
| `title` | Article title |
| `byline` | Author attribution |
| `excerpt` | Short description or first paragraph |
| `content` | Cleaned article HTML |
| `text_content` | Plain text (via InnerText algorithm) |
| `length` | Character count of text content |
| `site_name` | Publisher name |
| `image` | Lead image URL |
| `language` | Detected language |
| `published_time` | Publication timestamp |
| `modified_time` | Last modified timestamp |
| `dir` | Text direction (`ltr` or `rtl`) |

## Configuration

Configure via public fields or chainable builder methods:

```rust
use libreadability::Parser;

// Builder style
let mut parser = Parser::new()
    .with_char_threshold(200)
    .with_keep_classes(true)
    .with_disable_jsonld(true);

// Or set fields directly
let mut parser = Parser::new();
parser.char_threshold = 200;
parser.keep_classes = true;
```

## Optional features

| Feature | Description |
|---------|-------------|
| `tracing` | Enable debug/trace logging at key algorithm points (zero-cost when disabled) |

```toml
libreadability = { version = "0.1", features = ["tracing"] }
```

## License

MIT
