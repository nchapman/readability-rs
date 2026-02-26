// Content scoring + grab_article + CondStats/walk_cond.

use ego_tree::NodeId;
use scraper::Node;

use super::helpers::{clone_node, last_child_node};
use super::{Parser, ALTER_TO_DIV_EXCEPTIONS, UNLIKELY_ROLES};
use crate::dom::Document;
use crate::regexp::*;
use crate::traverse::{is_comma, CharCounter};

impl Parser {
    // ── Main article extraction ───────────────────────────────────────────

    /// Port of `grabArticle` — score and select article content.
    pub(super) fn grab_article(&mut self) -> Option<NodeId> {
        // Save a pristine snapshot to restore at the start of each pass.
        let base_doc = self.doc.clone();

        loop {
            // Restore the document to a clean state for this pass.
            self.doc = base_doc.clone();
            self.score_map.clear();
            self.data_tables.clear();

            let page = self.doc.body()?;

            // ── Node prepping ─────────────────────────────────────────────
            let mut elements_to_score: Vec<NodeId> = Vec::new();
            let mut node = self.doc.document_element();
            let mut should_remove_title_header = true;

            'grab_loop: while let Some(n) = node {
                let tag = self.doc.tag_name(n).to_string();
                let class = self.doc.attr(n, "class").unwrap_or("").to_string();
                let id_attr = self.doc.attr(n, "id").unwrap_or("").to_string();
                let match_string = format!("{class} {id_attr}");

                if tag == "html" {
                    self.article_lang = self.doc.attr(n, "lang").unwrap_or("").to_string();
                }

                if !self.is_probably_visible(n) {
                    node = self.remove_and_get_next(n);
                    continue;
                }

                // Remove aria-modal="true" role="dialog" elements.
                if self.doc.attr(n, "aria-modal").unwrap_or("") == "true"
                    && self.doc.attr(n, "role").unwrap_or("") == "dialog"
                {
                    node = self.remove_and_get_next(n);
                    continue;
                }

                // Byline detection and removal.
                if self.article_byline.is_empty() && self.is_valid_byline(n, &match_string) {
                    // Look for [itemprop="name"] child for a more accurate byline.
                    let end_marker = self.get_next_node(n, true);
                    let mut next = self.get_next_node(n, false);
                    let mut found_name = false;
                    while let Some(nx) = next {
                        if end_marker.map(|e| e == nx).unwrap_or(false) {
                            break;
                        }
                        let itemprop = self.doc.attr(nx, "itemprop").unwrap_or("").to_string();
                        if itemprop.contains("name") {
                            self.article_byline = self.get_inner_text(nx, false);
                            node = self.remove_and_get_next(n);
                            found_name = true;
                            break;
                        }
                        next = self.get_next_node(nx, false);
                    }
                    if found_name {
                        continue 'grab_loop;
                    }
                    let byline_text = self.get_inner_text(n, false);
                    let n_char = crate::utils::char_count(&byline_text);
                    if n_char > 0 && n_char < 100 {
                        self.article_byline = normalize_spaces(byline_text.trim());
                        node = self.remove_and_get_next(n);
                        continue;
                    }
                }

                if should_remove_title_header && self.header_duplicates_title(n) {
                    should_remove_title_header = false;
                    node = self.remove_and_get_next(n);
                    continue;
                }

                // Remove unlikely candidates.
                if self.flags.strip_unlikelys {
                    if tag != "body"
                        && tag != "a"
                        && crate::regexp::is_unlikely_candidate(&match_string)
                        && !crate::regexp::maybe_its_a_candidate(&match_string)
                        && !self
                            .has_ancestor_tag::<fn(&Document, NodeId) -> bool>(n, "table", 3, None)
                        && !self
                            .has_ancestor_tag::<fn(&Document, NodeId) -> bool>(n, "code", 3, None)
                    {
                        node = self.remove_and_get_next(n);
                        continue;
                    }

                    let role = self.doc.attr(n, "role").unwrap_or("").to_string();
                    if UNLIKELY_ROLES.contains(&role.as_str()) {
                        node = self.remove_and_get_next(n);
                        continue;
                    }
                }

                // Remove empty structural elements.
                match tag.as_str() {
                    "div" | "section" | "header" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        if self.is_element_without_content(n) {
                            node = self.remove_and_get_next(n);
                            continue;
                        }
                    }
                    _ => {}
                }

                if self.tags_to_score.contains(&tag) {
                    elements_to_score.push(n);
                }

                // Convert divs to p where appropriate.
                if tag == "div" {
                    // Wrap inline phrasing content into <p> elements.
                    let child_nodes_snap = self.doc.child_nodes(n);
                    let mut p: Option<NodeId> = None;
                    for &child in &child_nodes_snap {
                        if self.doc.parent(child).is_none() {
                            // Already moved.
                            p = None;
                            continue;
                        }
                        if self.is_phrasing_content(child) {
                            if let Some(p_id) = p {
                                self.doc.append_child(p_id, child);
                            } else if !self.is_whitespace(child) {
                                let new_p = self.doc.create_element("p");
                                // Clone child, put clone into new_p, replace child with new_p.
                                let child_clone = clone_node(&mut self.doc, child);
                                self.doc.insert_before(child, new_p);
                                self.doc.append_child(new_p, child_clone);
                                self.doc.remove(child);
                                p = Some(new_p);
                            }
                        } else if p.is_some() {
                            // Trim trailing whitespace from p, then close it.
                            // Go: check p.NextSibling (immediate, may be a text node).
                            // Only reinsert before next if that immediate sibling is an
                            // element — otherwise just remove. This matches Go's behaviour
                            // where a text node between <p> and <ol> means "remove" rather
                            // than "move".
                            if let Some(p_id) = p {
                                loop {
                                    let last = last_child_node(&self.doc, p_id);
                                    match last {
                                        Some(l) if self.is_whitespace(l) => {
                                            // Port of Go: p.NextSibling != nil &&
                                            // p.NextSibling.Type == html.ElementNode
                                            let next_sib = self.doc.next_sibling(p_id);
                                            match next_sib {
                                                Some(ns) if self.doc.is_element(ns) => {
                                                    // Detach from p and reinsert before next sibling.
                                                    self.doc.remove(l);
                                                    self.doc.insert_before(ns, l);
                                                }
                                                _ => {
                                                    self.doc.remove(l);
                                                }
                                            }
                                        }
                                        _ => break,
                                    }
                                }
                            }
                            p = None;
                        }
                    }

                    // div with single p child → promote the p.
                    if self.has_single_tag_inside_element(n, "p") && self.get_link_density(n) < 0.25
                    {
                        let div_id = self.doc.attr(n, "id").unwrap_or("").to_string();
                        let div_class = self.doc.attr(n, "class").unwrap_or("").to_string();
                        let new_node = self.doc.children(n)[0];
                        // Replace div with its child p.
                        self.doc.insert_before(n, new_node);
                        self.doc.remove(n);
                        // Inherit id/class if the promoted node lacks them.
                        if !div_id.is_empty()
                            && self.doc.attr(new_node, "id").unwrap_or("").is_empty()
                        {
                            self.doc.set_attr(new_node, "id", &div_id);
                        }
                        if !div_class.is_empty()
                            && self.doc.attr(new_node, "class").unwrap_or("").is_empty()
                        {
                            self.doc.set_attr(new_node, "class", &div_class);
                        }
                        elements_to_score.push(new_node);
                        node = self.get_next_node(new_node, false);
                        continue;
                    } else if !self.has_child_block_element(n) {
                        self.set_node_tag(n, "p");
                        elements_to_score.push(n);
                    }
                }

                node = self.get_next_node(n, false);
            }

            // ── Scoring loop ──────────────────────────────────────────────
            let mut candidates: Vec<NodeId> = Vec::new();

            for &elem in &elements_to_score {
                if self.doc.parent(elem).is_none() {
                    continue;
                }
                let parent_tag = self
                    .doc
                    .parent(elem)
                    .map(|p| self.doc.tag_name(p).to_string())
                    .unwrap_or_default();
                if parent_tag.is_empty() {
                    continue;
                }

                let (num_chars, num_commas) =
                    crate::traverse::count_chars_and_commas(&self.doc, elem);
                if num_chars < 25 {
                    continue;
                }

                let ancestors = self.get_node_ancestors(elem, 5);
                if ancestors.is_empty() {
                    continue;
                }

                // Base score + commas + 1 + char bonus.
                let content_score =
                    1 + num_commas + 1 + (((num_chars as f64) / 100.0).floor() as usize).min(3);

                for (level, &ancestor) in ancestors.iter().enumerate() {
                    let anc_tag = self.doc.tag_name(ancestor).to_string();
                    if anc_tag.is_empty() {
                        continue;
                    }
                    if self.doc.parent(ancestor).is_none() {
                        continue;
                    }
                    // Verify parent of ancestor is an element.
                    let anc_parent_is_elem = self
                        .doc
                        .parent(ancestor)
                        .map(|p| self.doc.is_element(p))
                        .unwrap_or(false);
                    if !anc_parent_is_elem {
                        continue;
                    }

                    if !self.has_content_score(ancestor) {
                        self.initialize_node(ancestor);
                        candidates.push(ancestor);
                    }

                    let score_divider: f64 = match level {
                        0 => 1.0,
                        1 => 2.0,
                        _ => (level as f64) * 3.0,
                    };

                    let ancestor_score = self.get_content_score(ancestor);
                    self.set_content_score(
                        ancestor,
                        ancestor_score + content_score as f64 / score_divider,
                    );
                }
            }

            // Scale scores by link density.
            for &candidate in &candidates {
                let score =
                    self.get_content_score(candidate) * (1.0 - self.get_link_density(candidate));
                self.set_content_score(candidate, score);
            }

            // Sort candidates descending by score.
            candidates.sort_by(|&a, &b| {
                self.get_content_score(b)
                    .partial_cmp(&self.get_content_score(a))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let top_candidates: Vec<NodeId> = candidates
                .iter()
                .copied()
                .take(self.n_top_candidates)
                .collect();

            // ── Top candidate selection ───────────────────────────────────
            let mut top_candidate: Option<NodeId> = top_candidates.first().copied();
            let mut needed_to_create_top_candidate = false;

            if top_candidate.is_none()
                || top_candidate
                    .map(|tc| self.doc.tag_name(tc) == "body")
                    .unwrap_or(false)
            {
                // Wrap all body children in a new div.
                let new_div = self.doc.create_element("div");
                needed_to_create_top_candidate = true;
                // Move all body children into new_div.
                loop {
                    let first = self
                        .doc
                        .html
                        .tree
                        .get(page)
                        .and_then(|n| n.first_child().map(|c| c.id()));
                    match first {
                        Some(child) => self.doc.append_child(new_div, child),
                        None => break,
                    }
                }
                self.doc.append_child(page, new_div);
                self.initialize_node(new_div);
                top_candidate = Some(new_div);
            } else {
                let tc = top_candidate.expect("top_candidate set by scoring loop above");
                let top_candidate_score = self.get_content_score(tc);

                // Check for alternative ancestors shared by multiple top candidates.
                let mut alternative_ancestors: Vec<Vec<NodeId>> = Vec::new();
                for &alt in top_candidates.iter().skip(1) {
                    if self.get_content_score(alt) / top_candidate_score >= 0.75 {
                        let ancs = self.get_node_ancestors(alt, 0);
                        alternative_ancestors.push(ancs);
                    }
                }

                const MINIMUM_TOP_CANDIDATES: usize = 3;
                if alternative_ancestors.len() >= MINIMUM_TOP_CANDIDATES {
                    let mut parent_of_tc = self.doc.parent(tc);
                    'walk_up: while let Some(pot) = parent_of_tc {
                        if self.doc.tag_name(pot) == "body" {
                            break;
                        }
                        let mut count = 0;
                        for anc_list in &alternative_ancestors {
                            if anc_list.contains(&pot) {
                                count += 1;
                            }
                            if count >= MINIMUM_TOP_CANDIDATES {
                                top_candidate = Some(pot);
                                break 'walk_up;
                            }
                        }
                        parent_of_tc = self.doc.parent(pot);
                    }
                }

                let tc = top_candidate.expect("top_candidate never reset to None");
                if !self.has_content_score(tc) {
                    self.initialize_node(tc);
                }

                // Walk up the tree if score improves.
                let mut parent_of_tc = self.doc.parent(tc);
                let mut last_score = self.get_content_score(tc);
                let score_threshold = last_score / 3.0;
                while let Some(pot) = parent_of_tc {
                    if self.doc.tag_name(pot) == "body" {
                        break;
                    }
                    if !self.has_content_score(pot) {
                        parent_of_tc = self.doc.parent(pot);
                        continue;
                    }
                    let parent_score = self.get_content_score(pot);
                    if parent_score < score_threshold {
                        break;
                    }
                    if parent_score > last_score {
                        top_candidate = Some(pot);
                        break;
                    }
                    last_score = parent_score;
                    parent_of_tc = self.doc.parent(pot);
                }

                // If top candidate is the only child, use parent.
                let tc = top_candidate.expect("top_candidate never reset to None");
                let mut parent_of_tc = self.doc.parent(tc);
                while let Some(pot) = parent_of_tc {
                    if self.doc.tag_name(pot) == "body" {
                        break;
                    }
                    if self.doc.children(pot).len() != 1 {
                        break;
                    }
                    top_candidate = Some(pot);
                    parent_of_tc = self.doc.parent(pot);
                }

                let tc = top_candidate.expect("top_candidate never reset to None");
                if !self.has_content_score(tc) {
                    self.initialize_node(tc);
                }
            }

            let top_candidate = top_candidate
                .expect("if-branch creates new_div, else-branch inherits from candidates list");

            // ── Sibling gathering ─────────────────────────────────────────
            let article_content = self.doc.create_element("div");
            let sibling_score_threshold = 10.0_f64.max(self.get_content_score(top_candidate) * 0.2);
            let top_candidate_score = self.get_content_score(top_candidate);
            let top_candidate_class = self
                .doc
                .attr(top_candidate, "class")
                .unwrap_or("")
                .to_string();

            let parent_of_tc = match self.doc.parent(top_candidate) {
                Some(p) => p,
                None => {
                    // No parent — just wrap top_candidate alone.
                    self.doc.append_child(article_content, top_candidate);
                    self.prep_article(article_content);
                    return Some(article_content);
                }
            };

            let siblings = self.doc.children(parent_of_tc);
            for sibling in siblings {
                let mut append = false;

                if sibling == top_candidate {
                    append = true;
                } else {
                    let mut content_bonus = 0.0_f64;
                    let sib_class = self.doc.attr(sibling, "class").unwrap_or("").to_string();
                    if sib_class == top_candidate_class && !top_candidate_class.is_empty() {
                        content_bonus += top_candidate_score * 0.2;
                    }

                    if self.has_content_score(sibling)
                        && self.get_content_score(sibling) + content_bonus
                            >= sibling_score_threshold
                    {
                        append = true;
                    } else if self.doc.tag_name(sibling) == "p" {
                        let link_density = self.get_link_density(sibling);
                        let node_content = self.get_inner_text(sibling, true);
                        let node_length = crate::utils::char_count(&node_content);

                        append = (node_length > 80 && link_density < 0.25)
                            || (node_length < 80
                                && node_length > 0
                                && link_density == 0.0
                                && RX_SENTENCE_PERIOD.is_match(&node_content));
                    }
                }

                if append {
                    let sib_tag = self.doc.tag_name(sibling).to_string();
                    if !ALTER_TO_DIV_EXCEPTIONS.contains(&sib_tag.as_str()) {
                        self.doc.rename_tag(sibling, "div");
                    }
                    self.doc.append_child(article_content, sibling);
                }
            }

            // ── Prep and wrap ─────────────────────────────────────────────
            self.prep_article(article_content);

            if needed_to_create_top_candidate {
                // The fake div was already moved into article_content.
                // Find it (should be the first div child) and tag it.
                let first_child = self.doc.first_element_child(article_content);
                if let Some(fc) = first_child {
                    if self.doc.tag_name(fc) == "div" {
                        self.doc.set_attr(fc, "id", "readability-page-1");
                        self.doc.set_attr(fc, "class", "page");
                    }
                }
            } else {
                let page_div = self.doc.create_element("div");
                self.doc.set_attr(page_div, "id", "readability-page-1");
                self.doc.set_attr(page_div, "class", "page");
                // Move all children of article_content into page_div.
                loop {
                    let first = self
                        .doc
                        .html
                        .tree
                        .get(article_content)
                        .and_then(|n| n.first_child().map(|c| c.id()));
                    match first {
                        Some(child) => self.doc.append_child(page_div, child),
                        None => break,
                    }
                }
                self.doc.append_child(article_content, page_div);
            }

            // ── Length check and flag cycling ─────────────────────────────
            let (text_length, _) =
                crate::traverse::count_chars_and_commas(&self.doc, article_content);

            if text_length < self.char_threshold {
                let doc_snap = self.doc.clone();
                self.attempts.push(super::ParseAttempt {
                    article_content,
                    doc_snapshot: doc_snap,
                    text_length,
                });

                if self.flags.strip_unlikelys {
                    self.flags.strip_unlikelys = false;
                } else if self.flags.use_weight_classes {
                    self.flags.use_weight_classes = false;
                } else if self.flags.clean_conditionally {
                    self.flags.clean_conditionally = false;
                } else {
                    // All flags exhausted — use the attempt with the most text.
                    self.attempts
                        .sort_by(|a, b| b.text_length.cmp(&a.text_length));
                    if self.attempts[0].text_length == 0 {
                        return None;
                    }
                    let best_content = self.attempts[0].article_content;
                    self.doc = self.attempts[0].doc_snapshot.clone();
                    return Some(best_content);
                }
                // Try next pass.
                continue;
            }

            return Some(article_content);
        }
    }
}

// ── CondStats and walk_cond (used by clean_conditionally) ────────────────────

/// Accumulates statistics for `should_clean_conditionally`.
pub(super) struct CondStats {
    pub(super) chars: CharCounter,
    pub(super) text_chars: CharCounter,
    pub(super) list_chars: CharCounter,
    pub(super) heading_chars: CharCounter,
    pub(super) link_chars_weighted: f64,
    pub(super) commas: usize,
    pub(super) p_count: usize,
    pub(super) img_count: usize,
    pub(super) li_count: usize,
    pub(super) input_count: usize,
    pub(super) embed_count: usize,
    pub(super) has_video_embed: bool,
    pub(super) inner_text_single: String,
}

impl CondStats {
    pub(super) fn new() -> Self {
        CondStats {
            chars: CharCounter::new(),
            text_chars: CharCounter::new(),
            list_chars: CharCounter::new(),
            heading_chars: CharCounter::new(),
            link_chars_weighted: 0.0,
            commas: 0,
            p_count: 0,
            img_count: 0,
            li_count: 0,
            input_count: 0,
            embed_count: 0,
            has_video_embed: false,
            inner_text_single: String::new(),
        }
    }
}

/// Recursive walker for `should_clean_conditionally`.
///
/// Mirrors Go's `walk` closure in `cleanConditionally`: accumulates char counts
/// per element class (text/list/heading) and per-`<a>` link counts.
///
/// `link_coeff` is non-zero only when we are inside an `<a>` element.
/// `link_acc` accumulates chars that belong to the current `<a>` (if any).
#[allow(clippy::too_many_arguments)]
pub(super) fn walk_cond(
    doc: &Document,
    n: NodeId,
    stats: &mut CondStats,
    in_text: bool,
    in_list: bool,
    in_heading: bool,
    link_coeff: f64,
    link_acc: &mut CharCounter,
    is_video_fn: &dyn Fn(NodeId) -> bool,
) {
    match doc.html.tree.get(n).map(|x| x.value()) {
        Some(Node::Text(text)) => {
            let old_total = stats.chars.total();
            for r in text.text.chars() {
                stats.chars.count(r);
                if is_comma(r) {
                    stats.commas += 1;
                }
                if in_text {
                    stats.text_chars.count(r);
                }
                if in_list {
                    stats.list_chars.count(r);
                }
                if in_heading {
                    stats.heading_chars.count(r);
                }
                if link_coeff != 0.0 {
                    link_acc.count(r);
                }
            }
            if stats.chars.total() > old_total {
                stats.inner_text_single = text.text.to_string();
            }
        }
        Some(Node::Element(_)) => {
            let tag = doc.tag_name(n);
            match tag {
                "p" => stats.p_count += 1,
                "img" => stats.img_count += 1,
                "li" => stats.li_count += 1,
                "input" => stats.input_count += 1,
                "object" | "embed" | "iframe" => {
                    stats.embed_count += 1;
                    if is_video_fn(n) {
                        stats.has_video_embed = true;
                    }
                }
                _ => {}
            }

            // Go's walk resets context unconditionally on every entry into these element
            // types, even when already inside a nested one (e.g., a ul inside a ul). This
            // matches Go's ResetContext() call which is outside any "already in list" guard.
            let new_in_list = in_list || matches!(tag, "ul" | "ol");
            if matches!(tag, "ul" | "ol") {
                stats.list_chars.reset_context();
            }

            let new_in_heading =
                in_heading || matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
            if matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                stats.heading_chars.reset_context();
            }

            let new_in_text = in_text
                || matches!(
                    tag,
                    "blockquote"
                        | "dl"
                        | "div"
                        | "img"
                        | "ol"
                        | "p"
                        | "pre"
                        | "table"
                        | "ul"
                        | "span"
                        | "li"
                        | "td"
                );
            if matches!(
                tag,
                "blockquote"
                    | "dl"
                    | "div"
                    | "img"
                    | "ol"
                    | "p"
                    | "pre"
                    | "table"
                    | "ul"
                    | "span"
                    | "li"
                    | "td"
            ) {
                stats.text_chars.reset_context();
            }

            if tag == "a" {
                // Each <a> gets its own fresh link counter (port of Go's per-a cc).
                // Mirror Go: coefficient is 0 when the <a>'s DIRECT parent is a figcaption.
                let parent_tag = doc.parent(n).map(|p| doc.tag_name(p)).unwrap_or("");
                let coeff = if parent_tag != "figcaption" {
                    let href = doc.attr(n, "href").unwrap_or("").trim().to_string();
                    if href.len() > 1 && href.starts_with('#') {
                        0.3
                    } else {
                        1.0
                    }
                } else {
                    0.0
                };
                let mut my_acc = CharCounter::new();
                for child in doc.child_nodes(n) {
                    walk_cond(
                        doc,
                        child,
                        stats,
                        new_in_text,
                        new_in_list,
                        new_in_heading,
                        coeff,
                        &mut my_acc,
                        is_video_fn,
                    );
                }
                stats.link_chars_weighted += my_acc.total() as f64 * coeff;
            } else {
                for child in doc.child_nodes(n) {
                    walk_cond(
                        doc,
                        child,
                        stats,
                        new_in_text,
                        new_in_list,
                        new_in_heading,
                        link_coeff,
                        link_acc,
                        is_video_fn,
                    );
                }
            }
        }
        _ => {}
    }
}
