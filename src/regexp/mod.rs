// Port of go-readability/internal/re2go/*.re and parser.go var block.
//
// All patterns are compiled once into LazyLock<Regex> globals.
// The re2go files use state-machine lexers; here we use standard regex
// with the same underlying pattern strings (see "Original pattern:" comments).

use std::sync::LazyLock;

use regex::Regex;

// ────────────────────────────────────────────────────────────────────────────
// Re2go patterns (from internal/re2go/*.re)
// ────────────────────────────────────────────────────────────────────────────

// Port of grab-article.re — IsUnlikelyCandidates
// Original pattern: (?i)-ad-|ai2html|banner|breadcrumbs|combx|comment|community|cover-wrap|disqus|extra|footer|gdpr|header|legends|menu|related|remark|replies|rss|shoutbox|sidebar|skyscraper|social|sponsor|supplemental|ad-break|agegate|pagination|pager|popup|yom-remote
static RX_UNLIKELY_CANDIDATES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)-ad-|ai2html|banner|breadcrumbs|combx|comment|community|cover-wrap|disqus|extra|footer|gdpr|header|legends|menu|related|remark|replies|rss|shoutbox|sidebar|skyscraper|social|sponsor|supplemental|ad-break|agegate|pagination|pager|popup|yom-remote").unwrap()
});

// Port of grab-article.re — MaybeItsACandidate
// Original pattern: (?i)and|article|body|column|content|main|mathjax|shadow
static RX_MAYBE_CANDIDATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)and|article|body|column|content|main|mathjax|shadow").unwrap()
});

// Port of class-weight.re — IsPositiveClass
// Original pattern: (?i)article|body|content|entry|hentry|h-entry|main|page|pagination|post|text|blog|story
static RX_POSITIVE_CLASS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)article|body|content|entry|hentry|h-entry|main|page|pagination|post|text|blog|story").unwrap()
});

// Port of class-weight.re — IsNegativeClass (anchored part)
// Original: (?i)(^| )(hid|hidden|d-none)( |$)
static RX_NEGATIVE_CLASS_1: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(^| )(hid|hidden|d-none)( |$)").unwrap());

// Port of class-weight.re — IsNegativeClass (substring part)
// Original: (?i)-ad-|banner|combx|comment|com-|contact|footer|gdpr|masthead|meta|outbrain|promo|related|share|shoutbox|sidebar|skyscraper|sponsor|shopping|tags|widget
static RX_NEGATIVE_CLASS_2: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)-ad-|banner|combx|comment|com-|contact|footer|gdpr|masthead|meta|outbrain|promo|related|share|shoutbox|sidebar|skyscraper|sponsor|shopping|tags|widget").unwrap()
});

// Port of check-byline.re — IsByline
// Original pattern: (?i)byline|author|dateline|writtenby|p-author
static RX_BYLINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)byline|author|dateline|writtenby|p-author").unwrap());

// Port of normalize.re — NormalizeSpaces
// Original pattern: [\t\n\f\r ]{2,} → " "
static RX_NORMALIZE_SPACES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\t\n\f\r ]{2,}").unwrap());

// ────────────────────────────────────────────────────────────────────────────
// Parser-level patterns (from parser.go var block, lines ~24–51)
// ────────────────────────────────────────────────────────────────────────────

pub static RX_VIDEOS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)//(www\.)?((dailymotion|youtube|youtube-nocookie|player\.vimeo|v\.qq|bilibili|live\.bilibili)\.com|(archive|upload\.wikimedia)\.org|player\.twitch\.tv)").unwrap()
});

// Port of Go RE2 textSimilarity tokenizer: Go's RE2 \W is ASCII-only ([^a-zA-Z0-9_]),
// so non-ASCII characters (e.g. Chinese) are treated as word delimiters. Mirror that
// behavior with an explicit ASCII-only negated class so split points are at valid
// Unicode codepoint boundaries (unlike (?-u)\W+ which splits at bytes).
pub static RX_TOKENIZE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-zA-Z0-9_]+").unwrap());

pub static RX_HAS_CONTENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\S$").unwrap());

pub static RX_PROPERTY_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s*(dc|dcterm|og|article|twitter)\s*:\s*(author|creator|description|title|site_name|published_time|modified_time|image\S*)\s*").unwrap()
});

pub static RX_NAME_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(?:(dc|dcterm|article|og|twitter|parsely|weibo:(article|webpage))\s*[-\.:]\s*)?(author|creator|pub-date|description|title|site_name|published_time|modified_time|image)\s*$").unwrap()
});

pub static RX_TITLE_SEPARATOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i) [\|\-–—\\/>»] ").unwrap());

pub static RX_TITLE_HIERARCHY_SEP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i) [\\/>»] ").unwrap());

pub static RX_TITLE_REMOVE_FINAL_PART: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(.*)[\|\-–—\\/>»] .*").unwrap());

pub static RX_TITLE_REMOVE_1ST_PART: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[^\|\-–—\\/>»]*[\|\-–—\\/>»](.*)").unwrap());

pub static RX_TITLE_ANY_SEPARATOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[\|\-–—\\/>»]+").unwrap());

pub static RX_DISPLAY_NONE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)display\s*:\s*none").unwrap());

pub static RX_VISIBILITY_HIDDEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)visibility\s*:\s*hidden").unwrap());

pub static RX_SENTENCE_PERIOD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.( |$)").unwrap());

pub static RX_SHARE_ELEMENTS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\b|_)(share|sharedaddy)(\b|_)").unwrap());

pub static RX_FAVICON_SIZE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\d+)x(\d+)").unwrap());

pub static RX_LAZY_IMAGE_SRCSET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.(jpg|jpeg|png|webp)\s+\d").unwrap());

pub static RX_LAZY_IMAGE_SRC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*\S+\.(jpg|jpeg|png|webp)\S*\s*$").unwrap());

pub static RX_IMG_EXTENSIONS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.(jpg|jpeg|png|webp)").unwrap());

pub static RX_SRCSET_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\S+)(\s+[\d.]+[xw])?(\s*(?:,|$))").unwrap());

pub static RX_B64_DATA_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^data:\s*([^\s;,]+)\s*;\s*base64\s*,").unwrap()
});

pub static RX_JSON_LD_ARTICLE_TYPES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^Article|AdvertiserContentArticle|NewsArticle|AnalysisNewsArticle|AskPublicNewsArticle|BackgroundNewsArticle|OpinionNewsArticle|ReportageNewsArticle|ReviewNewsArticle|Report|SatiricalArticle|ScholarlyArticle|MedicalScholarlyArticle|SocialMediaPosting|BlogPosting|LiveBlogPosting|DiscussionForumPosting|TechArticle|APIReference$").unwrap()
});

pub static RX_CDATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*<!\[CDATA\[|\]\]>\s*$").unwrap());

pub static RX_SCHEMA_ORG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^https?\:\/\/schema\.org\/?$").unwrap());

pub static RX_AD_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(ad(vertising|vertisement)?|pub(licité)?|werb(ung)?|广告|Реклама|Anuncio)$")
        .unwrap()
});

pub static RX_LOADING_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^((loading|正在加载|Загрузка|chargement|cargando)(…|\.\.\.)?)$").unwrap()
});

// ────────────────────────────────────────────────────────────────────────────
// Public functions (port of re2go function API)
// ────────────────────────────────────────────────────────────────────────────

/// Port of IsUnlikelyCandidates — true if `input` contains an unlikely-candidate substring.
pub fn is_unlikely_candidate(input: &str) -> bool {
    RX_UNLIKELY_CANDIDATES.is_match(input)
}

/// Port of MaybeItsACandidate — true if `input` contains a positive-candidate substring.
pub fn maybe_its_a_candidate(input: &str) -> bool {
    RX_MAYBE_CANDIDATE.is_match(input)
}

/// Port of IsPositiveClass — true if `input` contains a positive class name.
pub fn is_positive_class(input: &str) -> bool {
    RX_POSITIVE_CLASS.is_match(input)
}

/// Port of IsNegativeClass — true if `input` matches either the anchored or substring negatives.
///
/// Two patterns are OR'd together (as in the Go source):
/// - anchored: `(^| )(hid|hidden|d-none)( |$)`
/// - substring: `-ad-|banner|combx|…`
pub fn is_negative_class(input: &str) -> bool {
    RX_NEGATIVE_CLASS_1.is_match(input) || RX_NEGATIVE_CLASS_2.is_match(input)
}

/// Port of IsByline — true if `input` contains a byline-related term.
pub fn is_byline(input: &str) -> bool {
    RX_BYLINE.is_match(input)
}

/// Port of NormalizeSpaces — collapses runs of 2+ whitespace chars to a single space.
pub fn normalize_spaces(input: &str) -> String {
    RX_NORMALIZE_SPACES.replace_all(input, " ").into_owned()
}

// ────────────────────────────────────────────────────────────────────────────
// Tests (port of internal/re2go/re2go_test.go)
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn both_cases(s: &str) -> [String; 2] {
        [s.to_string(), s.to_uppercase()]
    }

    #[test]
    fn is_byline_matches() {
        let yes = [
            "article-byline",
            "author",
            "dateline",
            "meta-prep-author",
            "writtenbynames",
        ];
        let no = [
            "article-line",
            "autor",
            "bynames",
            "date",
            "meta-autor",
        ];
        for s in yes {
            for v in both_cases(s) {
                assert!(is_byline(&v), "IsByline({v:?}) should be true");
            }
        }
        for s in no {
            for v in both_cases(s) {
                assert!(!is_byline(&v), "IsByline({v:?}) should be false");
            }
        }
    }

    #[test]
    fn is_positive_class_matches() {
        let yes = [
            "article", "blog", "body", "content", "entry", "h-entry",
            "hentry", "main", "page", "pagination", "post", "story", "text",
        ];
        let no = [
            "container", "description", "footer", "gallery", "header",
            "layout", "navigation", "news", "sidebar", "toolbar", "widget",
        ];
        for s in yes {
            for v in both_cases(s) {
                assert!(is_positive_class(&v), "IsPositiveClass({v:?}) should be true");
            }
        }
        for s in no {
            for v in both_cases(s) {
                assert!(!is_positive_class(&v), "IsPositiveClass({v:?}) should be false");
            }
        }
    }

    #[test]
    fn is_negative_class_matches() {
        let yes = [
            "-ad-", "ad-banner", "banner", "class hid good", "class hid",
            "com-", "combx", "comment", "contact", "d-none", "footer",
            "gdpr", "hid class", "hid", "hidden", "masthead", "meta",
            "outbrain", "promo", "related", "share", "shopping", "shoutbox",
            "sidebar", "skyscraper", "sponsor", "tags", "widget",
        ];
        let no = [
            "catalog", "details", "foot", "footnote", "gallery", "media",
            "navbar", "news-feed", "overview", "profile", "scroll",
            "sad-nonet", "support", "tool", "toolbar", "user-menu",
            "visually-hidden",
        ];
        for s in yes {
            for v in both_cases(s) {
                assert!(is_negative_class(&v), "IsNegativeClass({v:?}) should be true");
            }
        }
        for s in no {
            for v in both_cases(s) {
                assert!(!is_negative_class(&v), "IsNegativeClass({v:?}) should be false");
            }
        }
    }

    #[test]
    fn is_unlikely_candidate_matches() {
        let yes = [
            "-ad-", "ad-banner", "ad-break", "agegate", "ai2html", "banner",
            "breadcrumbs", "combx", "comment", "community", "cover-wrap",
            "disqus", "extra", "footer", "gdpr", "header", "legends", "menu",
            "pager", "pagination", "popup", "related", "remark", "replies",
            "rss", "shoutbox", "sidebar", "skyscraper", "social", "sponsor",
            "supplemental", "yom-remote",
        ];
        let no = [
            "catalog", "container", "gallery", "newsfeed", "overview",
            "summary", "toolbar",
        ];
        for s in yes {
            for v in both_cases(s) {
                assert!(
                    is_unlikely_candidate(&v),
                    "IsUnlikelyCandidates({v:?}) should be true"
                );
            }
        }
        for s in no {
            for v in both_cases(s) {
                assert!(
                    !is_unlikely_candidate(&v),
                    "IsUnlikelyCandidates({v:?}) should be false"
                );
            }
        }
    }

    #[test]
    fn maybe_its_a_candidate_matches() {
        let yes = ["and", "article", "body", "column", "content", "main", "shadow"];
        let no = ["footer", "gallery", "header", "menu", "navbar", "text"];
        for s in yes {
            for v in both_cases(s) {
                assert!(
                    maybe_its_a_candidate(&v),
                    "MaybeItsACandidate({v:?}) should be true"
                );
            }
        }
        for s in no {
            for v in both_cases(s) {
                assert!(
                    !maybe_its_a_candidate(&v),
                    "MaybeItsACandidate({v:?}) should be false"
                );
            }
        }
    }

    #[test]
    fn normalize_spaces_collapses_whitespace() {
        assert_eq!(normalize_spaces("some   sentence"), "some sentence");
        assert_eq!(normalize_spaces("with \t \ttabs"), "with tabs");
        assert_eq!(
            normalize_spaces(" single space is ok "),
            " single space is ok "
        );
        assert_eq!(
            normalize_spaces("   multi   space   removed   "),
            " multi space removed "
        );
    }

    #[test]
    fn css_utility_classes_do_not_trigger() {
        // Utility classnames (Tailwind-style) must not match scoring heuristics.
        let classes = [
            "overflow-hidden",
            "sm:hidden",
            "stacked-navigation--hidden",
            "py-3 details-overlay-dark",
            "overflow-x-scroll",
        ];
        for cls in classes {
            assert!(
                !is_positive_class(cls),
                "IsPositiveClass should not trigger for {cls:?}"
            );
            assert!(
                !is_negative_class(cls),
                "IsNegativeClass should not trigger for {cls:?}"
            );
            assert!(
                !maybe_its_a_candidate(cls),
                "MaybeItsACandidate should not trigger for {cls:?}"
            );
            assert!(
                !is_unlikely_candidate(cls),
                "IsUnlikelyCandidates should not trigger for {cls:?}"
            );
        }
    }
}
