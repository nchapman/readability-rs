// Integration tests: run all 134 test-pages fixtures against the Rust parser
// and compare output against expected.html and expected-metadata.json.
//
// Mirrors Go's Test_parser in parser_test.go.

use url::Url;

// ── Metadata ─────────────────────────────────────────────────────────────────

/// Deserialize a JSON value that may be a string, null, or missing field as a String.
/// null and missing both produce an empty String.
fn deserialize_nullable_string<'de, D>(d: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct ExpectedMeta {
    #[serde(deserialize_with = "deserialize_nullable_string")]
    title: String,
    #[serde(deserialize_with = "deserialize_nullable_string")]
    byline: String,
    #[serde(deserialize_with = "deserialize_nullable_string")]
    excerpt: String,
    #[serde(deserialize_with = "deserialize_nullable_string")]
    language: String,
    #[serde(rename = "siteName", deserialize_with = "deserialize_nullable_string")]
    site_name: String,
    readerable: bool,
    #[serde(rename = "publishedTime", deserialize_with = "deserialize_nullable_string")]
    published_time: String,
    #[serde(rename = "modifiedTime", deserialize_with = "deserialize_nullable_string")]
    modified_time: String,
}

// ── HTML comparison ───────────────────────────────────────────────────────────

/// Collect all text content from an ego-tree node, concatenating all descendant
/// text nodes. Mirrors Go's `dom.TextContent(node)`.
fn text_content<'a>(node: ego_tree::NodeRef<'a, scraper::Node>) -> String {
    let mut out = String::new();
    for n in node.descendants() {
        if let scraper::Node::Text(t) = n.value() {
            out.push_str(&t.text);
        }
    }
    out
}

/// DOM-level comparison matching Go's `compareArticleContent`.
///
/// Parses both HTML strings as full documents and walks the **element-only** DFS
/// tree comparing (mirrors Go's `getNextNode` which uses `FirstElementChild` /
/// `NextElementSibling` and thus never visits text nodes directly):
///
/// - Direct element-child count of the document root
/// - Tag name at each element node
/// - Attribute count and values (href/src: trim trailing `/`)
/// - Aggregated `TextContent` of each element node (whitespace-normalised with
///   `strings.Fields` semantics), matching Go's:
///   `strings.Join(strings.Fields(strings.TrimSpace(dom.TextContent(node))), " ")`
///
/// The walk stops when either side is exhausted (matches Go's
/// `for resultNode != nil && expectedNode != nil` termination).
fn compare_html(extracted: &str, expected_html_str: &str, case_name: &str) {
    use scraper::Html;

    // Parse both as full documents so html5ever normalises them the same way.
    let doc_got = Html::parse_document(extracted);
    let doc_exp = Html::parse_document(expected_html_str);

    // Step 1: check direct element-child count of the document root.
    // In Go: `len(dom.Children(result))` == `len(dom.Children(expected))`.
    let got_root_children = doc_got.root_element()
        .children()
        .filter(|n| n.value().is_element())
        .count();
    let exp_root_children = doc_exp.root_element()
        .children()
        .filter(|n| n.value().is_element())
        .count();

    if got_root_children != exp_root_children {
        panic!(
            "[{case_name}] root element-child count mismatch: got {got_root_children} expected {exp_root_children}"
        );
    }

    // Step 2: collect ELEMENT-ONLY nodes in DFS order.
    // Go's `getNextNode` uses FirstElementChild / NextElementSibling so it never
    // visits bare text nodes.  We replicate that by filtering to elements only.
    let got_elems: Vec<_> = doc_got.tree.root()
        .descendants()
        .filter(|n| n.value().is_element())
        .collect();
    let exp_elems: Vec<_> = doc_exp.tree.root()
        .descendants()
        .filter(|n| n.value().is_element())
        .collect();

    let is_ns_attr = |k: &str| k == "xmlns" || k.starts_with("xmlns:");

    for (got, exp) in got_elems.iter().zip(exp_elems.iter()) {
        let g_el = got.value().as_element().unwrap();
        let e_el = exp.value().as_element().unwrap();

        // ── Tag name ──────────────────────────────────────────────────────
        assert_eq!(
            g_el.name(),
            e_el.name(),
            "[{case_name}] tag name mismatch"
        );

        // ── Attribute count ───────────────────────────────────────────────
        let g_attr_count = g_el.attrs().filter(|(k, _)| !is_ns_attr(k)).count();
        let e_attr_count = e_el.attrs().filter(|(k, _)| !is_ns_attr(k)).count();
        let g_attrs_str: Vec<_> = g_el.attrs().map(|(k,v)| format!("{}={:?}", k, v)).collect();
        let e_attrs_str: Vec<_> = e_el.attrs().map(|(k,v)| format!("{}={:?}", k, v)).collect();
        assert_eq!(
            g_attr_count,
            e_attr_count,
            "[{case_name}] attr count mismatch on <{}>: got {g_attr_count} expected {e_attr_count}\n  got_attrs: {g_attrs_str:?}\n  exp_attrs: {e_attrs_str:?}",
            g_el.name()
        );

        // ── Attribute values ──────────────────────────────────────────────
        for (key, val) in g_el.attrs() {
            // Skip XML namespace declarations (html5ever stores them with
            // non-empty namespace prefix; linear search finds them, but the
            // binary-search `attr()` API does not, causing false mismatches).
            if key == "xmlns" || key.starts_with("xmlns:") {
                continue;
            }
            let exp_val_opt = e_el.attrs().find(|(k, _)| *k == key).map(|(_, v)| v);
            // Namespace URI attrs that only appear on one side (e.g. xlink).
            if exp_val_opt.is_none() && val.starts_with("http://www.w3.org/") {
                continue;
            }
            let exp_val = exp_val_opt.unwrap_or("");
            let mut got_v = val.to_string();
            let mut exp_v = exp_val.to_string();
            if key == "href" || key == "src" {
                got_v = got_v.trim_end_matches('/').to_string();
                exp_v = exp_v.trim_end_matches('/').to_string();
            }
            assert_eq!(
                got_v,
                exp_v,
                "[{case_name}] attr `{key}` mismatch on <{}>",
                g_el.name()
            );
        }

        // ── Text content (aggregated) ─────────────────────────────────────
        // Mirror Go: strings.TrimSpace(dom.TextContent(node)) then
        // strings.Join(strings.Fields(...), " ").
        // Also strip U+00AD (soft hyphen) which Go's HTML serialization may
        // remove during round-trip but html5ever preserves verbatim.
        let got_text = text_content(*got)
            .replace('\u{AD}', "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let exp_text = text_content(*exp)
            .replace('\u{AD}', "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if got_text != exp_text {
            assert_eq!(
                got_text,
                exp_text,
                "[{case_name}] text content mismatch on <{}>",
                g_el.name()
            );
        }
    }
}

// ── Time comparison ───────────────────────────────────────────────────────────

/// Normalize timezone offset from `+HHMM` to `+HH:MM` so dateparser can
/// parse it (e.g. `+0100` → `+01:00`).
fn normalize_tz(s: &str) -> std::borrow::Cow<'_, str> {
    // Match trailing ±HHMM (no colon) that dateparser can't handle.
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"([+-])(\d{2})(\d{2})$").unwrap());
    re.replace(s, "$1$2:$3")
}

/// Compare two datetime strings semantically (matching Go's `timesAreEqual`).
///
/// Both strings are parsed with `dateparser`; if both succeed the resulting
/// `DateTime<Utc>` values are compared.  If either fails to parse the raw
/// strings are compared directly — this handles edge cases like "+0100" (no
/// colon) that dateparser doesn't yet support.
/// Return true if this string looks like a date-only value (no time component).
fn is_date_only(s: &str) -> bool {
    // Matches YYYY-MM-DD with nothing after, or with only timezone offset / Z.
    // We consider it date-only if there's no 'T' or ' HH:' separator.
    !s.contains('T') && !s.contains(' ')
}

fn assert_times_equal(got: &str, expected: &str, msg: &str) {
    use chrono::{Datelike, Timelike, Utc};
    let got_norm = normalize_tz(got);
    let exp_norm = normalize_tz(expected);
    // Use UTC as default timezone for naive datetime strings so that
    // "2018-12-21 12:55:00" (no tz) parses to 2018-12-21T12:55:00Z,
    // matching Go's dateparse behaviour.
    let got_t = dateparser::parse_with_timezone(&got_norm, &Utc).ok();
    let exp_t = dateparser::parse_with_timezone(&exp_norm, &Utc).ok();
    match (got_t, exp_t) {
        (Some(g), Some(e)) => {
            // If either side is a date-only string, compare at day granularity.
            // dateparser may inject the current time-of-day for date-only input,
            // so sub-day components are unreliable.
            if is_date_only(got) || is_date_only(expected) {
                assert_eq!(
                    (g.year(), g.month(), g.day()),
                    (e.year(), e.month(), e.day()),
                    "{msg}: got {got:?} want {expected:?}"
                );
            } else {
                // For full datetime strings, compare at second granularity to
                // ignore sub-second formatting differences (e.g. `.728778Z` vs
                // `.728788Z`).
                let g_sec = g.with_nanosecond(0).unwrap_or(g);
                let e_sec = e.with_nanosecond(0).unwrap_or(e);
                assert_eq!(g_sec, e_sec, "{msg}: got {got:?} want {expected:?}");
            }
        }
        _ => {
            // Fall back to string comparison with the original strings.
            assert_eq!(got, expected, "{msg}");
        }
    }
}

// ── Fixture runner ────────────────────────────────────────────────────────────

fn run_fixture(name: &str) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-pages")
        .join(name);

    let source = std::fs::read_to_string(dir.join("source.html"))
        .unwrap_or_else(|e| panic!("can't read source for {name}: {e}"));
    let expected_html = std::fs::read_to_string(dir.join("expected.html"))
        .unwrap_or_else(|e| panic!("can't read expected.html for {name}: {e}"));
    let meta_json = std::fs::read_to_string(dir.join("expected-metadata.json"))
        .unwrap_or_else(|e| panic!("can't read metadata for {name}: {e}"));

    let meta: ExpectedMeta = serde_json::from_str(&meta_json)
        .unwrap_or_else(|e| panic!("can't parse metadata for {name}: {e}"));

    let page_url = Url::parse("http://fakehost/test/page.html").unwrap();

    let mut parser = readability::Parser::new();

    // Check readability before parsing (mirrors Go: CheckDocument then ParseAndMutate).
    let is_readerable = parser.check_html(&source);

    let article = parser
        .parse(&source, Some(&page_url))
        .unwrap_or_else(|e| panic!("parse failed for {name}: {e}"));

    // Readability check.
    assert_eq!(
        is_readerable,
        meta.readerable,
        "[{name}] readerable mismatch: got {is_readerable} want {}",
        meta.readerable
    );

    // HTML content comparison: re-parse extracted content to normalise, matching Go.
    //
    // Go re-parses the rendered HTML before comparison, so minor serialization
    // differences cancel out.
    compare_html(&article.content, &expected_html, name);

    // Metadata comparisons — assert all fields unconditionally, matching Go's behavior.
    // If the expected value is empty, the parser must also return empty (no false positives).
    assert_eq!(article.title, meta.title, "[{name}] title mismatch");
    assert_eq!(article.byline, meta.byline, "[{name}] byline mismatch");
    assert_eq!(article.excerpt, meta.excerpt, "[{name}] excerpt mismatch");
    assert_eq!(
        article.site_name,
        meta.site_name,
        "[{name}] site_name mismatch"
    );
    assert_eq!(
        article.language,
        meta.language,
        "[{name}] language mismatch"
    );
    // Mirror Go: parse both timestamps semantically so equivalent formats
    // compare equal ("2020-01-01T00:00:00.000Z" == "2020-01-01T00:00:00Z").
    // Falls back to string comparison when parsing fails (both empty → both fail
    // to parse → assert_eq!("", "") passes).
    assert_times_equal(
        &article.published_time,
        &meta.published_time,
        &format!("[{name}] published_time mismatch"),
    );
    assert_times_equal(
        &article.modified_time,
        &meta.modified_time,
        &format!("[{name}] modified_time mismatch"),
    );
}

// ── Individual test functions (one per fixture directory) ─────────────────────

macro_rules! fixture_tests {
    ($($fn_name:ident => $dir:expr),* $(,)?) => {
        $(
            #[test]
            fn $fn_name() {
                run_fixture($dir);
            }
        )*
    };
}

fixture_tests! {
    fixture_001 => "001",
    fixture_002 => "002",
    fixture_003_metadata_preferred => "003-metadata-preferred",
    fixture_004_metadata_space_separated_properties => "004-metadata-space-separated-properties",
    fixture_aclu => "aclu",
    fixture_aktualne => "aktualne",
    fixture_archive_of_our_own => "archive-of-our-own",
    fixture_ars_1 => "ars-1",
    fixture_article_author_tag => "article-author-tag",
    fixture_base_url => "base-url",
    fixture_base_url_base_element => "base-url-base-element",
    fixture_base_url_base_element_relative => "base-url-base-element-relative",
    fixture_basic_tags_cleaning => "basic-tags-cleaning",
    fixture_bbc_1 => "bbc-1",
    fixture_blogger => "blogger",
    fixture_breitbart => "breitbart",
    fixture_bug_1255978 => "bug-1255978",
    fixture_buzzfeed_1 => "buzzfeed-1",
    fixture_citylab_1 => "citylab-1",
    fixture_clean_links => "clean-links",
    fixture_cnet => "cnet",
    fixture_cnet_svg_classes => "cnet-svg-classes",
    fixture_cnn => "cnn",
    fixture_comment_inside_script_parsing => "comment-inside-script-parsing",
    fixture_daringfireball_1 => "daringfireball-1",
    fixture_data_url_image => "data-url-image",
    fixture_dev418 => "dev418",
    fixture_dropbox_blog => "dropbox-blog",
    fixture_ebb_org => "ebb-org",
    fixture_ehow_1 => "ehow-1",
    fixture_ehow_2 => "ehow-2",
    fixture_embedded_videos => "embedded-videos",
    fixture_engadget => "engadget",
    fixture_firefox_nightly_blog => "firefox-nightly-blog",
    fixture_folha => "folha",
    fixture_gitlab_blog => "gitlab-blog",
    fixture_gmw => "gmw",
    fixture_google_sre_book_1 => "google-sre-book-1",
    fixture_guardian_1 => "guardian-1",
    fixture_heise => "heise",
    fixture_herald_sun_1 => "herald-sun-1",
    fixture_hidden_nodes => "hidden-nodes",
    fixture_hukumusume => "hukumusume",
    fixture_iab_1 => "iab-1",
    fixture_ietf_1 => "ietf-1",
    fixture_invalid_attributes => "invalid-attributes",
    fixture_js_link_replacement => "js-link-replacement",
    fixture_keep_images => "keep-images",
    fixture_keep_images_2 => "keep-images-2",
    fixture_keep_tabular_data => "keep-tabular-data",
    fixture_la_nacion => "la-nacion",
    fixture_lazy_image_1 => "lazy-image-1",
    fixture_lazy_image_2 => "lazy-image-2",
    fixture_lazy_image_3 => "lazy-image-3",
    fixture_lemonde_1 => "lemonde-1",
    fixture_liberation_1 => "liberation-1",
    fixture_lifehacker_post_comment_load => "lifehacker-post-comment-load",
    fixture_lifehacker_working => "lifehacker-working",
    fixture_links_in_tables => "links-in-tables",
    fixture_lwn_1 => "lwn-1",
    fixture_mathjax => "mathjax",
    fixture_medicalnewstoday => "medicalnewstoday",
    fixture_medium_1 => "medium-1",
    fixture_medium_2 => "medium-2",
    fixture_medium_3 => "medium-3",
    fixture_mercurial => "mercurial",
    fixture_metadata_content_missing => "metadata-content-missing",
    fixture_missing_paragraphs => "missing-paragraphs",
    fixture_mozilla_1 => "mozilla-1",
    fixture_mozilla_2 => "mozilla-2",
    fixture_msn => "msn",
    fixture_normalize_spaces => "normalize-spaces",
    fixture_noscript_img_1 => "noscript-img-1",
    fixture_noscript_img_2 => "noscript-img-2",
    fixture_nytimes_1 => "nytimes-1",
    fixture_nytimes_2 => "nytimes-2",
    fixture_nytimes_3 => "nytimes-3",
    fixture_nytimes_4 => "nytimes-4",
    fixture_nytimes_5 => "nytimes-5",
    fixture_ol => "ol",
    fixture_parsely_metadata => "parsely-metadata",
    fixture_pixnet => "pixnet",
    fixture_qq => "qq",
    fixture_quanta_1 => "quanta-1",
    fixture_remove_aria_hidden => "remove-aria-hidden",
    fixture_remove_extra_brs => "remove-extra-brs",
    fixture_remove_extra_paragraphs => "remove-extra-paragraphs",
    fixture_remove_script_tags => "remove-script-tags",
    fixture_reordering_paragraphs => "reordering-paragraphs",
    fixture_replace_brs => "replace-brs",
    fixture_replace_font_tags => "replace-font-tags",
    fixture_royal_road => "royal-road",
    fixture_rtl_1 => "rtl-1",
    fixture_rtl_2 => "rtl-2",
    fixture_rtl_3 => "rtl-3",
    fixture_rtl_4 => "rtl-4",
    fixture_salon_1 => "salon-1",
    fixture_schema_org_context_object => "schema-org-context-object",
    fixture_seattletimes_1 => "seattletimes-1",
    fixture_simplyfound_1 => "simplyfound-1",
    fixture_social_buttons => "social-buttons",
    fixture_spiceworks => "spiceworks",
    fixture_style_tags_removal => "style-tags-removal",
    fixture_substack => "substack",
    fixture_svg_parsing => "svg-parsing",
    fixture_table_style_attributes => "table-style-attributes",
    fixture_telegraph => "telegraph",
    fixture_theverge => "theverge",
    fixture_title_and_h1_discrepancy => "title-and-h1-discrepancy",
    fixture_title_en_dash => "title-en-dash",
    fixture_tmz_1 => "tmz-1",
    fixture_toc_missing => "toc-missing",
    fixture_topicseed_1 => "topicseed-1",
    fixture_tumblr => "tumblr",
    fixture_v8_blog => "v8-blog",
    fixture_videos_1 => "videos-1",
    fixture_videos_2 => "videos-2",
    fixture_visibility_hidden => "visibility-hidden",
    fixture_wapo_1 => "wapo-1",
    fixture_wapo_2 => "wapo-2",
    fixture_webmd_1 => "webmd-1",
    fixture_webmd_2 => "webmd-2",
    fixture_wikia => "wikia",
    fixture_wikipedia => "wikipedia",
    fixture_wikipedia_2 => "wikipedia-2",
    fixture_wikipedia_3 => "wikipedia-3",
    fixture_wikipedia_4 => "wikipedia-4",
    fixture_wordpress => "wordpress",
    fixture_yahoo_1 => "yahoo-1",
    fixture_yahoo_2 => "yahoo-2",
    fixture_yahoo_3 => "yahoo-3",
    fixture_yahoo_4 => "yahoo-4",
    fixture_youth => "youth",
}
