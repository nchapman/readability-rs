// Node iteration helpers, DOM traversal helpers, and free helper functions.

use std::collections::HashSet;

use ego_tree::NodeId;
use scraper::Node;

use super::{Parser, DIV_TO_P_ELEMS, MAX_TREE_DEPTH, PHRASING_ELEMS};
use crate::dom::Document;
use crate::regexp::*;
use crate::traverse::has_text_content;
use crate::utils::text_similarity;

impl Parser {
    // ── Node iteration helpers ────────────────────────────────────────────

    /// Port of `removeNodes` — remove nodes (optionally filtered) from the tree.
    ///
    /// Iterates backwards to allow safe removal during iteration.
    /// If `filter` is `None`, removes all nodes; if `Some(f)`, removes those where `f` returns true.
    pub(super) fn remove_nodes<F>(&mut self, nodes: Vec<NodeId>, filter: Option<F>)
    where
        F: Fn(&Document, NodeId) -> bool,
    {
        for id in nodes.into_iter().rev() {
            if self.doc.parent(id).is_none() {
                continue;
            }
            let should_remove = match &filter {
                None => true,
                Some(f) => f(&self.doc, id),
            };
            if should_remove {
                self.doc.remove(id);
            }
        }
    }

    /// Port of `replaceNodeTags` — rename all nodes in the list to `new_tag`.
    pub(super) fn replace_node_tags(&mut self, nodes: Vec<NodeId>, new_tag: &str) {
        for id in nodes.into_iter().rev() {
            self.doc.rename_tag(id, new_tag);
        }
    }

    // ── DOM traversal helpers ─────────────────────────────────────────────

    /// Port of `getElementByTagName` — first descendant element with the given tag (DFS).
    pub(super) fn get_element_by_tag_name(&self, id: NodeId, tag: &str) -> Option<NodeId> {
        // get_elements_by_tag_name returns all matches; take the first.
        self.doc
            .get_elements_by_tag_name(id, tag)
            .into_iter()
            .next()
    }

    /// Port of `getInnerText` — text content, optionally whitespace-normalized.
    pub(super) fn get_inner_text(&self, id: NodeId, normalize: bool) -> String {
        let text = self.doc.text_content(id);
        if normalize {
            normalize_spaces(text.trim())
        } else {
            text.trim().to_string()
        }
    }

    /// Port of `isWhitespace` — true if the node is purely whitespace.
    pub(super) fn is_whitespace(&self, id: NodeId) -> bool {
        match self.doc.html.tree.get(id).map(|n| n.value()) {
            Some(Node::Text(text)) => {
                !has_text_content(&self.doc, id) && text.text.trim().is_empty()
            }
            Some(Node::Element(_)) => self.doc.tag_name(id) == "br",
            _ => false,
        }
    }

    /// Port of `isPhrasingContent` — true if the node qualifies as phrasing content.
    pub(super) fn is_phrasing_content(&self, id: NodeId) -> bool {
        let tag = self.doc.tag_name(id);
        if self.doc.is_text_node(id) {
            return true;
        }
        if PHRASING_ELEMS.contains(&tag) {
            return true;
        }
        if (tag == "a" || tag == "del" || tag == "ins")
            && self
                .doc
                .child_nodes(id)
                .iter()
                .all(|&c| self.is_phrasing_content(c))
        {
            return true;
        }
        false
    }

    /// Port of `nextNode` — advance past whitespace-only nodes.
    ///
    /// Starting at `id`, returns the first sibling (or `id` itself) that is either
    /// an element node or a non-whitespace text node.
    pub(super) fn next_node(&self, id: NodeId) -> Option<NodeId> {
        let mut cur = Some(id);
        while let Some(n) = cur {
            let is_element = self.doc.is_element(n);
            let has_text = has_text_content(&self.doc, n);
            if is_element || has_text {
                return Some(n);
            }
            cur = self.doc.next_sibling(n);
        }
        None
    }

    /// Port of `getNextNode` — depth-first traversal step.
    ///
    /// If `ignore_self_and_kids` is true, skip this node's children (used when removing).
    pub(super) fn get_next_node(&self, id: NodeId, ignore_self_and_kids: bool) -> Option<NodeId> {
        // Descend into first child unless we're skipping.
        if !ignore_self_and_kids {
            if let Some(child) = self.doc.first_element_child(id) {
                return Some(child);
            }
        }
        // Try next sibling.
        if let Some(sibling) = self.doc.next_element_sibling(id) {
            return Some(sibling);
        }
        // Walk up until we find a parent with a next sibling.
        let mut cur = id;
        loop {
            match self.doc.parent(cur) {
                None => return None,
                Some(p) => {
                    if let Some(sibling) = self.doc.next_element_sibling(p) {
                        return Some(sibling);
                    }
                    cur = p;
                }
            }
        }
    }

    /// Port of `removeAndGetNext` — remove a node and return its traversal successor.
    pub(super) fn remove_and_get_next(&mut self, id: NodeId) -> Option<NodeId> {
        let next = self.get_next_node(id, true);
        self.doc.remove(id);
        next
    }

    /// Port of `isElementWithoutContent` — true if node is an element with no meaningful content.
    pub(super) fn is_element_without_content(&self, id: NodeId) -> bool {
        if !self.doc.is_element(id) {
            return false;
        }
        for child_id in self.doc.child_nodes(id) {
            match self.doc.html.tree.get(child_id).map(|n| n.value()) {
                Some(Node::Text(t)) => {
                    if crate::utils::has_content(&t.text) {
                        return false;
                    }
                }
                Some(Node::Element(_)) => {
                    let tag = self.doc.tag_name(child_id);
                    if tag != "br" && tag != "hr" {
                        return false;
                    }
                }
                _ => {}
            }
        }
        true
    }

    /// Port of `hasSingleTagInsideElement` — true if element has exactly one element child
    /// with the given tag, and no non-whitespace text nodes.
    pub(super) fn has_single_tag_inside_element(&self, id: NodeId, tag: &str) -> bool {
        let children = self.doc.children(id);
        if children.len() != 1 || self.doc.tag_name(children[0]) != tag {
            return false;
        }
        // Must have no non-whitespace text nodes among all child nodes.
        !self.doc.child_nodes(id).iter().any(|&c| {
            if let Some(Node::Text(t)) = self.doc.html.tree.get(c).map(|n| n.value()) {
                RX_HAS_CONTENT.is_match(&t.text)
            } else {
                false
            }
        })
    }

    /// Port of `hasChildBlockElement` — true if any child is a block-level element.
    pub(super) fn has_child_block_element(&self, id: NodeId) -> bool {
        self.has_child_block_element_inner(id, 0)
    }

    fn has_child_block_element_inner(&self, id: NodeId, depth: usize) -> bool {
        if depth >= MAX_TREE_DEPTH {
            return false;
        }
        self.doc.child_nodes(id).iter().any(|&c| {
            let tag = self.doc.tag_name(c);
            DIV_TO_P_ELEMS.contains(&tag) || self.has_child_block_element_inner(c, depth + 1)
        })
    }

    // ── Scoring / classification helpers ────────────────────────────────

    /// Port of `isProbablyVisible` — true if the node is not hidden.
    pub(super) fn is_probably_visible(&self, id: NodeId) -> bool {
        let style = self.doc.attr(id, "style").unwrap_or("");
        let aria_hidden = self.doc.attr(id, "aria-hidden").unwrap_or("");
        let class = self.doc.attr(id, "class").unwrap_or("");

        (style.is_empty() || !RX_DISPLAY_NONE.is_match(style))
            && (style.is_empty() || !RX_VISIBILITY_HIDDEN.is_match(style))
            && !self.doc.has_attribute(id, "hidden")
            && (aria_hidden.is_empty() || aria_hidden != "true" || class.contains("fallback-image"))
    }

    /// Port of `isValidByline` — true if the node looks like a byline.
    pub(super) fn is_valid_byline(&self, id: NodeId, match_string: &str) -> bool {
        let rel = self.doc.attr(id, "rel").unwrap_or("");
        let itemprop = self.doc.attr(id, "itemprop").unwrap_or("");
        rel == "author" || itemprop.contains("author") || crate::regexp::is_byline(match_string)
    }

    /// Port of `headerDuplicatesTitle` — true if the node is an h1/h2 whose text is
    /// very similar to the article title.
    pub(super) fn header_duplicates_title(&self, id: NodeId) -> bool {
        let tag = self.doc.tag_name(id);
        if tag != "h1" && tag != "h2" {
            return false;
        }
        let heading = self.get_inner_text(id, false);
        text_similarity(&self.article_title, &heading) > 0.75
    }

    /// Port of `getClassWeight` — score bonus/penalty from class/id names.
    ///
    /// Returns 0 when `use_weight_classes` is false.
    pub(super) fn get_class_weight(&self, id: NodeId) -> i32 {
        if !self.flags.use_weight_classes {
            return 0;
        }
        let mut weight = 0i32;
        if let Some(cls) = self.doc.attr(id, "class") {
            if crate::regexp::is_negative_class(cls) {
                weight -= 25;
            }
            if crate::regexp::is_positive_class(cls) {
                weight += 25;
            }
        }
        if let Some(id_attr) = self.doc.attr(id, "id") {
            if crate::regexp::is_negative_class(id_attr) {
                weight -= 25;
            }
            if crate::regexp::is_positive_class(id_attr) {
                weight += 25;
            }
        }
        weight
    }

    /// Port of `getLinkDensityCoefficient` — hash-only links are weighted lower.
    pub(super) fn get_link_density_coefficient(doc: &Document, a: NodeId) -> f64 {
        let href = doc.attr(a, "href").unwrap_or("").trim().to_string();
        if href.len() > 1 && href.starts_with('#') {
            0.3
        } else {
            1.0
        }
    }

    /// Port of `getLinkDensity` — ratio of link chars to total chars in the node.
    pub(super) fn get_link_density(&self, id: NodeId) -> f64 {
        let mut total = crate::traverse::CharCounter::new();
        let mut link_weighted: f64 = 0.0;

        fn walk(
            doc: &Document,
            n: NodeId,
            link_counter: &mut Option<(crate::traverse::CharCounter, f64)>,
            total: &mut crate::traverse::CharCounter,
            link_weighted: &mut f64,
        ) {
            if let Some(Node::Text(text)) = doc.html.tree.get(n).map(|x| x.value()) {
                for r in text.text.chars() {
                    total.count(r);
                    if let Some((ref mut lc, _coeff)) = link_counter {
                        lc.count(r);
                    }
                }
                return;
            }
            let tag = doc.tag_name(n);
            if tag == "a" {
                let coeff = Parser::get_link_density_coefficient(doc, n);
                let mut my_counter: Option<(crate::traverse::CharCounter, f64)> =
                    Some((crate::traverse::CharCounter::new(), coeff));
                for child in doc.child_nodes(n) {
                    walk(doc, child, &mut my_counter, total, link_weighted);
                }
                if let Some((lc, c)) = my_counter {
                    *link_weighted += lc.total() as f64 * c;
                }
            } else {
                for child in doc.child_nodes(n) {
                    walk(doc, child, link_counter, total, link_weighted);
                }
            }
        }

        walk(&self.doc, id, &mut None, &mut total, &mut link_weighted);

        if total.total() == 0 {
            0.0
        } else {
            link_weighted / total.total() as f64
        }
    }

    /// Port of `hasAncestorTag` — true if any ancestor (up to `max_depth`) has the given tag.
    ///
    /// `max_depth <= 0` means no limit.
    pub(super) fn has_ancestor_tag<F>(
        &self,
        id: NodeId,
        tag: &str,
        max_depth: i32,
        filter: Option<F>,
    ) -> bool
    where
        F: Fn(&Document, NodeId) -> bool,
    {
        let mut depth = 0;
        let mut cur = id;
        while let Some(parent) = self.doc.parent(cur) {
            if max_depth > 0 && depth > max_depth {
                return false;
            }
            if self.doc.tag_name(parent) == tag
                && filter
                    .as_ref()
                    .map(|f| f(&self.doc, parent))
                    .unwrap_or(true)
            {
                return true;
            }
            cur = parent;
            depth += 1;
        }
        false
    }

    /// Port of `getNodeAncestors` — collect ancestors up to `max_depth` (0 = unlimited).
    pub(super) fn get_node_ancestors(&self, id: NodeId, max_depth: usize) -> Vec<NodeId> {
        let mut result = Vec::new();
        let mut cur = id;
        while let Some(parent) = self.doc.parent(cur) {
            result.push(parent);
            if max_depth > 0 && result.len() == max_depth {
                break;
            }
            cur = parent;
        }
        result
    }

    // ── Score / table side-table accessors ───────────────────────────────

    /// Store a content score for `id` in the per-pass side table.
    pub(super) fn set_content_score(&mut self, id: NodeId, score: f64) {
        self.score_map.insert(id, score);
    }

    /// Get the content score for `id` (0.0 if not scored).
    pub(super) fn get_content_score(&self, id: NodeId) -> f64 {
        self.score_map.get(&id).copied().unwrap_or(0.0)
    }

    /// True if `id` has been scored.
    pub(super) fn has_content_score(&self, id: NodeId) -> bool {
        self.score_map.contains_key(&id)
    }

    /// Mark or unmark `id` as a data (non-layout) table.
    pub(super) fn set_readability_data_table(&mut self, id: NodeId, is_data: bool) {
        if is_data {
            self.data_tables.insert(id);
        } else {
            self.data_tables.remove(&id);
        }
    }

    /// True if `id` has been marked as a data table.
    pub(super) fn is_readability_data_table(&self, id: NodeId) -> bool {
        self.data_tables.contains(&id)
    }

    // ── Node initialization ───────────────────────────────────────────────

    /// Port of `initializeNode` — set initial content score from tag name and class weight.
    pub(super) fn initialize_node(&mut self, id: NodeId) {
        let class_weight = self.get_class_weight(id) as f64;
        let tag_score: f64 = match self.doc.tag_name(id) {
            "div" => 5.0,
            "pre" | "td" | "blockquote" => 3.0,
            "address" | "ol" | "ul" | "dl" | "dd" | "dt" | "li" | "form" => -3.0,
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "th" => -5.0,
            _ => 0.0,
        };
        self.set_content_score(id, class_weight + tag_score);
    }

    // ── Video embed detection ─────────────────────────────────────────────

    /// Port of `isVideoEmbed` — true if the element is an embedded video.
    pub(super) fn is_video_embed(&self, id: NodeId) -> bool {
        let tag = self.doc.tag_name(id);
        if tag != "object" && tag != "embed" && tag != "iframe" {
            return false;
        }
        let rx = self.allowed_video_regex.as_ref().unwrap_or(&*RX_VIDEOS);
        for (_, val) in self.doc.get_all_attrs(id) {
            if rx.is_match(&val) {
                return true;
            }
        }
        if tag == "object" {
            let inner = self.doc.inner_html(id);
            if rx.is_match(&inner) {
                return true;
            }
        }
        false
    }

    // ── Table analysis ────────────────────────────────────────────────────

    /// Port of `getRowAndColumnCount` — count rows and max columns in a table.
    pub(super) fn get_row_and_column_count(&self, table: NodeId) -> (usize, usize) {
        let mut rows: usize = 0;
        let mut columns: usize = 0;
        let trs = self.doc.get_elements_by_tag_name(table, "tr");
        for tr in &trs {
            let row_span: usize = self
                .doc
                .attr(*tr, "rowspan")
                .and_then(|s| s.parse().ok())
                .filter(|&v| v > 0)
                .unwrap_or(1);
            rows += row_span;

            let mut cols_in_row: usize = 0;
            let cells = self.doc.get_elements_by_tag_name(*tr, "td");
            for cell in &cells {
                let col_span: usize = self
                    .doc
                    .attr(*cell, "colspan")
                    .and_then(|s| s.parse().ok())
                    .filter(|&v| v > 0)
                    .unwrap_or(1);
                cols_in_row += col_span;
            }
            if cols_in_row > columns {
                columns = cols_in_row;
            }
        }
        (rows, columns)
    }

    /// Port of `markDataTables` — classify each `<table>` as data or layout.
    pub(super) fn mark_data_tables(&mut self, root: NodeId) {
        let tables = self.doc.get_elements_by_tag_name(root, "table");
        for table in tables {
            // If parent was removed (e.g. nested within a removed table), skip.
            if self.doc.parent(table).is_none() {
                continue;
            }

            // role="presentation" → layout table.
            if self.doc.attr(table, "role").unwrap_or("") == "presentation" {
                self.set_readability_data_table(table, false);
                continue;
            }

            // datatable="0" → layout table.
            if self.doc.attr(table, "datatable").unwrap_or("") == "0" {
                self.set_readability_data_table(table, false);
                continue;
            }

            // summary attribute → data table.
            if self.doc.has_attribute(table, "summary") {
                self.set_readability_data_table(table, true);
                continue;
            }

            // Scan children for structural indicators.
            let (is_data, conclusive) = scan_for_data_table_signals(&self.doc, table);
            if conclusive {
                self.set_readability_data_table(table, is_data);
                continue;
            }

            // Fall back to row/column heuristics.
            let (rows, cols) = self.get_row_and_column_count(table);
            if rows == 1 || cols == 1 {
                self.set_readability_data_table(table, false);
                continue;
            }
            if rows >= 10 || cols > 4 {
                self.set_readability_data_table(table, true);
                continue;
            }
            if rows * cols > 10 {
                self.set_readability_data_table(table, true);
            }
        }
    }

    /// Port of `setNodeTag` — rename a node's tag in place (NodeId stays valid).
    pub(super) fn set_node_tag(&mut self, id: NodeId, new_tag: &str) {
        self.doc.rename_tag(id, new_tag);
    }

    /// Advance past text nodes that are whitespace-only; return the next non-whitespace-text or element node.
    pub(super) fn advance_past_whitespace_siblings(&self, start: Option<NodeId>) -> Option<NodeId> {
        let mut cur = start;
        while let Some(n) = cur {
            let is_elem = self.doc.is_element(n);
            if is_elem {
                return Some(n);
            }
            if has_text_content(&self.doc, n) {
                return Some(n);
            }
            cur = self
                .doc
                .html
                .tree
                .get(n)
                .and_then(|x| x.next_sibling().map(|s| s.id()));
        }
        None
    }

    // ── Clean classes ──────────────────────────────────────────────────────

    /// Port of `cleanClasses` — strip class attributes, preserving `classes_to_preserve`.
    pub(super) fn clean_classes(&mut self, id: NodeId) {
        let preserve: HashSet<String> = self.classes_to_preserve.iter().cloned().collect();
        self.clean_classes_impl(id, &preserve, 0);
    }

    fn clean_classes_impl(&mut self, id: NodeId, preserve: &HashSet<String>, depth: usize) {
        if depth >= MAX_TREE_DEPTH {
            return;
        }
        if self.doc.is_element(id) {
            if let Some(cls) = self.doc.attr(id, "class") {
                let kept: Vec<&str> = cls
                    .split_whitespace()
                    .filter(|c| preserve.contains(*c))
                    .collect();
                if kept.is_empty() {
                    self.doc.remove_attr(id, "class");
                } else {
                    let new_cls = kept.join(" ");
                    self.doc.set_attr(id, "class", &new_cls);
                }
            }
        }
        for child in self.doc.child_nodes(id) {
            self.clean_classes_impl(child, preserve, depth + 1);
        }
    }
}

// ── Free helpers (not methods — may operate on a foreign Document) ────────────

/// Scan direct children (and their descendants) for data-table structural signals.
///
/// Returns `(is_data_table, is_conclusive)`.
pub(super) fn scan_for_data_table_signals(doc: &Document, n: NodeId) -> (bool, bool) {
    let Some(node) = doc.html.tree.get(n) else {
        return (false, false);
    };
    for child in node.children() {
        if let Node::Element(el) = child.value() {
            match el.name() {
                "col" | "colgroup" | "tfoot" | "thead" | "th" => {
                    return (true, true);
                }
                "caption" => {
                    if child.has_children() {
                        return (true, true);
                    }
                }
                "table" => {
                    return (false, true);
                }
                _ => {}
            }
        }
        let (result, conclusive) = scan_for_data_table_signals(doc, child.id());
        if conclusive {
            return (result, conclusive);
        }
    }
    (false, false)
}

/// Return the last child of any type (text, element, comment…).
pub(super) fn last_child_node(doc: &Document, id: NodeId) -> Option<NodeId> {
    doc.html.tree.get(id)?.last_child().map(|n| n.id())
}

/// Deep-clone a node into the same tree and return the new NodeId.
///
/// Handles element nodes (recursively cloning children) and text nodes.
pub(super) fn clone_node(doc: &mut Document, id: NodeId) -> NodeId {
    match doc.html.tree.get(id).map(|n| n.value().clone()) {
        Some(Node::Element(el)) => {
            let new_name = el.name.clone();
            let attrs: Vec<_> = el
                .attrs
                .iter()
                .map(|(k, v)| html5ever::Attribute {
                    name: k.clone(),
                    value: v.clone(),
                })
                .collect();
            let new_el = scraper::node::Element::new(new_name, attrs);
            let new_id = doc.html.tree.orphan(Node::Element(new_el)).id();
            // Clone children.
            let children: Vec<NodeId> = doc.child_nodes(id);
            for child in children {
                let child_clone = clone_node(doc, child);
                doc.append_child(new_id, child_clone);
            }
            new_id
        }
        Some(Node::Text(t)) => {
            let text_val = t.text.as_ref().to_string();
            doc.create_text_node(&text_val)
        }
        _ => {
            // For other node types (comments, etc.), create an empty text node as a fallback.
            doc.create_text_node("")
        }
    }
}

/// True if this node or any descendant has useful content (images, embeds, or text).
///
/// Port of the inline `findContent` closure in Go's `prepArticle`.
pub(super) fn find_content_in_node(doc: &Document, id: NodeId) -> bool {
    let Some(node) = doc.html.tree.get(id) else {
        return false;
    };
    match node.value() {
        Node::Element(el) => match el.name() {
            "img" | "picture" | "embed" | "object" | "iframe" => return true,
            _ => {}
        },
        Node::Text(_) => {
            if has_text_content(doc, id) {
                return true;
            }
        }
        _ => {}
    }
    for child in node.children() {
        if find_content_in_node(doc, child.id()) {
            return true;
        }
    }
    false
}

/// Port of `isSingleImage` as a free function operating on any Document.
pub(super) fn is_single_image_in(doc: &Document, id: NodeId) -> bool {
    if doc.tag_name(id) == "img" {
        return true;
    }
    let children = doc.children(id);
    if children.len() != 1 || has_text_content(doc, id) {
        return false;
    }
    is_single_image_in(doc, children[0])
}
