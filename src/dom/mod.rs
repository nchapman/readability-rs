// DOM abstraction layer wrapping scraper + ego-tree.
// Provides node access, mutation, and attribute helpers used throughout the parser.
//
// Port of go-readability's DOM usage via github.com/go-shiori/dom.

use std::sync::LazyLock;

use ego_tree::NodeId;
use html5ever::Attribute;
use markup5ever::{namespace_url, ns, LocalName, QualName};
use regex::Regex;
use scraper::{ElementRef, Html, Node, Selector};

pub use ego_tree::NodeId as Id;

// Inline patterns to avoid circular dep on regexp module.
static RX_DISPLAY_NONE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)display\s*:\s*none").unwrap());
static RX_VISIBILITY_HIDDEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)visibility\s*:\s*hidden").unwrap());

/// DOM wrapper around `scraper::Html` that provides both read and write access.
///
/// `scraper::Html` is designed for read-only use; this wrapper adds mutation via
/// the underlying `ego_tree::Tree<scraper::Node>` exposed as `html.tree`.
#[derive(Clone)]
pub struct Document {
    pub(crate) html: Html,
}

impl Document {
    /// Parse an HTML string into a Document.
    pub fn parse(html_str: &str) -> Self {
        Document {
            html: Html::parse_document(html_str),
        }
    }

    /// The root node of the tree (Document node, not the `<html>` element).
    pub fn root(&self) -> NodeId {
        self.html.tree.root().id()
    }

    /// The `<html>` element, i.e. the document element.
    pub fn document_element(&self) -> Option<NodeId> {
        Some(self.html.root_element().id())
    }

    /// The `<body>` element.
    pub fn body(&self) -> Option<NodeId> {
        let doc_elem = self.document_element()?;
        self.html.tree.get(doc_elem)?.children().find_map(|n| {
            if let Node::Element(el) = n.value() {
                if el.name() == "body" {
                    return Some(n.id());
                }
            }
            None
        })
    }

    // ── Node identity ──────────────────────────────────────────────────────

    /// Tag name (lowercase) of an element node, or `""` for non-elements.
    pub fn tag_name(&self, id: NodeId) -> &str {
        match self.html.tree.get(id) {
            Some(node) => match node.value() {
                Node::Element(el) => el.name(),
                _ => "",
            },
            None => "",
        }
    }

    /// True if the node is an element node.
    pub fn is_element(&self, id: NodeId) -> bool {
        self.html
            .tree
            .get(id)
            .map(|n| n.value().is_element())
            .unwrap_or(false)
    }

    /// True if the node is a text node.
    pub fn is_text_node(&self, id: NodeId) -> bool {
        self.html
            .tree
            .get(id)
            .map(|n| matches!(n.value(), Node::Text(_)))
            .unwrap_or(false)
    }

    // ── Attributes ─────────────────────────────────────────────────────────

    /// Get an attribute value.
    pub fn attr(&self, id: NodeId, name: &str) -> Option<&str> {
        self.html
            .tree
            .get(id)?
            .value()
            .as_element()?
            .attr(name)
    }

    /// True if the element has the given attribute.
    pub fn has_attribute(&self, id: NodeId, name: &str) -> bool {
        self.attr(id, name).is_some()
    }

    /// Set an attribute value, creating it if it doesn't exist.
    pub fn set_attr(&mut self, id: NodeId, name: &str, value: &str) {
        if let Some(mut node) = self.html.tree.get_mut(id) {
            if let Node::Element(ref mut el) = *node.value() {
                let qual = QualName::new(None, ns!(), LocalName::from(name));
                let tendril: scraper::StrTendril = value.into();
                // Update existing attr in place (linear scan to match iter_mut).
                for (k, v) in el.attrs.iter_mut() {
                    if k.local.as_ref() == name {
                        *v = tendril;
                        return;
                    }
                }
                // New attribute: insert in sorted order so that scraper's
                // binary-search-based `Element::attr()` can find it.
                let pos = el.attrs.partition_point(|(k, _)| k < &qual);
                el.attrs.insert(pos, (qual, tendril));
            }
        }
    }

    /// Remove an attribute.
    pub fn remove_attr(&mut self, id: NodeId, name: &str) {
        if let Some(mut node) = self.html.tree.get_mut(id) {
            if let Node::Element(ref mut el) = *node.value() {
                el.attrs.retain(|(k, _)| k.local.as_ref() != name);
            }
        }
    }

    // ── Text ───────────────────────────────────────────────────────────────

    /// Recursively collect all text from this node and its descendants.
    pub fn text_content(&self, id: NodeId) -> String {
        let mut s = String::new();
        self.collect_text_content(id, &mut s);
        s
    }

    fn collect_text_content(&self, id: NodeId, out: &mut String) {
        let Some(node) = self.html.tree.get(id) else {
            return;
        };
        match node.value() {
            Node::Text(text) => out.push_str(&text.text),
            _ => {
                for child in node.children() {
                    self.collect_text_content(child.id(), out);
                }
            }
        }
    }

    /// Serialized inner HTML of the node (children only, not the node's own tag).
    pub fn inner_html(&self, id: NodeId) -> String {
        self.html
            .tree
            .get(id)
            .and_then(ElementRef::wrap)
            .map(|el| el.inner_html())
            .unwrap_or_default()
    }

    /// Serialized outer HTML of the node (including the node's own tag).
    pub fn outer_html(&self, id: NodeId) -> String {
        if let Some(node) = self.html.tree.get(id) {
            if let Some(el) = ElementRef::wrap(node) {
                return el.html();
            }
            // Text nodes
            if let Node::Text(ref text) = *node.value() {
                return html_escape_text(&text.text);
            }
        }
        String::new()
    }

    /// Replace all children with a single text node.
    pub fn set_text(&mut self, id: NodeId, text: &str) {
        // Detach all existing children
        let children: Vec<NodeId> = self.child_nodes(id);
        for child_id in children {
            self.remove(child_id);
        }
        // Create text node and append
        let text_id = self.create_text_node(text);
        if let Some(mut parent) = self.html.tree.get_mut(id) {
            parent.append_id(text_id);
        }
    }

    // ── Tree navigation ────────────────────────────────────────────────────

    /// Parent of this node.
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.html.tree.get(id)?.parent().map(|n| n.id())
    }

    /// Direct element children only (excludes text, comments, etc.).
    pub fn children(&self, id: NodeId) -> Vec<NodeId> {
        self.html
            .tree
            .get(id)
            .map(|n| {
                n.children()
                    .filter(|c| c.value().is_element())
                    .map(|c| c.id())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// All direct children, including text and comment nodes.
    pub fn child_nodes(&self, id: NodeId) -> Vec<NodeId> {
        self.html
            .tree
            .get(id)
            .map(|n| n.children().map(|c| c.id()).collect())
            .unwrap_or_default()
    }

    /// First element child.
    pub fn first_element_child(&self, id: NodeId) -> Option<NodeId> {
        self.html.tree.get(id)?.children().find_map(|c| {
            if c.value().is_element() {
                Some(c.id())
            } else {
                None
            }
        })
    }

    /// Last element child.
    pub fn last_element_child(&self, id: NodeId) -> Option<NodeId> {
        self.html
            .tree
            .get(id)?
            .children()
            .filter(|c| c.value().is_element())
            .last()
            .map(|c| c.id())
    }

    /// All ancestors in order (parent, grandparent, …, up to tree root).
    pub fn ancestors(&self, id: NodeId) -> Vec<NodeId> {
        self.html
            .tree
            .get(id)
            .map(|n| n.ancestors().map(|a| a.id()).collect())
            .unwrap_or_default()
    }

    /// Next sibling node (any type, not just elements).
    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.html.tree.get(id)?.next_sibling().map(|n| n.id())
    }

    /// Previous sibling node (any type).
    pub fn prev_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.html.tree.get(id)?.prev_sibling().map(|n| n.id())
    }

    /// Next element sibling (skips text/comment nodes).
    pub fn next_element_sibling(&self, id: NodeId) -> Option<NodeId> {
        let mut cur = self.html.tree.get(id)?.next_sibling();
        while let Some(node) = cur {
            if node.value().is_element() {
                return Some(node.id());
            }
            cur = node.next_sibling();
        }
        None
    }

    /// Previous element sibling (skips text/comment nodes).
    pub fn prev_element_sibling(&self, id: NodeId) -> Option<NodeId> {
        let mut cur = self.html.tree.get(id)?.prev_sibling();
        while let Some(node) = cur {
            if node.value().is_element() {
                return Some(node.id());
            }
            cur = node.prev_sibling();
        }
        None
    }

    /// All element descendants in document order (depth-first), excluding `id` itself.
    pub fn descendants(&self, id: NodeId) -> Vec<NodeId> {
        self.html
            .tree
            .get(id)
            .map(|n| {
                // ego-tree's descendants() includes the root node; skip it.
                n.descendants()
                    .skip(1)
                    .filter(|d| d.value().is_element())
                    .map(|d| d.id())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// All element descendants with the given tag name, excluding `id` itself.
    /// If `tag == "*"` returns all element descendants.
    pub fn get_elements_by_tag_name(&self, id: NodeId, tag: &str) -> Vec<NodeId> {
        self.html
            .tree
            .get(id)
            .map(|n| {
                n.descendants()
                    .skip(1)
                    .filter(|d| {
                        if let Node::Element(el) = d.value() {
                            tag == "*" || el.name() == tag
                        } else {
                            false
                        }
                    })
                    .map(|d| d.id())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// All element descendants matching any of the given tag names, excluding `id` itself.
    pub fn get_all_nodes_with_tag(&self, id: NodeId, tags: &[&str]) -> Vec<NodeId> {
        self.html
            .tree
            .get(id)
            .map(|n| {
                n.descendants()
                    .skip(1)
                    .filter(|d| {
                        if let Node::Element(el) = d.value() {
                            tags.contains(&el.name())
                        } else {
                            false
                        }
                    })
                    .map(|d| d.id())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// First element descendant matching the CSS selector, scoped to subtree `id`.
    pub fn query_selector(&self, id: NodeId, selector: &str) -> Option<NodeId> {
        self.query_selector_all(id, selector).into_iter().next()
    }

    /// All element descendants matching the CSS selector, scoped to subtree `id`.
    pub fn query_selector_all(&self, id: NodeId, selector: &str) -> Vec<NodeId> {
        let Ok(sel) = Selector::parse(selector) else {
            return Vec::new();
        };
        // Collect the set of all descendant NodeIds for filtering (excluding `id` itself).
        let desc_set: std::collections::HashSet<NodeId> = self
            .html
            .tree
            .get(id)
            .map(|n| n.descendants().skip(1).map(|d| d.id()).collect())
            .unwrap_or_default();

        self.html
            .select(&sel)
            .filter(|el| desc_set.contains(&el.id()))
            .map(|el| el.id())
            .collect()
    }

    // ── Tree mutation ──────────────────────────────────────────────────────

    /// Detach a node from its parent. The NodeId remains valid and accessible;
    /// the node just has no parent or siblings.
    pub fn remove(&mut self, id: NodeId) {
        if let Some(mut node) = self.html.tree.get_mut(id) {
            node.detach();
        }
    }

    /// Unwrap an element: detach it and promote its children into its former position.
    ///
    /// Example: `<div><p>hello</p></div>` → `<p>hello</p>` (the div is removed).
    pub fn replace_with_children(&mut self, id: NodeId) {
        let children: Vec<NodeId> = self.child_nodes(id);
        // Insert each child before `id` in order. Because we insert one at a time
        // before `id`, which stays at the end, the order is preserved.
        for child_id in children {
            if let Some(mut node) = self.html.tree.get_mut(id) {
                node.insert_id_before(child_id);
            }
        }
        self.remove(id);
    }

    /// Move `child` to the end of `parent`'s children.
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        if let Some(mut parent_node) = self.html.tree.get_mut(parent) {
            parent_node.append_id(child);
        }
    }

    /// Insert `new_node` as the sibling immediately before `id`.
    pub fn insert_before(&mut self, id: NodeId, new_node: NodeId) {
        if let Some(mut node) = self.html.tree.get_mut(id) {
            node.insert_id_before(new_node);
        }
    }

    /// Create a new orphaned element node with the given tag name.
    ///
    /// The returned NodeId is not yet attached to the tree — use `append_child`
    /// or `insert_before` to place it.
    pub fn create_element(&mut self, tag: &str) -> NodeId {
        let name = QualName::new(None, ns!(html), LocalName::from(tag));
        let element = scraper::node::Element::new(name, vec![]);
        self.html.tree.orphan(Node::Element(element)).id()
    }

    /// Create a new orphaned text node.
    pub fn create_text_node(&mut self, text: &str) -> NodeId {
        let node = Node::Text(scraper::node::Text { text: text.into() });
        self.html.tree.orphan(node).id()
    }

    /// Change an element's tag name by creating a new element, copying all
    /// attributes and children, then replacing the old node.
    ///
    /// Returns the new NodeId — callers must use the returned ID from this point on.
    pub fn set_tag_name(&mut self, id: NodeId, new_tag: &str) -> NodeId {
        // Collect attributes from the old node.
        let attrs: Vec<Attribute> = self
            .html
            .tree
            .get(id)
            .and_then(|n| n.value().as_element())
            .map(|el| {
                el.attrs
                    .iter()
                    .map(|(k, v)| Attribute {
                        name: k.clone(),
                        value: v.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Create the replacement element with the new tag name.
        let new_name = QualName::new(None, ns!(html), LocalName::from(new_tag));
        let new_element = scraper::node::Element::new(new_name, attrs);
        let new_id = self.html.tree.orphan(Node::Element(new_element)).id();

        // Move all children from old to new element.
        if let Some(mut new_node) = self.html.tree.get_mut(new_id) {
            new_node.reparent_from_id_append(id);
        }

        // Insert new element before old, then detach old.
        if let Some(mut old_node) = self.html.tree.get_mut(id) {
            old_node.insert_id_before(new_id);
            old_node.detach();
        }

        new_id
    }

    // ── Visibility ─────────────────────────────────────────────────────────

    /// True if the node is hidden via `display:none`, `visibility:hidden`, or the
    /// `hidden` HTML attribute.
    pub fn is_hidden(&self, id: NodeId) -> bool {
        let Some(node) = self.html.tree.get(id) else {
            return false;
        };
        let Some(el) = node.value().as_element() else {
            return false;
        };
        if el.attr("hidden").is_some() {
            return true;
        }
        if let Some(style) = el.attr("style") {
            if RX_DISPLAY_NONE.is_match(style) || RX_VISIBILITY_HIDDEN.is_match(style) {
                return true;
            }
        }
        false
    }

    // ── Convenience ────────────────────────────────────────────────────────

    /// True if the node has any children (element or text).
    pub fn has_child_nodes(&self, id: NodeId) -> bool {
        self.html
            .tree
            .get(id)
            .map(|n| n.has_children())
            .unwrap_or(false)
    }

    /// Return all attributes of an element as owned `(name, value)` pairs.
    pub fn get_all_attrs(&self, id: NodeId) -> Vec<(String, String)> {
        self.html
            .tree
            .get(id)
            .and_then(|n| n.value().as_element())
            .map(|el| {
                el.attrs
                    .iter()
                    .map(|(k, v)| (k.local.as_ref().to_string(), v.as_ref().to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Rename an element's tag in-place without creating a new node or changing the NodeId.
    ///
    /// Unlike `set_tag_name` (which creates a new element and returns a new NodeId),
    /// this directly mutates the element's `QualName.local` so the NodeId stays stable.
    /// Use this when the caller needs the NodeId to remain valid after the rename.
    pub fn rename_tag(&mut self, id: NodeId, new_tag: &str) {
        if let Some(mut node) = self.html.tree.get_mut(id) {
            if let Node::Element(ref mut el) = *node.value() {
                el.name = QualName::new(None, ns!(html), LocalName::from(new_tag));
            }
        }
    }

    /// True if the node has any element children.
    pub fn has_children(&self, id: NodeId) -> bool {
        self.html
            .tree
            .get(id)
            .map(|n| n.children().any(|c| c.value().is_element()))
            .unwrap_or(false)
    }
}

/// Escape `<`, `>`, and `&` for output in HTML text content.
fn html_escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(html: &str) -> Document {
        Document::parse(html)
    }

    #[test]
    fn text_content_collects_recursively() {
        let d = doc("<div><p>hello</p> <span>world</span></div>");
        let body = d.body().unwrap();
        let text = d.text_content(body);
        assert!(text.contains("hello"), "got: {text:?}");
        assert!(text.contains("world"), "got: {text:?}");
    }

    #[test]
    fn tag_name_returns_lowercase() {
        let d = doc("<p id=\"x\">text</p>");
        let body = d.body().unwrap();
        let p = d.first_element_child(body).unwrap();
        assert_eq!(d.tag_name(p), "p");
    }

    #[test]
    fn attr_get_set_remove() {
        let mut d = doc("<p id=\"x\">text</p>");
        let p = d
            .query_selector(d.document_element().unwrap(), "p")
            .unwrap();
        assert_eq!(d.attr(p, "id"), Some("x"));

        d.set_attr(p, "class", "foo");
        assert_eq!(d.attr(p, "class"), Some("foo"));

        d.set_attr(p, "class", "bar"); // update existing
        assert_eq!(d.attr(p, "class"), Some("bar"));

        d.remove_attr(p, "id");
        assert!(d.attr(p, "id").is_none());
    }

    #[test]
    fn replace_with_children_unwraps_element() {
        let mut d = doc("<div><p>hello</p><span>world</span></div>");
        let body = d.body().unwrap();
        let div = d.first_element_child(body).unwrap();
        d.replace_with_children(div);
        // body should now directly contain p and span
        let children: Vec<_> = d.children(body);
        assert_eq!(children.len(), 2);
        assert_eq!(d.tag_name(children[0]), "p");
        assert_eq!(d.tag_name(children[1]), "span");
    }

    #[test]
    fn remove_detaches_node() {
        let mut d = doc("<div><p id=\"target\">x</p></div>");
        let target = d
            .query_selector(d.document_element().unwrap(), "#target")
            .unwrap();
        d.remove(target);
        // Node data still accessible via NodeId
        assert_eq!(d.tag_name(target), "p");
        // But it's no longer reachable via tree traversal from root
        let all_p = d.get_elements_by_tag_name(d.root(), "p");
        assert!(all_p.is_empty());
    }

    #[test]
    fn is_hidden_detects_style_and_attr() {
        let d = doc(r#"<p style="display:none">a</p><p style="visibility:hidden">b</p><p hidden>c</p><p>d</p>"#);
        let body = d.body().unwrap();
        let kids = d.children(body);
        assert_eq!(kids.len(), 4);
        assert!(d.is_hidden(kids[0]), "display:none should be hidden");
        assert!(d.is_hidden(kids[1]), "visibility:hidden should be hidden");
        assert!(d.is_hidden(kids[2]), "hidden attr should be hidden");
        assert!(!d.is_hidden(kids[3]), "plain <p> should not be hidden");
    }

    #[test]
    fn get_elements_by_tag_name_star() {
        let d = doc("<div><p>a</p><span>b</span></div>");
        let body = d.body().unwrap();
        let all = d.get_elements_by_tag_name(body, "*");
        // div + p + span = 3
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn ars1_caption_credit_tag() {
        let src = std::fs::read_to_string(
            "/Users/nchapman/Drive/Code/lessisbetter/readability-rs/test-pages/ars-1/source.html"
        ).unwrap();
        let d = Document::parse(&src);
        let all = d.get_elements_by_tag_name(d.root(), "*");
        for &n in &all {
            let cls = d.attr(n, "class").unwrap_or("").to_string();
            if cls.contains("caption") {
                eprintln!("  tag={} class={:?}", d.tag_name(n), cls);
                // Also print parent's tag
                if let Some(parent) = d.parent(n) {
                    eprintln!("    parent: tag={}", d.tag_name(parent));
                }
            }
        }
    }

    #[test]
    fn set_tag_name_changes_tag_returns_new_id() {
        let mut d = doc(r#"<div id="x"><p>text</p></div>"#);
        let body = d.body().unwrap();
        let div = d.first_element_child(body).unwrap();
        let new_id = d.set_tag_name(div, "section");
        assert_eq!(d.tag_name(new_id), "section");
        assert_eq!(d.attr(new_id, "id"), Some("x"), "attrs should be preserved");
        // child should still be there
        let child = d.first_element_child(new_id).unwrap();
        assert_eq!(d.tag_name(child), "p");
        // Old NodeId is now detached — it's no longer in the tree
        let body_kids = d.children(body);
        assert_eq!(body_kids.len(), 1);
        assert_eq!(body_kids[0], new_id);
    }

    #[test]
    fn clone_is_independent() {
        let d = Document::parse("<p id=\"x\">hello</p>");
        let p = d
            .query_selector(d.document_element().unwrap(), "p")
            .unwrap();
        let mut d2 = d.clone();
        let p2 = d2
            .query_selector(d2.document_element().unwrap(), "p")
            .unwrap();
        // Mutate d2, verify d is unchanged
        d2.set_attr(p2, "id", "y");
        assert_eq!(d.attr(p, "id"), Some("x"), "original should be unchanged");
        assert_eq!(d2.attr(p2, "id"), Some("y"));
    }

    #[test]
    fn query_selector_scoped_to_subtree() {
        let d = doc("<div id=\"a\"><p class=\"t\">in a</p></div><p class=\"t\">outside</p>");
        let div_a = d
            .query_selector(d.document_element().unwrap(), "#a")
            .unwrap();
        let results = d.query_selector_all(div_a, ".t");
        assert_eq!(results.len(), 1, "should only find <p> inside #a");
    }

    #[test]
    fn ancestors_walks_up() {
        let d = doc("<div><section><p>x</p></section></div>");
        let p = d
            .query_selector(d.document_element().unwrap(), "p")
            .unwrap();
        let ancs = d.ancestors(p);
        // section, div, body, html, Document
        let tags: Vec<_> = ancs.iter().map(|&id| d.tag_name(id)).collect();
        assert!(tags.contains(&"section"), "got: {tags:?}");
        assert!(tags.contains(&"div"), "got: {tags:?}");
        assert!(tags.contains(&"body"), "got: {tags:?}");
    }

    #[test]
    fn next_and_prev_element_sibling() {
        let d = doc("<ul><li>a</li><li>b</li><li>c</li></ul>");
        let ul = d
            .query_selector(d.document_element().unwrap(), "ul")
            .unwrap();
        let items = d.children(ul);
        assert_eq!(items.len(), 3);
        assert_eq!(d.next_element_sibling(items[0]), Some(items[1]));
        assert_eq!(d.prev_element_sibling(items[2]), Some(items[1]));
        assert!(d.prev_element_sibling(items[0]).is_none());
    }

    #[test]
    fn child_nodes_includes_text() {
        let d = doc("<p>hello <em>world</em></p>");
        let p = d
            .query_selector(d.document_element().unwrap(), "p")
            .unwrap();
        let nodes = d.child_nodes(p);
        // "hello " text node + em element
        assert_eq!(nodes.len(), 2);
        assert!(d.is_text_node(nodes[0]));
        assert!(d.is_element(nodes[1]));
    }

    #[test]
    fn create_element_and_append() {
        let mut d = Document::parse("<div id=\"parent\"></div>");
        let parent = d
            .query_selector(d.document_element().unwrap(), "#parent")
            .unwrap();
        let span = d.create_element("span");
        d.set_attr(span, "class", "new");
        d.append_child(parent, span);
        assert_eq!(d.children(parent).len(), 1);
        assert_eq!(d.tag_name(d.children(parent)[0]), "span");
    }
}
