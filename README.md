# readability

Extract the main article content from web pages.

A Rust port of [readability by readeck](https://codeberg.org/readeck/readability),
which is a Go port of Mozilla's [Readability.js](https://github.com/mozilla/readability).

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
readability = "0.1"
```

```rust
use readability::Parser;

let html = include_str!("article.html");

let mut parser = Parser::new();
let article = parser.parse(html, None).unwrap();

println!("Title: {}", article.title);
println!("Author: {}", article.byline);
println!("Text: {}", article.text_content);
println!("HTML: {}", article.content);
```

## What it returns

The [`Article`] struct contains:

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
| `dir` | Text direction (`ltr` or `rtl`) |

## Configuration

```rust
use readability::Parser;

let mut parser = Parser::new();
parser.char_threshold = 200;       // minimum article length
parser.keep_classes = true;        // preserve CSS classes
parser.disable_jsonld = true;      // skip JSON-LD metadata
```

## License

Apache-2.0
