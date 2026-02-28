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
libreadability = "0.2"
```

```rust
use libreadability::extract;

let html = r#"<html><body>
  <article>
    <h1>Breaking News</h1>
    <p>This is the main article body with enough text to be extracted.</p>
    <p>The readability algorithm scores content density and identifies the
    primary article content, stripping navigation, ads, and other boilerplate.</p>
  </article>
</body></html>"#;

let article = extract(html, None).unwrap();

println!("Title: {}", article.title);
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
libreadability = { version = "0.2", features = ["tracing"] }
```

## Benchmarks

Rust vs Go ([readeck/readability](https://codeberg.org/readeck/readability)) on representative pages:

| Page | Go | Rust | Speedup |
|------|---:|-----:|--------:|
| ars-1 (~56 KB) | 1.85 ms | 854 µs | 2.2x |
| wapo-1 (~180 KB) | 10.1 ms | 2.46 ms | 4.1x |
| wikipedia (~244 KB) | 18.3 ms | 6.79 ms | 2.7x |
| nytimes-3 (~489 KB) | 19.5 ms | 3.35 ms | 5.8x |
| yahoo-2 (~1.6 MB) | 35.8 ms | 7.11 ms | 5.0x |
| **all 133 fixtures** | **670 ms** | **320 ms** | **2.1x** |

Measured on Apple M4 Max, Rust 1.93, Go 1.25, macOS 15.7.

Reproduce:

```sh
cargo bench                  # Criterion benchmarks
./benches/compare.sh         # Rust vs Go comparison table
```

## License

MIT
