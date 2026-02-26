// Cleaning and post-processing: clean, clean_headers, clean_conditionally,
// prep_article, post_process_content, fix_relative_uris, clean_styles,
// remove_share_elements.

use ego_tree::NodeId;
use scraper::Node;

use super::helpers::find_content_in_node;
use super::scoring::{walk_cond, CondStats};
use super::Parser;
use super::{DEPRECATED_SIZE_ATTR_ELEMS, PRESENTATIONAL_ATTRS};
use crate::dom::Document;
use crate::regexp::*;
use crate::traverse::CharCounter;
use crate::utils::to_absolute_uri;

impl Parser {
    // ── Post-processing ───────────────────────────────────────────────────

    /// Port of `postProcessContent` — fix URIs, simplify nesting, strip classes.
    pub(super) fn post_process_content(&mut self, article_content: NodeId) {
        self.fix_relative_uris(article_content);
        self.simplify_nested_elements(article_content);
        if !self.keep_classes {
            self.clean_classes(article_content);
        }
        self.clear_readability_attr(article_content);
    }

    /// Port of `fixRelativeURIs` — convert relative links and media URLs to absolute.
    pub(super) fn fix_relative_uris(&mut self, article_content: NodeId) {
        let base_uri = self.document_uri.clone();

        // Fix <a href> links.
        let links = self.doc.get_elements_by_tag_name(article_content, "a");
        for link in links {
            let href = self.doc.attr(link, "href").unwrap_or("").to_string();
            if href.is_empty() {
                continue;
            }

            if href.starts_with("javascript:") {
                let children = self.doc.child_nodes(link);
                if children.len() == 1 {
                    if let Some(Node::Text(t)) =
                        self.doc.html.tree.get(children[0]).map(|n| n.value())
                    {
                        let text_content = t.text.as_ref().to_string();
                        let text_node = self.doc.create_text_node(&text_content);
                        self.doc.insert_before(link, text_node);
                        self.doc.remove(link);
                    } else {
                        let span = self.doc.create_element("span");
                        let kids: Vec<NodeId> = self.doc.child_nodes(link);
                        for kid in kids {
                            self.doc.append_child(span, kid);
                        }
                        self.doc.insert_before(link, span);
                        self.doc.remove(link);
                    }
                } else {
                    let span = self.doc.create_element("span");
                    let kids: Vec<NodeId> = self.doc.child_nodes(link);
                    for kid in kids {
                        self.doc.append_child(span, kid);
                    }
                    self.doc.insert_before(link, span);
                    self.doc.remove(link);
                }
                continue;
            }

            if let Some(base) = &base_uri {
                let new_href = to_absolute_uri(&href, base);
                if new_href.is_empty() {
                    self.doc.remove_attr(link, "href");
                } else {
                    self.doc.set_attr(link, "href", &new_href);
                }
            }
        }

        // Fix media elements (src, poster, srcset).
        let medias = self.doc.get_all_nodes_with_tag(
            article_content,
            &["img", "picture", "figure", "video", "audio", "source"],
        );
        for media in medias {
            if let Some(base) = &base_uri.clone() {
                if let Some(src) = self.doc.attr(media, "src").map(|s| s.to_string()) {
                    if !src.is_empty() {
                        self.doc
                            .set_attr(media, "src", &to_absolute_uri(&src, base));
                    }
                }
                if let Some(poster) = self.doc.attr(media, "poster").map(|s| s.to_string()) {
                    if !poster.is_empty() {
                        self.doc
                            .set_attr(media, "poster", &to_absolute_uri(&poster, base));
                    }
                }
                if let Some(srcset) = self.doc.attr(media, "srcset").map(|s| s.to_string()) {
                    if !srcset.is_empty() {
                        let base_clone = base.clone();
                        let new_srcset = RX_SRCSET_URL
                            .replace_all(&srcset, |caps: &regex::Captures| {
                                let url = caps.get(1).map_or("", |m| m.as_str());
                                let size = caps.get(2).map_or("", |m| m.as_str());
                                let sep = caps.get(3).map_or("", |m| m.as_str());
                                format!("{}{}{}", to_absolute_uri(url, &base_clone), size, sep)
                            })
                            .into_owned();
                        self.doc.set_attr(media, "srcset", &new_srcset);
                    }
                }
            }
        }
    }

    /// Port of `simplifyNestedElements` — collapse empty or redundant div/section wrappers.
    fn simplify_nested_elements(&mut self, article_content: NodeId) {
        // Mirror Go: start at articleContent itself (not its first child).
        // Go's guard `node.Parent != nil` means articleContent is skipped when
        // it has no parent (it's a detached wrapper div), so traversal descends
        // directly into page_div.  Starting one level too deep caused the
        // page_div's single-div children to be incorrectly collapsed.
        let mut node = Some(article_content);

        while let Some(n) = node {
            let parent = self.doc.parent(n);
            let tag = self.doc.tag_name(n).to_string();
            let node_id_attr = self.doc.attr(n, "id").unwrap_or("").to_string();

            if parent.is_some()
                && (tag == "div" || tag == "section")
                && !node_id_attr.starts_with("readability")
            {
                if self.is_element_without_content(n) {
                    node = self.remove_and_get_next(n);
                    continue;
                }
                if self.has_single_tag_inside_element(n, "div")
                    || self.has_single_tag_inside_element(n, "section")
                {
                    let child = self.doc.first_element_child(n).expect(
                        "has_single_tag_inside_element guarantees exactly one element child",
                    );
                    // Copy parent attrs to child.
                    let parent_attrs = self.doc.get_all_attrs(n);
                    for (k, v) in parent_attrs {
                        self.doc.set_attr(child, &k, &v);
                    }
                    // Replace n with child.
                    self.doc.insert_before(n, child);
                    self.doc.remove(n);
                    node = Some(child);
                    continue;
                }
            }
            node = self.get_next_node(n, false);
        }
    }

    /// Port of `clearReadabilityAttr` — remove `data-readability-*` attributes.
    ///
    /// In this Rust port we use side tables instead of DOM attributes, so these
    /// attributes are never set. This is a no-op kept for structural completeness.
    fn clear_readability_attr(&mut self, _id: NodeId) {
        // Side tables are dropped at end of parse_and_mutate; nothing to clean.
    }

    /// Port of `cleanStyles` — remove presentational attributes from all elements.
    pub(super) fn clean_styles(&mut self, id: NodeId) {
        let tag = self.doc.tag_name(id).to_string();
        if tag == "svg" {
            return;
        }

        let is_size_elem = DEPRECATED_SIZE_ATTR_ELEMS.contains(&tag.as_str());

        let attrs_to_remove: Vec<String> = {
            self.doc
                .get_all_attrs(id)
                .into_iter()
                .filter_map(|(k, _)| {
                    // Remove width/height only from deprecated size-attribute elements
                    // (table, th, td, hr, pre).  Keep them on <img> and others.
                    // Go: `if !isDeprecatedSizeAttributeElems { continue }; fallthrough`
                    if (k == "width" || k == "height") && is_size_elem {
                        return Some(k);
                    }
                    if PRESENTATIONAL_ATTRS.contains(&k.as_str()) {
                        return Some(k);
                    }
                    None
                })
                .collect()
        };

        for attr in attrs_to_remove {
            self.doc.remove_attr(id, &attr);
        }

        for child in self.doc.children(id) {
            self.clean_styles(child);
        }
    }

    // ── Cleaning helpers ──────────────────────────────────────────────────

    /// Port of `clean` — remove all elements with `tag` unless they are video embeds.
    pub(super) fn clean(&mut self, root: NodeId, tag: &str) {
        // Evaluate the filter at removal time (children before parents) so that
        // is_video_embed on an <object> sees its live innerHTML after inner embeds
        // have already been removed.
        let nodes = self.doc.get_elements_by_tag_name(root, tag);
        for &id in nodes.iter().rev() {
            if self.doc.parent(id).is_some() && !self.is_video_embed(id) {
                self.doc.remove(id);
            }
        }
    }

    /// Port of `cleanHeaders` — remove h1/h2 with negative class weight.
    pub(super) fn clean_headers(&mut self, root: NodeId) {
        let nodes = self.doc.get_all_nodes_with_tag(root, &["h1", "h2"]);
        for &id in nodes.iter().rev() {
            if self.doc.parent(id).is_some() && self.get_class_weight(id) < 0 {
                self.doc.remove(id);
            }
        }
    }

    /// Port of `cleanConditionally` — remove elements that look like non-content.
    pub(super) fn clean_conditionally(&mut self, root: NodeId, tag: &str) {
        if !self.flags.clean_conditionally {
            return;
        }
        // Port of Go's removeNodes with a filter: iterate in reverse order (children before
        // parents) so that when we evaluate a parent, its already-removed children don't
        // inflate the link density and cause the parent to be incorrectly removed.
        let nodes = self.doc.get_elements_by_tag_name(root, tag);
        for &node in nodes.iter().rev() {
            if self.doc.parent(node).is_some() && self.should_clean_conditionally(node, tag) {
                #[cfg(feature = "tracing")]
                tracing::trace!(
                    tag,
                    class = %self.doc.attr(node, "class").unwrap_or(""),
                    id = %self.doc.attr(node, "id").unwrap_or(""),
                    "clean_conditionally: removing node"
                );
                self.doc.remove(node);
            }
        }
    }

    /// Determine whether a single node should be removed by `clean_conditionally`.
    fn should_clean_conditionally(&self, node: NodeId, tag: &str) -> bool {
        // Data tables are never removed.
        if tag == "table" && self.is_readability_data_table(node) {
            return false;
        }

        // Nodes inside data tables are never removed.
        let data_tables = &self.data_tables;
        if self.has_ancestor_tag(
            node,
            "table",
            -1,
            Some(|_doc: &Document, id: NodeId| data_tables.contains(&id)),
        ) {
            return false;
        }

        // Nodes inside <code> blocks are never removed.
        if self.has_ancestor_tag::<fn(&Document, NodeId) -> bool>(node, "code", 3, None) {
            return false;
        }

        let weight = self.get_class_weight(node);
        if weight < 0 {
            return true;
        }

        // Walk the subtree to collect content statistics.
        let mut stats = CondStats::new();
        let is_video_fn = |id: NodeId| self.is_video_embed(id);
        let mut dummy_link_acc = CharCounter::new();
        for child in self.doc.child_nodes(node) {
            walk_cond(
                &self.doc,
                child,
                &mut stats,
                false,
                false,
                false,
                0.0,
                &mut dummy_link_acc,
                &is_video_fn,
            );
        }

        if stats.has_video_embed {
            return false;
        }

        let is_list = tag == "ul"
            || tag == "ol"
            || (stats.chars.total() > 0
                && stats.list_chars.total() as f64 / stats.chars.total() as f64 > 0.9);

        if stats.commas < 10 {
            // Single-text-node ad / loading word check.
            if !stats.inner_text_single.is_empty() {
                let trimmed = stats.inner_text_single.trim();
                if RX_AD_WORDS.is_match(trimmed) || RX_LOADING_WORDS.is_match(trimmed) {
                    return true;
                }
            }

            let total = stats.chars.total() as f64;
            let (text_density, link_density, heading_density) = if total > 0.0 {
                (
                    stats.text_chars.total() as f64 / total,
                    stats.link_chars_weighted / total,
                    stats.heading_chars.total() as f64 / total,
                )
            } else {
                (0.0, 0.0, 0.0)
            };

            const LI_COUNT_OFFSET: i64 = -100;

            let have_to_remove = (stats.img_count > 1
                && (stats.p_count as f64 / stats.img_count as f64) < 0.5
                && !self
                    .has_ancestor_tag::<fn(&Document, NodeId) -> bool>(node, "figure", 3, None))
                || (!is_list && (stats.li_count as i64 + LI_COUNT_OFFSET) > stats.p_count as i64)
                || ((stats.input_count as f64) > (stats.p_count as f64 / 3.0).floor())
                || (!is_list
                    && heading_density < 0.9
                    && stats.chars.total() < 25
                    && (stats.img_count == 0 || stats.img_count > 2)
                    && link_density > 0.0
                    && !self.has_ancestor_tag::<fn(&Document, NodeId) -> bool>(
                        node, "figure", 3, None,
                    ))
                || (!is_list && weight < 25 && link_density > 0.2)
                || (weight >= 25 && link_density > 0.5)
                || ((stats.embed_count == 1 && stats.chars.total() < 75) || stats.embed_count > 1)
                || (stats.img_count == 0 && text_density == 0.0);

            // Allow simple lists of images to remain.
            if is_list && have_to_remove {
                for child in self.doc.children(node) {
                    if self.doc.children(child).len() > 1 {
                        return have_to_remove;
                    }
                }
                if stats.img_count == stats.li_count {
                    return false;
                }
            }

            return have_to_remove;
        }

        false
    }

    // ── Article preparation ───────────────────────────────────────────────

    /// Port of `prepArticle` — clean article content for display.
    pub(super) fn prep_article(&mut self, article_content: NodeId) {
        self.mark_data_tables(article_content);
        self.fix_lazy_images(article_content);

        self.clean_conditionally(article_content, "form");
        self.clean_conditionally(article_content, "fieldset");
        self.clean(article_content, "object");
        self.clean(article_content, "embed");
        self.clean(article_content, "footer");
        self.clean(article_content, "link");
        self.clean(article_content, "aside");

        // Remove elements that have "share" in their class/id and are small.
        let share_threshold = self.char_threshold;
        self.remove_share_elements(article_content, share_threshold);

        self.clean(article_content, "iframe");
        self.clean(article_content, "input");
        self.clean(article_content, "textarea");
        self.clean(article_content, "select");
        self.clean(article_content, "button");
        self.clean_headers(article_content);

        // These last since prior cleaning may affect them.
        self.clean_conditionally(article_content, "table");
        self.clean_conditionally(article_content, "ul");
        self.clean_conditionally(article_content, "div");

        // Replace h1 with h2 — h1 should only appear as the title.
        let h1s = self.doc.get_elements_by_tag_name(article_content, "h1");
        for id in h1s {
            self.doc.rename_tag(id, "h2");
        }

        // Remove empty paragraphs (no meaningful content).
        let ps = self.doc.get_elements_by_tag_name(article_content, "p");
        let to_remove: Vec<NodeId> = ps
            .into_iter()
            .filter(|&p_id| !find_content_in_node(&self.doc, p_id))
            .collect();
        for id in to_remove.into_iter().rev() {
            if self.doc.parent(id).is_some() {
                self.doc.remove(id);
            }
        }

        // Remove <br> immediately before a <p>.
        let brs = self.doc.get_elements_by_tag_name(article_content, "br");
        let to_remove_brs: Vec<NodeId> = brs
            .into_iter()
            .filter(|&br_id| {
                let next_sib = self.doc.next_sibling(br_id);
                next_sib
                    .and_then(|n| self.next_node(n))
                    .map(|n| self.doc.tag_name(n) == "p")
                    .unwrap_or(false)
            })
            .collect();
        for id in to_remove_brs.into_iter().rev() {
            if self.doc.parent(id).is_some() {
                self.doc.remove(id);
            }
        }

        self.clean_styles(article_content);

        // Flatten single-cell tables.
        let tables = self.doc.get_elements_by_tag_name(article_content, "table");
        for table_id in tables {
            if self.doc.parent(table_id).is_none() {
                continue;
            }

            let tbody = if self.has_single_tag_inside_element(table_id, "tbody") {
                self.doc.first_element_child(table_id)
            } else {
                Some(table_id)
            };
            let tbody = match tbody {
                Some(t) => t,
                None => continue,
            };

            if !self.has_single_tag_inside_element(tbody, "tr") {
                continue;
            }
            let row = match self.doc.first_element_child(tbody) {
                Some(r) => r,
                None => continue,
            };

            if !self.has_single_tag_inside_element(row, "td") {
                continue;
            }
            let cell = match self.doc.first_element_child(row) {
                Some(c) => c,
                None => continue,
            };

            let new_tag = if self
                .doc
                .child_nodes(cell)
                .iter()
                .all(|&c| self.is_phrasing_content(c))
            {
                "p"
            } else {
                "div"
            };

            self.doc.rename_tag(cell, new_tag);

            // Replace the table with the cell in table's parent.
            self.doc.insert_before(table_id, cell);
            self.doc.remove(table_id);
        }
    }

    /// Remove share-element divs (elements whose class+id contains "share" and whose text is short).
    fn remove_share_elements(&mut self, node: NodeId, share_threshold: usize) {
        // Collect candidates first to avoid borrow issues.
        let children: Vec<NodeId> = self.doc.child_nodes(node);
        for child in children {
            if !self.doc.is_element(child) {
                continue;
            }
            let class = self.doc.attr(child, "class").unwrap_or("").to_string();
            let id_attr = self.doc.attr(child, "id").unwrap_or("").to_string();
            let match_string = format!("{class} {id_attr}");
            if match_string.len() > 1
                && RX_SHARE_ELEMENTS.is_match(&match_string)
                && crate::utils::char_count(&self.doc.text_content(child)) < share_threshold
            {
                if self.doc.parent(child).is_some() {
                    self.doc.remove(child);
                }
            } else {
                self.remove_share_elements(child, share_threshold);
            }
        }
    }
}
