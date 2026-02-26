// Document preparation: remove comments/scripts, replace <br> chains, fix lazy images,
// unwrap noscript images.

use ego_tree::NodeId;
use scraper::Node;

use super::helpers::is_single_image_in;
use super::Parser;
use crate::dom::Document;
use crate::regexp::*;
use crate::utils::is_valid_url;

impl Parser {
    // ── Document preparation ──────────────────────────────────────────────

    /// Port of `removeComments` — remove all HTML comment nodes.
    pub(super) fn remove_comments(&mut self) {
        let root = self.doc.root();
        self.remove_comments_from(root, 0);
    }

    fn remove_comments_from(&mut self, id: NodeId, depth: usize) {
        if depth >= super::MAX_TREE_DEPTH {
            return;
        }
        let children: Vec<NodeId> = self.doc.child_nodes(id);
        for child in children {
            if matches!(
                self.doc.html.tree.get(child).map(|n| n.value()),
                Some(Node::Comment(_))
            ) {
                self.doc.remove(child);
            } else {
                self.remove_comments_from(child, depth + 1);
            }
        }
    }

    /// Port of `removeScripts` — remove all `<script>` and `<noscript>` elements.
    pub(super) fn remove_scripts(&mut self) {
        let root = self.doc.root();
        let targets = self
            .doc
            .get_all_nodes_with_tag(root, &["script", "noscript"]);
        self.remove_nodes(targets, None::<fn(&Document, NodeId) -> bool>);
    }

    /// Port of `prepDocument` — remove comments, styles, replace `<br>` chains, `<font>` → `<span>`.
    pub(super) fn prep_document(&mut self) {
        self.remove_comments();

        let root = self.doc.root();
        let styles = self.doc.get_elements_by_tag_name(root, "style");
        self.remove_nodes(styles, None::<fn(&Document, NodeId) -> bool>);

        if let Some(body) = self.doc.body() {
            self.replace_brs(body);
        }

        let root = self.doc.root();
        let fonts = self.doc.get_elements_by_tag_name(root, "font");
        self.replace_node_tags(fonts, "span");
    }

    /// Port of `replaceBrs` — replace runs of 2+ `<br>` with `<p>` elements.
    fn replace_brs(&mut self, elem: NodeId) {
        self.replace_brs_finder(elem);
    }

    fn replace_brs_finder(&mut self, n: NodeId) {
        // Get the first child (any type) by using ego_tree's native traversal.
        let first = self
            .doc
            .html
            .tree
            .get(n)
            .and_then(|x| x.first_child().map(|c| c.id()));
        let mut cur = first;

        while let Some(child) = cur {
            // Capture next sibling before any mutations.
            let next_sib = self
                .doc
                .html
                .tree
                .get(child)
                .and_then(|x| x.next_sibling().map(|s| s.id()));
            let tag = self.doc.tag_name(child).to_string();

            if tag == "pre" {
                cur = next_sib;
                continue;
            }
            if tag == "br" {
                let new_node = self.replace_br(child);
                // Continue from after the new node.
                cur = self
                    .doc
                    .html
                    .tree
                    .get(new_node)
                    .and_then(|x| x.next_sibling().map(|s| s.id()));
                continue;
            }
            if !tag.is_empty() {
                // Element (not pre/br): recurse.
                self.replace_brs_finder(child);
            }
            cur = next_sib;
        }
    }

    /// Replace a single `<br>` with a `<p>` if it's part of a chain of 2+ `<br>`s.
    ///
    /// Returns the original `br` NodeId if no replacement happened, or the new `<p>` NodeId.
    fn replace_br(&mut self, br: NodeId) -> NodeId {
        // Collect the chain: skip whitespace-only nodes; stop at non-<br> elements.
        let mut next = self
            .doc
            .html
            .tree
            .get(br)
            .and_then(|x| x.next_sibling().map(|s| s.id()));
        let mut replaced = false;

        loop {
            // Skip whitespace-only nodes.
            let advanced = self.advance_past_whitespace_siblings(next);
            next = advanced;
            let Some(n) = next else { break };
            if self.doc.tag_name(n) != "br" {
                break;
            }
            replaced = true;
            let after = self
                .doc
                .html
                .tree
                .get(n)
                .and_then(|x| x.next_sibling().map(|s| s.id()));
            self.doc.remove(n);
            next = after;
        }

        if !replaced {
            return br;
        }

        // Replace the first `<br>` with a new `<p>`.
        let p = self.doc.create_element("p");
        let br_parent = self.doc.parent(br).unwrap_or_else(|| self.doc.root());
        self.doc.insert_before(br, p);
        self.doc.remove(br);

        // Absorb phrasing-content siblings into the new `<p>`.
        let mut sib = self
            .doc
            .html
            .tree
            .get(p)
            .and_then(|x| x.next_sibling().map(|s| s.id()));
        while let Some(s) = sib {
            // Stop at a second `<br>` run.
            if self.doc.tag_name(s) == "br" {
                let nxt = self
                    .doc
                    .html
                    .tree
                    .get(s)
                    .and_then(|x| x.next_sibling().map(|s2| s2.id()));
                let next_elem = nxt.and_then(|n| self.advance_past_whitespace_siblings(Some(n)));
                if next_elem
                    .map(|ne| self.doc.tag_name(ne) == "br")
                    .unwrap_or(false)
                {
                    break;
                }
            }
            if !self.is_phrasing_content(s) {
                break;
            }
            let after = self
                .doc
                .html
                .tree
                .get(s)
                .and_then(|x| x.next_sibling().map(|s2| s2.id()));
            self.doc.append_child(p, s);
            sib = after;
        }

        // Trim trailing whitespace from the new `<p>`.
        loop {
            let last = self
                .doc
                .html
                .tree
                .get(p)
                .and_then(|x| x.last_child().map(|c| c.id()));
            match last {
                None => break,
                Some(l) if self.is_whitespace(l) => {
                    self.doc.remove(l);
                }
                _ => break,
            }
        }

        // If `<p>` ended up inside another `<p>`, promote parent to `<div>`.
        if let Some(parent_p) = self.doc.parent(p) {
            if self.doc.tag_name(parent_p) == "p" {
                self.set_node_tag(parent_p, "div");
            }
        }

        let _ = br_parent; // used earlier for context
        p
    }

    // ── Image handling ────────────────────────────────────────────────────

    /// Port of `fixLazyImages` — convert data-src / lazy-loaded images to real src attrs.
    pub(super) fn fix_lazy_images(&mut self, root: NodeId) {
        let nodes = self
            .doc
            .get_all_nodes_with_tag(root, &["img", "picture", "figure"]);
        for elem in nodes {
            let src = self.doc.attr(elem, "src").unwrap_or("").to_string();
            let tag = self.doc.tag_name(elem).to_string();
            let class = self.doc.attr(elem, "class").unwrap_or("").to_string();

            // Remove tiny base64 placeholders if another attribute has the real image.
            if !src.is_empty() && RX_B64_DATA_URL.is_match(&src) {
                let mime = RX_B64_DATA_URL
                    .captures(&src)
                    .and_then(|c| c.get(1))
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default();

                if mime != "image/svg+xml" {
                    // Check if another attribute has a real image URL.
                    let attrs = self.doc.get_all_attrs(elem);
                    let src_removable = attrs.iter().any(|(k, v)| {
                        k != "src" && RX_IMG_EXTENSIONS.is_match(v) && is_valid_url(v)
                    });
                    if src_removable {
                        let b64_start = src.find("base64").map(|i| i + 7).unwrap_or(src.len());
                        if src.len() - b64_start < 133 {
                            self.doc.remove_attr(elem, "src");
                        }
                    }
                }
            }

            // Re-read src/srcset after potential removal.
            let src = self.doc.attr(elem, "src").unwrap_or("").to_string();
            let srcset = self.doc.attr(elem, "srcset").unwrap_or("").to_string();
            if (!src.is_empty() || !srcset.is_empty()) && !class.to_lowercase().contains("lazy") {
                continue;
            }

            // Copy lazy-load attributes to src/srcset.
            let attrs = self.doc.get_all_attrs(elem);
            for (attr_key, attr_val) in attrs {
                if attr_key == "src" || attr_key == "srcset" || attr_key == "alt" {
                    continue;
                }
                let copy_to = if RX_LAZY_IMAGE_SRCSET.is_match(&attr_val) {
                    "srcset"
                } else if RX_LAZY_IMAGE_SRC.is_match(&attr_val) {
                    "src"
                } else {
                    continue;
                };
                if !is_valid_url(&attr_val) {
                    continue;
                }
                if tag == "img" || tag == "picture" {
                    self.doc.set_attr(elem, copy_to, &attr_val);
                } else if tag == "figure" {
                    let has_img = !self
                        .doc
                        .get_all_nodes_with_tag(elem, &["img", "picture"])
                        .is_empty();
                    if !has_img {
                        let img = self.doc.create_element("img");
                        self.doc.set_attr(img, copy_to, &attr_val);
                        self.doc.append_child(elem, img);
                    }
                }
            }
        }
    }

    /// Port of `unwrapNoscriptImages` — replace lazy-load `<img>` placeholders with the
    /// real image from the adjacent `<noscript>` tag.
    pub(super) fn unwrap_noscript_images(&mut self) {
        // Step 1: Remove <img> elements that have no source-like attributes.
        let root = self.doc.root();
        let imgs = self.doc.get_elements_by_tag_name(root, "img");
        let to_remove: Vec<NodeId> = imgs
            .into_iter()
            .filter(|&img| {
                let attrs = self.doc.get_all_attrs(img);
                !attrs.iter().any(|(k, v)| {
                    matches!(k.as_str(), "src" | "data-src" | "srcset" | "data-srcset")
                        || RX_IMG_EXTENSIONS.is_match(v)
                })
            })
            .collect();
        for img in to_remove {
            self.doc.remove(img);
        }

        // Step 2: Replace <noscript> with its contained image when preceded by a single-image element.
        let root = self.doc.root();
        let noscripts = self.doc.get_elements_by_tag_name(root, "noscript");
        for noscript in noscripts {
            if self.doc.parent(noscript).is_none() {
                continue; // already removed
            }

            let content = self.doc.text_content(noscript);
            let fragment = Document::parse(&content);
            let Some(frag_body) = fragment.body() else {
                continue;
            };
            if !is_single_image_in(&fragment, frag_body) {
                continue;
            }

            let prev = self.doc.prev_element_sibling(noscript);

            if let Some(prev_elem) = prev {
                if is_single_image_in(&self.doc, prev_elem) {
                    // Find the prev img element.
                    let prev_img = if self.doc.tag_name(prev_elem) == "img" {
                        prev_elem
                    } else if let Some(i) = self
                        .doc
                        .get_elements_by_tag_name(prev_elem, "img")
                        .into_iter()
                        .next()
                    {
                        i
                    } else {
                        continue;
                    };

                    // Get fragment img attrs.
                    let frag_img = match fragment
                        .get_elements_by_tag_name(frag_body, "img")
                        .into_iter()
                        .next()
                    {
                        Some(i) => i,
                        None => continue,
                    };
                    let frag_attrs = fragment.get_all_attrs(frag_img);

                    // Create replacement img in main tree.
                    let new_img = self.doc.create_element("img");
                    // Copy fragment img attrs first.
                    for (k, v) in &frag_attrs {
                        self.doc.set_attr(new_img, k, v);
                    }
                    // Copy image-relevant attrs from prev_img.
                    let prev_attrs = self.doc.get_all_attrs(prev_img);
                    for (k, v) in prev_attrs {
                        if v.is_empty() {
                            continue;
                        }
                        if k == "src" || k == "srcset" || RX_IMG_EXTENSIONS.is_match(&v) {
                            let existing = self
                                .doc
                                .attr(new_img, &k)
                                .map(|s| s.to_string())
                                .unwrap_or_default();
                            if existing == v {
                                continue;
                            }
                            let dest_key = if self.doc.has_attribute(new_img, &k) {
                                format!("data-old-{k}")
                            } else {
                                k
                            };
                            self.doc.set_attr(new_img, &dest_key, &v);
                        }
                    }

                    // Replace prev_elem with the new img.
                    self.doc.insert_before(prev_elem, new_img);
                    self.doc.remove(prev_elem);
                    self.doc.remove(noscript);
                    continue;
                }
            }

            // No prev single-image element: replace noscript with the fragment's first element.
            let frag_first = match fragment.first_element_child(frag_body) {
                Some(f) => f,
                None => continue,
            };
            let actual_img = if fragment.tag_name(frag_first) == "img" {
                frag_first
            } else {
                match fragment
                    .get_elements_by_tag_name(frag_first, "img")
                    .into_iter()
                    .next()
                {
                    Some(i) => i,
                    None => continue,
                }
            };

            // Skip 1×1 pixel images.
            let w = fragment.attr(actual_img, "width").unwrap_or("");
            let h = fragment.attr(actual_img, "height").unwrap_or("");
            if w == "1" && h == "1" {
                continue;
            }

            // Create a copy of the img in the main tree.
            let new_img = self.doc.create_element("img");
            for (k, v) in fragment.get_all_attrs(actual_img) {
                self.doc.set_attr(new_img, &k, &v);
            }
            self.doc.insert_before(noscript, new_img);
            self.doc.remove(noscript);
        }
    }
}
