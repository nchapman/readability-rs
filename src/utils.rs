// Port of go-readability/utils.go

use url::Url;

use crate::regexp::RX_TOKENIZE;

/// Port of indexOf — returns the index of the first occurrence of `target` in `slice`,
/// or `None` if not found.
pub fn index_of(slice: &[&str], target: &str) -> Option<usize> {
    slice.iter().position(|&s| s == target)
}

/// Port of wordCount — splits on whitespace and counts words.
pub fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Port of charCount — simple Unicode codepoint count (NOT the whitespace-normalizing
/// version used in traverse.rs). Used for byline length checks, title comparisons, etc.
pub fn char_count(s: &str) -> usize {
    s.chars().count()
}

/// Port of hasContent — true if the string contains any non-whitespace character.
pub fn has_content(s: &str) -> bool {
    s.chars().any(|c| !c.is_whitespace())
}

/// Port of isValidURL — true if the string is a parseable absolute URI with a scheme.
///
/// Go's `url.ParseRequestURI` rejects relative paths (no scheme) but accepts any URI
/// with a valid scheme, including those without a host (e.g. `file:///path`). We match
/// that by requiring only that `Url::parse` succeeds (which also rejects relative paths).
pub fn is_valid_url(s: &str) -> bool {
    Url::parse(s).is_ok()
}

/// Port of toAbsoluteURI — resolve `uri` against `base` into an absolute URL string.
///
/// Special cases (unchanged):
/// - Empty string or hash (`#…`) — returned as-is
/// - `data:` URI — returned as-is
/// - Protocol-relative (`//…`) — base scheme prepended
/// - Already absolute http(s) — returned as-is
/// - Other parseable absolute URIs — returned as-is
/// - Relative paths — resolved against base
pub fn to_absolute_uri(uri: &str, base: &Url) -> String {
    if uri.is_empty() {
        return uri.to_string();
    }

    // Hash anchors stay as-is.
    if uri.starts_with('#') {
        return uri.to_string();
    }

    // data: URIs stay as-is.
    if uri.starts_with("data:") {
        return uri.to_string();
    }

    // Protocol-relative: prepend base scheme.
    if uri.starts_with("//") {
        return format!("{}:{}", base.scheme(), uri);
    }

    // Already absolute http/https.
    if uri.starts_with("https://") || uri.starts_with("http://") {
        return uri.to_string();
    }

    // Any other parseable absolute URI with a non-empty scheme and host.
    if let Ok(parsed) = Url::parse(uri) {
        if !parsed.scheme().is_empty() && parsed.host().is_some() {
            return uri.to_string();
        }
    }

    // Resolve relative path against base.
    // Percent-encode characters that the `url` crate would silently strip or
    // reject (e.g. space → `%20`, `|` → `%7C`), matching Go's url.Parse
    // behaviour which preserves them and lets ResolveReference encode them.
    let needs_encoding = uri.contains(' ') || uri.contains('|');
    let encoded: std::borrow::Cow<str> = if needs_encoding {
        uri.replace(' ', "%20").replace('|', "%7C").into()
    } else {
        uri.into()
    };
    match base.join(&encoded) {
        Ok(resolved) => resolved.to_string(),
        Err(_) => uri.to_string(),
    }
}

/// Port of strOr — returns the first non-empty string from the slice.
pub fn str_or<'a>(candidates: &[&'a str]) -> &'a str {
    candidates.iter().copied().find(|s| !s.is_empty()).unwrap_or("")
}

/// Port of textSimilarity — character-count-based similarity in `[0.0, 1.0]`.
///
/// Lowercases both inputs (matching Go's `strings.ToLower`), tokenizes using
/// `rxTokenize`, then computes `1 - charCount(unique_B) / charCount(B)` where
/// `unique_B` is the tokens in B that do not appear in A. Returns `0.0` when B
/// is empty.
///
/// Go's algorithm differs from Jaccard: it is character-count-based so that long
/// shared words contribute more than short ones. Two tokens joined with a space
/// count the join space, mirroring `strings.Join(tokens, " ")`.
pub fn text_similarity(a: &str, b: &str) -> f64 {
    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();

    let tokens_a: std::collections::HashSet<&str> = RX_TOKENIZE
        .split(&a_lower)
        .filter(|s| !s.is_empty())
        .collect();
    let tokens_b: Vec<&str> = RX_TOKENIZE
        .split(&b_lower)
        .filter(|s| !s.is_empty())
        .collect();

    // charCount of strings.Join(tokens_b, " ")
    let merged_b_len: usize = tokens_b.iter().map(|t| t.chars().count()).sum::<usize>()
        + tokens_b.len().saturating_sub(1);

    if merged_b_len == 0 {
        return 0.0;
    }

    // tokens in B that are NOT in A
    let unique_b: Vec<&str> = tokens_b
        .iter()
        .copied()
        .filter(|t| !tokens_a.contains(t))
        .collect();

    // charCount of strings.Join(unique_b, " ")
    let unique_b_len: usize = unique_b.iter().map(|t| t.chars().count()).sum::<usize>()
        + unique_b.len().saturating_sub(1);

    1.0 - unique_b_len as f64 / merged_b_len as f64
}

// ────────────────────────────────────────────────────────────────────────────
// Tests (port of utils_test.go)
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_of_finds_first_occurrence() {
        let sample: Vec<&str> = "hello this is a simple sentence and we try to repeat some simple word like this"
            .split_whitespace()
            .collect();
        assert_eq!(index_of(&sample, "hello"), Some(0));
        assert_eq!(index_of(&sample, "this"), Some(1));
        assert_eq!(index_of(&sample, "simple"), Some(4));
        assert_eq!(index_of(&sample, "we"), Some(7));
        assert_eq!(index_of(&sample, "repeat"), Some(10));
        assert_eq!(index_of(&sample, "notfound"), None);
    }

    #[test]
    fn word_count_counts_words() {
        assert_eq!(
            word_count("German fashion designer Karl Lagerfeld, best known for his creative work at Chanel, dies at the age of 85."),
            19
        );
        assert_eq!(
            word_count("A suicide bombing attack near Pulwama, in Indian administered Kashmir, kills 40 security personnel."),
            14
        );
        assert_eq!(
            word_count("NASA concludes the 15 year Opportunity Mars rover mission after being unable to wake the rover from hibernation."),
            18
        );
    }

    #[test]
    fn is_valid_url_matches_go_parse_request_uri() {
        // Absolute URLs with scheme are valid (Go's ParseRequestURI accepts them).
        assert!(is_valid_url("https://www.example.com/path"));
        assert!(is_valid_url("http://localhost:8080/"));
        assert!(is_valid_url("ftp://ftp.example.com/file.txt"));
        // file:/// has no host but Go's ParseRequestURI accepts it — so do we.
        assert!(is_valid_url("file:///etc/passwd"));
        // Relative paths have no scheme → rejected by both Go and Rust.
        assert!(!is_valid_url("/authors/jane"));
        assert!(!is_valid_url("relative/path"));
        assert!(!is_valid_url(""));
    }

    #[test]
    fn char_count_is_unicode_codepoints() {
        assert_eq!(char_count("hello"), 5);
        assert_eq!(char_count("héllo"), 5); // é is one codepoint
        assert_eq!(char_count(""), 0);
    }

    #[test]
    fn has_content_detects_non_whitespace() {
        assert!(has_content("hello"));
        assert!(has_content("  a  "));
        assert!(!has_content("   "));
        assert!(!has_content(""));
    }

    #[test]
    fn to_absolute_uri_resolves_correctly() {
        let base = Url::parse("http://localhost:8080/absolute/").unwrap();
        let cases = [
            ("#here", "#here"),
            ("/test/123", "http://localhost:8080/test/123"),
            ("test/123", "http://localhost:8080/absolute/test/123"),
            ("//www.google.com", "http://www.google.com"),
            ("https://www.google.com", "https://www.google.com"),
            ("ftp://ftp.server.com", "ftp://ftp.server.com"),
            (
                "www.google.com",
                "http://localhost:8080/absolute/www.google.com",
            ),
            (
                "http//www.google.com",
                "http://localhost:8080/absolute/http//www.google.com",
            ),
            ("../hello/relative", "http://localhost:8080/hello/relative"),
        ];
        for (uri, expected) in cases {
            assert_eq!(
                to_absolute_uri(uri, &base),
                expected,
                "to_absolute_uri({uri:?})"
            );
        }
    }

    #[test]
    fn to_absolute_uri_space_in_url() {
        let base = Url::parse("http://fakehost/test/page.html").unwrap();
        let result = to_absolute_uri("hmhome.gif ", &base);
        println!("space url result: {result:?}");
        // trailing space should be preserved as %20
        assert_eq!(result, "http://fakehost/test/hmhome.gif%20");

        let base2 = Url::parse("http://fakehost/test/page.html").unwrap();
        let pipe_url = "file:///C|/Documents%20and%20Settings/file.gif";
        let r2 = to_absolute_uri(pipe_url, &base2);
        println!("C| file url result: {r2:?}");
    }

    #[test]
    fn str_or_returns_first_non_empty() {
        assert_eq!(str_or(&["", "", "third"]), "third");
        assert_eq!(str_or(&["first", "second"]), "first");
        assert_eq!(str_or(&["", ""]), "");
        assert_eq!(str_or(&[]), "");
    }

    #[test]
    fn text_similarity_empty_is_zero() {
        assert_eq!(text_similarity("", ""), 0.0);
    }

    #[test]
    fn text_similarity_identical_is_one() {
        assert_eq!(text_similarity("hello world", "hello world"), 1.0);
    }

    #[test]
    fn text_similarity_case_insensitive() {
        // Go lowercases before comparing, so different case = identical.
        assert_eq!(text_similarity("Hello World", "hello world"), 1.0);
        assert_eq!(text_similarity("HELLO WORLD", "hello world"), 1.0);
    }

    #[test]
    fn text_similarity_disjoint_is_zero() {
        assert_eq!(text_similarity("foo bar", "baz qux"), 0.0);
    }

    #[test]
    fn text_similarity_partial() {
        // tokens_b = ["hello", "earth"], unique_b (not in A) = ["earth"]
        // merged_b_len = len("hello earth") = 11
        // unique_b_len = len("earth") = 5
        // similarity = 1 - 5/11 ≈ 0.545
        let sim = text_similarity("hello world", "hello earth");
        let expected = 1.0 - 5.0_f64 / 11.0;
        assert!(
            (sim - expected).abs() < 1e-9,
            "expected ~{expected:.4}, got {sim}"
        );
    }

    #[test]
    fn text_similarity_b_subset_of_a() {
        // All tokens in B are in A → unique_b = [] → similarity = 1.0
        assert_eq!(text_similarity("hello world today", "hello world"), 1.0);
    }
}
