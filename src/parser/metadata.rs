// Metadata extraction: title, JSON-LD, favicon, article metadata, HTML unescape.

use std::collections::HashMap;

use super::Parser;
use crate::regexp::*;
use crate::utils::{
    char_count, is_valid_url, str_or, text_similarity, to_absolute_uri, word_count,
};

/// Typed metadata extracted from `<meta>` tags and JSON-LD.
///
/// Replaces the previous `HashMap<String, String>` for type safety and clarity.
#[derive(Debug, Default)]
pub(crate) struct ArticleMetadata {
    pub(crate) title: String,
    pub(crate) byline: String,
    pub(crate) excerpt: String,
    pub(crate) site_name: String,
    pub(crate) image: String,
    pub(crate) favicon: String,
    pub(crate) published_time: String,
    pub(crate) modified_time: String,
}

/// Typed metadata extracted from JSON-LD `<script>` tags.
#[derive(Debug, Default)]
pub(super) struct JsonLdMetadata {
    pub(super) title: String,
    pub(super) byline: String,
    pub(super) excerpt: String,
    pub(super) site_name: String,
    pub(super) date_published: String,
}

impl Parser {
    // ── Metadata extraction ───────────────────────────────────────────────

    /// Port of `getArticleTitle` — extract and clean the page title.
    pub(super) fn get_article_title(&self) -> String {
        let title_node = self.get_element_by_tag_name(self.doc.root(), "title");
        let orig_title = title_node
            .map(|t| self.get_inner_text(t, true))
            .unwrap_or_default();
        let mut cur_title = orig_title.clone();
        let mut had_hierarchical_sep = false;

        if RX_TITLE_SEPARATOR.is_match(&cur_title) {
            had_hierarchical_sep = RX_TITLE_HIERARCHY_SEP.is_match(&cur_title);
            cur_title = RX_TITLE_REMOVE_FINAL_PART
                .replace(&orig_title, "$1")
                .into_owned();

            if word_count(&cur_title) < 3 {
                cur_title = RX_TITLE_REMOVE_1ST_PART
                    .replace(&orig_title, "$1")
                    .into_owned();
            }
        } else if cur_title.contains(": ") {
            let root = self.doc.root();
            let headings = self.doc.get_all_nodes_with_tag(root, &["h1", "h2"]);
            let trimmed = cur_title.trim().to_string();
            let match_found = headings
                .iter()
                .any(|&h| self.doc.text_content(h).trim() == trimmed.as_str());

            if !match_found {
                // Port of strings.LastIndex(origTitle, ":") + 1 — match Go exactly.
                // Leading space after ':' is stripped by normalize_spaces(trim()) below.
                let last_colon = orig_title.rfind(':').map(|i| i + 1).unwrap_or(0);
                cur_title = orig_title[last_colon..].to_string();

                if word_count(&cur_title) < 3 {
                    let first_colon = orig_title.find(':').map(|i| i + 1).unwrap_or(0);
                    cur_title = orig_title[first_colon..].to_string();
                } else {
                    // Port of strings.Index(origTitle, ":") — bare colon, not colon-space.
                    let pre_colon_words = orig_title
                        .find(':')
                        .map(|i| word_count(&orig_title[..i]))
                        .unwrap_or(0);
                    if pre_colon_words > 5 {
                        cur_title = orig_title.clone();
                    }
                }
            }
        } else if char_count(&cur_title) > 150 || char_count(&cur_title) < 15 {
            let root = self.doc.root();
            let h1s = self.doc.get_elements_by_tag_name(root, "h1");
            if h1s.len() == 1 {
                cur_title = self.get_inner_text(h1s[0], true);
            }
        }

        cur_title = normalize_spaces(cur_title.trim());

        let cur_word_count = word_count(&cur_title);
        let tmp_orig = RX_TITLE_ANY_SEPARATOR
            .replace_all(&orig_title, "")
            .into_owned();

        // Go uses signed integer subtraction: wordCount(tmpOrig) - 1 can be -1 when
        // wordCount is 0 (purely separator titles), which is always != any usize word count.
        // Mirror that by casting to i64.
        if cur_word_count <= 4
            && (!had_hierarchical_sep || cur_word_count as i64 != word_count(&tmp_orig) as i64 - 1)
        {
            cur_title = orig_title;
        }

        cur_title
    }

    /// Port of `getJSONLD` — extract Schema.org metadata from `<script type="application/ld+json">`.
    pub(super) fn get_jsonld(&self) -> JsonLdMetadata {
        let mut metadata: Option<JsonLdMetadata> = None;

        let root = self.doc.root();
        let scripts = self
            .doc
            .query_selector_all(root, r#"script[type="application/ld+json"]"#);

        for script in scripts {
            if metadata.is_some() {
                break;
            }

            let content = self.doc.text_content(script);
            let content = RX_CDATA.replace_all(&content, "").into_owned();

            let parsed: serde_json::Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Find the right object (may be an array of items, or a @graph, or a direct object).
            let obj: &serde_json::Map<String, serde_json::Value> = match parsed {
                serde_json::Value::Array(ref arr) => {
                    match arr
                        .iter()
                        .find(|item| {
                            item.get("@type")
                                .and_then(|t| t.as_str())
                                .map(|t| RX_JSON_LD_ARTICLE_TYPES.is_match(t))
                                .unwrap_or(false)
                        })
                        .and_then(|v| v.as_object())
                    {
                        Some(o) => o,
                        None => continue,
                    }
                }
                serde_json::Value::Object(ref m) => m,
                _ => continue,
            };

            // Validate @context is schema.org (always, for both array items and top-level objects).
            let context_ok = match obj.get("@context") {
                Some(serde_json::Value::String(s)) => RX_SCHEMA_ORG.is_match(s),
                Some(serde_json::Value::Object(m)) => m
                    .get("@vocab")
                    .and_then(|v| v.as_str())
                    .map(|s| RX_SCHEMA_ORG.is_match(s))
                    .unwrap_or(false),
                _ => false,
            };
            if !context_ok {
                continue;
            }

            // If no @type, look in @graph for an article type.
            let final_obj: &serde_json::Map<String, serde_json::Value>;
            let graph_obj; // storage for borrowed value
            if !obj.contains_key("@type") {
                let graph = match obj.get("@graph").and_then(|g| g.as_array()) {
                    Some(g) => g,
                    None => continue,
                };
                graph_obj = graph
                    .iter()
                    .find(|item| {
                        item.get("@type")
                            .and_then(|t| t.as_str())
                            .map(|t| RX_JSON_LD_ARTICLE_TYPES.is_match(t))
                            .unwrap_or(false)
                    })
                    .and_then(|v| v.as_object());
                match graph_obj {
                    Some(go) => final_obj = go,
                    None => continue,
                }
            } else {
                final_obj = obj;
            }

            // Validate @type.
            let type_ok = final_obj
                .get("@type")
                .and_then(|t| t.as_str())
                .map(|t| RX_JSON_LD_ARTICLE_TYPES.is_match(t))
                .unwrap_or(false);
            if !type_ok {
                continue;
            }

            let mut meta = JsonLdMetadata::default();

            // Title: prefer name/headline whichever better matches HTML title.
            let name = final_obj
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::trim);
            let headline = final_obj
                .get("headline")
                .and_then(|v| v.as_str())
                .map(str::trim);
            match (name, headline) {
                (Some(n), Some(h)) if n != h => {
                    let title = self.get_article_title();
                    let name_matches = text_similarity(n, &title) > 0.75;
                    let headline_matches = text_similarity(h, &title) > 0.75;
                    if headline_matches && !name_matches {
                        meta.title = h.to_string();
                    } else {
                        meta.title = n.to_string();
                    }
                }
                (Some(n), _) => {
                    meta.title = n.to_string();
                }
                (_, Some(h)) => {
                    meta.title = h.to_string();
                }
                _ => {}
            }

            // Author.
            match final_obj.get("author") {
                Some(serde_json::Value::Object(a)) => {
                    if let Some(n) = a.get("name").and_then(|v| v.as_str()) {
                        meta.byline = n.trim().to_string();
                    }
                }
                Some(serde_json::Value::Array(arr)) => {
                    let authors: Vec<String> = arr
                        .iter()
                        .filter_map(|a| a.get("name")?.as_str())
                        .map(|s| s.trim().to_string())
                        .collect();
                    meta.byline = authors.join(", ");
                }
                _ => {}
            }

            // Description / excerpt.
            if let Some(desc) = final_obj.get("description").and_then(|v| v.as_str()) {
                meta.excerpt = desc.trim().to_string();
            }

            // Publisher / site name.
            if let Some(pub_name) = final_obj
                .get("publisher")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
            {
                meta.site_name = pub_name.trim().to_string();
            }

            // Date published.
            if let Some(dp) = final_obj.get("datePublished").and_then(|v| v.as_str()) {
                meta.date_published = dp.to_string();
            }

            metadata = Some(meta);
        }

        metadata.unwrap_or_default()
    }

    /// Port of `getArticleFavicon` — find the best PNG favicon from `<link>` elements.
    pub(super) fn get_article_favicon(&self) -> String {
        let root = self.doc.root();
        let links = self.doc.get_elements_by_tag_name(root, "link");

        let mut favicon = String::new();
        let mut favicon_size: i32 = -1;

        for link in links {
            let rel = self.doc.attr(link, "rel").unwrap_or("").trim().to_string();
            let link_type = self.doc.attr(link, "type").unwrap_or("").trim().to_string();
            let href = self.doc.attr(link, "href").unwrap_or("").trim().to_string();
            let sizes = self
                .doc
                .attr(link, "sizes")
                .unwrap_or("")
                .trim()
                .to_string();

            if href.is_empty() || !rel.contains("icon") {
                continue;
            }
            if link_type != "image/png" && !href.contains(".png") {
                continue;
            }

            let mut size = 0i32;
            for loc in &[sizes.as_str(), href.as_str()] {
                if let Some(caps) = RX_FAVICON_SIZE.captures(loc) {
                    let w = caps.get(1).map_or("", |m| m.as_str());
                    let h = caps.get(2).map_or("", |m| m.as_str());
                    if w == h {
                        size = w.parse().unwrap_or(0);
                        break;
                    }
                }
            }

            if size > favicon_size {
                favicon_size = size;
                favicon = href;
            }
        }

        if let Some(base) = &self.document_uri {
            to_absolute_uri(&favicon, base)
        } else {
            favicon
        }
    }

    /// Port of `getArticleMetadata` — collect metadata from `<meta>` tags and JSON-LD.
    pub(super) fn get_article_metadata(&self, json_ld: &JsonLdMetadata) -> ArticleMetadata {
        let root = self.doc.root();
        let metas = self.doc.get_elements_by_tag_name(root, "meta");
        let mut values: HashMap<String, String> = HashMap::new();

        for meta in metas {
            let element_property = self.doc.attr(meta, "property").unwrap_or("").to_string();
            let element_name = self.doc.attr(meta, "name").unwrap_or("").to_string();
            let content = self.doc.attr(meta, "content").unwrap_or("").to_string();

            if content.is_empty() {
                continue;
            }

            let mut matches: Vec<String> = Vec::new();

            if !element_property.is_empty() {
                // Go processes matches in reverse order, so first match wins.
                let all_matches: Vec<_> =
                    RX_PROPERTY_PATTERN.find_iter(&element_property).collect();
                for m in all_matches.into_iter().rev() {
                    let name = m.as_str().to_lowercase();
                    let name: String = name.split_whitespace().collect();
                    matches.push(name.clone());
                    values.insert(name, content.trim().to_string());
                }
            }

            if matches.is_empty()
                && !element_name.is_empty()
                && RX_NAME_PATTERN.is_match(&element_name)
            {
                let name = element_name.to_lowercase();
                let name: String = name.split_whitespace().collect();
                let name = name.replace('.', ":");
                values.insert(name, content.trim().to_string());
            }
        }

        let empty = String::new();

        // Build a helper to look up in values map with fallback to empty.
        let v = |key: &str| values.get(key).unwrap_or(&empty).as_str();

        let metadata_title = {
            let t = str_or(&[
                &json_ld.title,
                v("dc:title"),
                v("dcterm:title"),
                v("og:title"),
                v("weibo:article:title"),
                v("weibo:webpage:title"),
                v("title"),
                v("twitter:title"),
                v("parsely-title"),
            ]);
            if t.is_empty() {
                self.get_article_title()
            } else {
                t.to_string()
            }
        };

        let metadata_byline = {
            let b = str_or(&[
                &json_ld.byline,
                v("dc:creator"),
                v("dcterm:creator"),
                v("author"),
                v("parsely-author"),
            ]);
            if b.is_empty() {
                let article_author = v("article:author");
                if !article_author.is_empty() && !is_valid_url(article_author) {
                    article_author.to_string()
                } else {
                    b.to_string()
                }
            } else {
                b.to_string()
            }
        };

        let metadata_excerpt = str_or(&[
            &json_ld.excerpt,
            v("dc:description"),
            v("dcterm:description"),
            v("og:description"),
            v("weibo:article:description"),
            v("weibo:webpage:description"),
            v("description"),
            v("twitter:description"),
        ])
        .to_string();

        let metadata_site_name = str_or(&[&json_ld.site_name, v("og:site_name")]).to_string();

        let metadata_image = str_or(&[v("og:image"), v("image"), v("twitter:image")]).to_string();

        let metadata_favicon = self.get_article_favicon();

        let metadata_published_time = str_or(&[
            &json_ld.date_published,
            v("article:published_time"),
            v("dcterms.available"),
            v("dcterms.created"),
            v("dcterms.issued"),
            v("weibo:article:create_at"),
            v("parsely-pub-date"),
        ])
        .to_string();

        // Go's getJSONLD never extracts dateModified, so no JSON-LD fallback here.
        let metadata_modified_time =
            str_or(&[v("article:modified_time"), v("dcterms.modified")]).to_string();

        ArticleMetadata {
            title: html_unescape(&metadata_title),
            byline: html_unescape(&metadata_byline),
            excerpt: html_unescape(&metadata_excerpt),
            site_name: html_unescape(&metadata_site_name),
            image: metadata_image,
            favicon: metadata_favicon,
            published_time: html_unescape(&metadata_published_time),
            modified_time: html_unescape(&metadata_modified_time),
        }
    }
}

// ── HTML entity helpers ──────────────────────────────────────────────────────

/// Minimal HTML entity decoder.
///
/// Handles the most common named entities and numeric character references.
/// Attribute values parsed by html5ever are already decoded; this is a safety pass
/// for occasionally double-encoded metadata fields.
pub(super) fn html_unescape(s: &str) -> String {
    use std::sync::LazyLock;
    static RX_ENTITY: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"&(?:#x([0-9a-fA-F]+)|#([0-9]+)|([a-zA-Z][a-zA-Z0-9]*));").unwrap()
    });

    if !s.contains('&') {
        return s.to_string();
    }

    RX_ENTITY
        .replace_all(s, |caps: &regex::Captures| {
            if let Some(hex) = caps.get(1) {
                let code = u32::from_str_radix(hex.as_str(), 16).unwrap_or(0xFFFD);
                char::from_u32(code)
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "\u{FFFD}".to_string())
            } else if let Some(dec) = caps.get(2) {
                let code: u32 = dec.as_str().parse().unwrap_or(0xFFFD);
                char::from_u32(code)
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "\u{FFFD}".to_string())
            } else if let Some(name) = caps.get(3) {
                named_html_entity(name.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| caps[0].to_string()) // keep unknown entities
            } else {
                caps[0].to_string()
            }
        })
        .into_owned()
}

fn named_html_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => "\u{00A0}",
        "shy" => "\u{00AD}",
        "mdash" => "\u{2014}",
        "ndash" => "\u{2013}",
        "lsquo" => "\u{2018}",
        "rsquo" => "\u{2019}",
        "ldquo" => "\u{201C}",
        "rdquo" => "\u{201D}",
        "hellip" => "\u{2026}",
        "bull" => "\u{2022}",
        "copy" => "\u{00A9}",
        "reg" => "\u{00AE}",
        "trade" => "\u{2122}",
        "euro" => "\u{20AC}",
        "pound" => "\u{00A3}",
        "yen" => "\u{00A5}",
        "cent" => "\u{00A2}",
        _ => return None,
    })
}
