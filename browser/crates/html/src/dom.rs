//! Arena DOM core (plan/05 §5.2).
//!
//! Nodes live in a `Document`-owned arena and are referenced by [`NodeId`]
//! (no `Rc` cycles, no aliasing hazards). The tree builder owns all
//! mutation; readers (layout, JS bindings, shell) borrow through accessors.

use std::collections::HashMap;

/// Arena index of a node. Stable for the document lifetime.
pub type NodeId = usize;

/// Namespace of an element (HTML/SVG/MathML per WHATWG).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Namespace {
    Html,
    Svg,
    MathMl,
}

/// `name="value"` attribute. HTML names are ASCII-lowercased at insertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    /// Attribute name.
    pub name: String,
    /// Attribute value.
    pub value: String,
}

/// Element payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementData {
    /// Tag name (ASCII-lowercased for HTML elements).
    pub tag_name: String,
    /// Namespace.
    pub namespace: Namespace,
    /// Attributes in source order.
    pub attributes: Vec<Attribute>,
}

/// Node payload.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeData {
    /// `#document` root (always node 0).
    Document,
    /// `<!DOCTYPE …>`.
    DocumentType {
        name: String,
        public_id: String,
        system_id: String,
    },
    /// Element node.
    Element(ElementData),
    /// Character data.
    Text(String),
    /// `<!-- … -->`.
    Comment(String),
    /// Inert `<template>` contents root.
    DocumentFragment,
}

/// One arena node: payload plus tree links.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Payload.
    pub data: NodeData,
    /// Parent node, if any.
    pub parent: Option<NodeId>,
    /// Children in order.
    pub children: Vec<NodeId>,
}

/// DOCTYPE quirks mode (WHATWG Quirks section).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuirksMode {
    /// Full quirks.
    Quirks,
    /// Limited quirks (almost standards).
    LimitedQuirks,
    /// Standards mode.
    #[default]
    NoQuirks,
}

/// Fatal resource exhaustion (limits from [`crate::ParseOpts`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Truncated {
    /// Node budget exhausted.
    TooManyNodes,
    /// Nesting depth exhausted.
    TooDeep,
}

/// The parsed document: arena plus indexes.
#[derive(Debug, Clone, Default)]
pub struct Document {
    nodes: Vec<Node>,
    /// First `id="…"` element per id (document order wins).
    id_index: HashMap<String, NodeId>,
    /// `<template>` element → its inert contents fragment.
    template_contents: HashMap<NodeId, NodeId>,
    /// Quirks mode from the DOCTYPE.
    pub quirks_mode: QuirksMode,
    /// Non-fatal parse errors (position = UTF-8 byte offset).
    pub errors: Vec<ParseError>,
    /// Set when a limit stopped parsing early (partial tree).
    pub truncated: Option<Truncated>,
    /// Document address (navigation URL, used as base URL later).
    pub url: Option<url::Url>,
}

/// A recoverable spec parse error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// UTF-8 byte offset in the decoded input.
    pub offset: usize,
    /// Machine-readable kind.
    pub kind: &'static str,
}

impl Document {
    /// Empty document with just a `#document` root.
    #[must_use]
    pub fn new() -> Self {
        let mut doc = Self {
            nodes: Vec::new(),
            id_index: HashMap::new(),
            template_contents: HashMap::new(),
            quirks_mode: QuirksMode::default(),
            errors: Vec::new(),
            truncated: None,
            url: None,
        };
        doc.nodes.push(Node {
            data: NodeData::Document,
            parent: None,
            children: Vec::new(),
        });
        doc
    }

    /// The `#document` root (always node 0).
    #[must_use]
    pub fn root(&self) -> NodeId {
        0
    }

    /// Node count (including the root).
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Borrow a node.
    #[must_use]
    pub fn get(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    /// Create a node with no parent yet; caller must attach it.
    pub fn create_node(&mut self, data: NodeData) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node {
            data,
            parent: None,
            children: Vec::new(),
        });
        id
    }

    /// Mutable node access (tree builder only).
    pub(crate) fn nodes_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id]
    }

    /// Register a `<template>` contents fragment.
    pub(crate) fn set_template_contents(&mut self, template: NodeId, fragment: NodeId) {
        self.template_contents.insert(template, fragment);
    }

    /// Contents fragment of a `<template>` element, if any.
    #[must_use]
    pub fn template_contents(&self, template: NodeId) -> Option<NodeId> {
        self.template_contents.get(&template).copied()
    }

    /// Create an element, lowercasing HTML names per spec.
    pub fn create_element(&mut self, tag_name: &str, namespace: Namespace) -> NodeId {
        let tag_name = match namespace {
            Namespace::Html => tag_name.to_ascii_lowercase(),
            _ => tag_name.to_owned(),
        };
        self.create_node(NodeData::Element(ElementData {
            tag_name,
            namespace,
            attributes: Vec::new(),
        }))
    }

    /// Attach `child` at the end of `parent` (reparenting first if needed).
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        self.detach(child);
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.push(child);
        self.index_node(child);
    }

    /// Insert `child` before `before` (which must be a child of `parent`).
    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, before: NodeId) {
        self.detach(child);
        self.nodes[child].parent = Some(parent);
        let siblings = &mut self.nodes[parent].children;
        match siblings.iter().position(|&id| id == before) {
            Some(pos) => siblings.insert(pos, child),
            None => siblings.push(child),
        }
        self.index_node(child);
    }

    /// Remove `node` from its parent (children keep pointing at it until
    /// re-attached; the tree builder always re-attaches immediately).
    pub fn detach(&mut self, node: NodeId) {
        if let Some(parent) = self.nodes[node].parent.take() {
            self.nodes[parent].children.retain(|&id| id != node);
        }
    }

    /// Remove `node` and its whole subtree from the tree.
    pub fn remove_subtree(&mut self, node: NodeId) {
        self.detach(node);
        let mut stack = vec![node];
        let mut removed = Vec::new();
        while let Some(id) = stack.pop() {
            removed.push(id);
            let children = std::mem::take(&mut self.nodes[id].children);
            stack.extend(children);
        }
        let removed_set: std::collections::HashSet<NodeId> = removed.into_iter().collect();
        self.id_index.retain(|_, id| !removed_set.contains(id));
    }

    /// Record a recoverable parse error.
    pub fn error(&mut self, offset: usize, kind: &'static str) {
        // Error spam from hostile input is capped: first 100 are kept.
        if self.errors.len() < 100 {
            self.errors.push(ParseError { offset, kind });
        }
    }

    /// Append text to the last child of `parent` when it is a text node,
    /// else append a new text node (spec character-token coalescing).
    pub fn append_text(&mut self, parent: NodeId, text: &str) {
        if text.is_empty() {
            return;
        }
        let merge_into = self.nodes[parent]
            .children
            .last()
            .copied()
            .filter(|&last| matches!(self.nodes[last].data, NodeData::Text(_)));
        match merge_into {
            Some(id) => {
                if let NodeData::Text(existing) = &mut self.nodes[id].data {
                    existing.push_str(text);
                }
            }
            None => {
                let id = self.create_node(NodeData::Text(text.to_owned()));
                self.nodes[id].parent = Some(parent);
                self.nodes[parent].children.push(id);
            }
        }
    }

    /// `<html>` element, if present.
    #[must_use]
    pub fn document_element(&self) -> Option<NodeId> {
        self.children_of(self.root())
            .into_iter()
            .find(|&id| self.is_element_named(id, "html"))
    }

    /// `<head>` element, if present.
    #[must_use]
    pub fn head(&self) -> Option<NodeId> {
        self.descendant_named(self.root(), "head")
    }

    /// `<body>` element, if present.
    #[must_use]
    pub fn body(&self) -> Option<NodeId> {
        self.descendant_named(self.root(), "body")
    }

    /// `<title>` text, if any.
    #[must_use]
    pub fn title(&self) -> Option<String> {
        let title = self.descendant_named(self.root(), "title")?;
        Some(self.text_content(title))
    }

    /// First element with `id="…"`.
    #[must_use]
    pub fn get_element_by_id(&self, id: &str) -> Option<NodeId> {
        self.id_index.get(id).copied()
    }

    /// All elements with this tag name, in document order.
    #[must_use]
    pub fn get_elements_by_tag_name(&self, tag: &str) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.collect_tag(self.root(), tag, &mut out);
        out
    }

    /// All elements carrying this class (whitespace-separated match).
    #[must_use]
    pub fn get_elements_by_class_name(&self, class: &str) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.collect_class(self.root(), class, &mut out);
        out
    }

    /// Concatenated descendant text (spec `textContent`).
    #[must_use]
    pub fn text_content(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.push_text(id, &mut out);
        out
    }

    /// Direct children of a node.
    #[must_use]
    pub fn children_of(&self, id: NodeId) -> Vec<NodeId> {
        self.nodes[id].children.clone()
    }

    /// True for `<tag>` in the HTML namespace.
    #[must_use]
    pub fn is_element_named(&self, id: NodeId, tag: &str) -> bool {
        match &self.nodes[id].data {
            NodeData::Element(el) => el.namespace == Namespace::Html && el.tag_name == tag,
            _ => false,
        }
    }

    /// Attribute value, if present.
    #[must_use]
    pub fn get_attribute(&self, id: NodeId, name: &str) -> Option<&str> {
        match &self.nodes[id].data {
            NodeData::Element(el) => el
                .attributes
                .iter()
                .find(|attr| attr.name == name)
                .map(|attr| attr.value.as_str()),
            _ => None,
        }
    }

    /// Serialize in html5lib tree-construction format (`| <html>` lines)
    /// for the `.dat` harness (plan/13).
    #[must_use]
    pub fn serialize_html5lib(&self) -> String {
        let mut out = String::new();
        for &child in &self.nodes[self.root()].children.clone() {
            self.serialize_node(child, 1, &mut out);
        }
        out
    }

    // ---------- internals ----------

    fn index_node(&mut self, id: NodeId) {
        if let NodeData::Element(el) = &self.nodes[id].data {
            if el.namespace == Namespace::Html {
                if let Some(value) = el.attributes.iter().find(|a| a.name == "id") {
                    if !value.value.is_empty() {
                        self.id_index.entry(value.value.clone()).or_insert(id);
                    }
                }
            }
        }
    }

    fn descendant_named(&self, id: NodeId, tag: &str) -> Option<NodeId> {
        let mut stack: Vec<NodeId> = self.nodes[id].children.clone();
        stack.reverse();
        while let Some(current) = stack.pop() {
            if self.is_element_named(current, tag) {
                return Some(current);
            }
            let mut children = self.nodes[current].children.clone();
            children.reverse();
            stack.extend(children);
        }
        None
    }

    fn collect_tag(&self, id: NodeId, tag: &str, out: &mut Vec<NodeId>) {
        if self.is_element_named(id, tag) {
            out.push(id);
        }
        for &child in &self.nodes[id].children.clone() {
            self.collect_tag(child, tag, out);
        }
    }

    fn collect_class(&self, id: NodeId, class: &str, out: &mut Vec<NodeId>) {
        if let NodeData::Element(el) = &self.nodes[id].data {
            if el.namespace == Namespace::Html {
                let has = el.attributes.iter().any(|attr| {
                    attr.name == "class" && attr.value.split_whitespace().any(|c| c == class)
                });
                if has {
                    out.push(id);
                }
            }
        }
        for &child in &self.nodes[id].children.clone() {
            self.collect_class(child, class, out);
        }
    }

    fn push_text(&self, id: NodeId, out: &mut String) {
        match &self.nodes[id].data {
            NodeData::Text(text) => out.push_str(text),
            NodeData::Element(_) | NodeData::Document | NodeData::DocumentFragment => {
                for &child in &self.nodes[id].children.clone() {
                    self.push_text(child, out);
                }
            }
            NodeData::DocumentType { .. } | NodeData::Comment(_) => {}
        }
    }

    fn serialize_node(&self, id: NodeId, depth: usize, out: &mut String) {
        let indent = "| ".to_owned() + &"  ".repeat(depth.saturating_sub(1));
        match &self.nodes[id].data.clone() {
            NodeData::DocumentType {
                name,
                public_id,
                system_id,
            } => {
                out.push_str(&indent);
                out.push_str("<!DOCTYPE ");
                out.push_str(name);
                if !public_id.is_empty() {
                    out.push_str(&format!(" PUBLIC \"{public_id}\""));
                    if !system_id.is_empty() {
                        out.push_str(&format!(" \"{system_id}\""));
                    }
                } else if !system_id.is_empty() {
                    out.push_str(&format!(" SYSTEM \"{system_id}\""));
                }
                out.push('>');
                out.push('\n');
            }
            NodeData::Element(el) => {
                out.push_str(&indent);
                out.push('<');
                if el.namespace == Namespace::Svg {
                    out.push_str("svg ");
                } else if el.namespace == Namespace::MathMl {
                    out.push_str("math ");
                }
                out.push_str(&el.tag_name);
                for attr in &el.attributes {
                    out.push_str(&format!(" {}=\"{}\"", attr.name, attr.value));
                }
                out.push('>');
                out.push('\n');
                if el.namespace == Namespace::Html && el.tag_name == "template" {
                    out.push_str(&indent);
                    out.push_str("content\n");
                    // Children live in the inert contents fragment.
                    let children = self
                        .template_contents
                        .get(&id)
                        .map(|&fragment| self.nodes[fragment].children.clone())
                        .unwrap_or_default();
                    for &child in &children {
                        self.serialize_node(child, depth + 1, out);
                    }
                } else {
                    for &child in &self.nodes[id].children.clone() {
                        self.serialize_node(child, depth + 1, out);
                    }
                }
            }
            NodeData::Text(text) => {
                out.push_str(&indent);
                out.push('"');
                out.push_str(text);
                out.push('"');
                out.push('\n');
            }
            NodeData::Comment(text) => {
                out.push_str(&indent);
                out.push_str("<!-- ");
                out.push_str(text.trim());
                out.push_str(" -->");
                out.push('\n');
            }
            NodeData::Document => {
                for &child in &self.nodes[id].children.clone() {
                    self.serialize_node(child, depth, out);
                }
            }
            NodeData::DocumentFragment => {
                for &child in &self.nodes[id].children.clone() {
                    self.serialize_node(child, depth, out);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element_doc() -> (Document, NodeId) {
        let mut doc = Document::new();
        let html = doc.create_element("html", Namespace::Html);
        doc.append_child(doc.root(), html);
        (doc, html)
    }

    #[test]
    fn append_and_text_coalescing() {
        let (mut doc, html) = element_doc();
        doc.append_text(html, "a");
        doc.append_text(html, "b");
        assert_eq!(doc.children_of(html).len(), 1);
        assert_eq!(doc.text_content(html), "ab");
    }

    #[test]
    fn id_index_first_wins() {
        let (mut doc, html) = element_doc();
        for _ in 0..2 {
            let div = doc.create_element("div", Namespace::Html);
            if let NodeData::Element(el) = &mut doc.nodes[div].data {
                el.attributes.push(Attribute {
                    name: "id".to_owned(),
                    value: "x".to_owned(),
                });
            }
            doc.append_child(html, div);
        }
        let found = doc.get_element_by_id("x").unwrap();
        assert_eq!(doc.children_of(html)[0], found);
    }

    #[test]
    fn detach_and_reparent() {
        let (mut doc, html) = element_doc();
        let a = doc.create_element("a", Namespace::Html);
        let b = doc.create_element("b", Namespace::Html);
        doc.append_child(html, a);
        doc.append_child(a, b);
        doc.append_child(html, b);
        assert_eq!(doc.children_of(a).len(), 0);
        assert_eq!(doc.children_of(html), vec![a, b]);
    }
}
