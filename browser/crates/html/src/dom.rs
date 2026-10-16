//! Arena DOM core (plan/05 §5.2).
//!
//! Nodes live in a `Document`-owned arena and are referenced by [`NodeId`]
//! (no `Rc` cycles, no aliasing hazards). The tree builder owns all
//! mutation; readers (layout, JS bindings, shell) borrow through accessors.

use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct CanvasBitmap {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}
impl PartialEq for CanvasBitmap {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationKind {
    ChildList,
    Attributes,
    CharacterData,
}
#[derive(Debug, Clone)]
pub struct Mutation {
    pub kind: MutationKind,
    pub target: NodeId,
    pub added: Vec<NodeId>,
    pub removed: Vec<NodeId>,
    pub previous: Option<NodeId>,
    pub next: Option<NodeId>,
    pub name: Option<String>,
    pub old_value: Option<Arc<str>>,
    pub ancestors: Vec<NodeId>,
}
impl Mutation {
    pub fn retained_bytes(&self) -> usize {
        128 + (self.added.len() + self.removed.len() + self.ancestors.len()) * 8
            + self.name.as_ref().map_or(0, String::len)
            + self.old_value.as_ref().map_or(0, |s| s.len())
    }
    pub fn nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::once(self.target)
            .chain(self.added.iter().copied())
            .chain(self.removed.iter().copied())
            .chain(self.previous)
            .chain(self.next)
            .chain(self.ancestors.iter().copied())
    }
}

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
#[derive(Debug, Clone)]
pub struct Document {
    nodes: Vec<Node>,
    free_nodes: Vec<NodeId>,
    controls: HashMap<NodeId, ControlState>,
    canvases: HashMap<NodeId, CanvasBitmap>,
    canvas_epochs: HashMap<NodeId, u64>,
    image_epochs: HashMap<NodeId, u64>,
    mutation_enabled: bool,
    mutations: Vec<Mutation>,
    mutation_bytes: usize,
    mutation_overflow: bool,
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
impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ControlState {
    pub value: Option<String>,
    pub checked: Option<bool>,
    pub validation_message: String,
}
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct DomError(pub &'static str);

/// A recoverable spec parse error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// UTF-8 byte offset in the decoded input.
    pub offset: usize,
    /// Machine-readable kind.
    pub kind: &'static str,
}

impl Document {
    /// Conservative owned-arena accounting for the per-page scripting cap.
    /// This measures retained capacities, not the operating system's RSS.
    pub fn retained_bytes(&self) -> usize {
        let mut bytes = self.nodes.capacity().saturating_mul(256);
        for node in &self.nodes {
            bytes = bytes.saturating_add(node.children.capacity().saturating_mul(8));
            let payload = match &node.data {
                NodeData::Text(s) | NodeData::Comment(s) => s.capacity(),
                NodeData::DocumentType {
                    name,
                    public_id,
                    system_id,
                } => name
                    .capacity()
                    .saturating_add(public_id.capacity())
                    .saturating_add(system_id.capacity()),
                NodeData::Element(el) => el.attributes.iter().fold(
                    el.tag_name
                        .capacity()
                        .saturating_add(el.attributes.capacity().saturating_mul(64)),
                    |n, a| {
                        n.saturating_add(a.name.capacity())
                            .saturating_add(a.value.capacity())
                    },
                ),
                _ => 0,
            };
            bytes = bytes.saturating_add(payload);
        }
        for state in self.controls.values() {
            bytes = bytes
                .saturating_add(128)
                .saturating_add(state.validation_message.capacity())
                .saturating_add(state.value.as_ref().map_or(0, String::capacity));
        }
        for name in self.id_index.keys() {
            bytes = bytes.saturating_add(64).saturating_add(name.capacity());
        }
        bytes
            .saturating_add(
                self.canvases
                    .values()
                    .map(|c| c.pixels.len())
                    .sum::<usize>(),
            )
            .saturating_add(self.mutation_bytes)
            .saturating_add(self.errors.capacity().saturating_mul(32))
            .saturating_add(self.free_nodes.capacity().saturating_mul(8))
            .saturating_add(self.template_contents.capacity().saturating_mul(32))
    }
    /// Empty document with just a `#document` root.
    #[must_use]
    pub fn new() -> Self {
        let mut doc = Self {
            nodes: Vec::new(),
            free_nodes: Vec::new(),
            controls: HashMap::new(),
            canvases: HashMap::new(),
            canvas_epochs: HashMap::new(),
            image_epochs: HashMap::new(),
            mutation_enabled: false,
            mutations: Vec::new(),
            mutation_bytes: 0,
            mutation_overflow: false,
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
        if let Some(id) = self.free_nodes.pop() {
            self.nodes[id] = Node {
                data,
                parent: None,
                children: Vec::new(),
            };
            return id;
        }
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
        let previous = self.nodes[parent].children.last().copied();
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.push(child);
        self.index_node(child);
        if self.mutation_enabled {
            self.child_mutation(parent, vec![child], Vec::new(), previous, None);
        }
    }

    /// Insert `child` before `before` (which must be a child of `parent`).
    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, before: NodeId) {
        if child == before {
            return;
        }
        self.detach(child);
        self.nodes[child].parent = Some(parent);
        let siblings = &mut self.nodes[parent].children;
        match siblings.iter().position(|&id| id == before) {
            Some(pos) => siblings.insert(pos, child),
            None => siblings.push(child),
        }
        self.index_node(child);
        let position = self.nodes[parent]
            .children
            .iter()
            .position(|&id| id == child)
            .unwrap_or(0);
        let previous = position
            .checked_sub(1)
            .and_then(|i| self.nodes[parent].children.get(i))
            .copied();
        if self.mutation_enabled {
            self.child_mutation(parent, vec![child], Vec::new(), previous, Some(before));
        }
    }

    /// Remove `node` from its parent (children keep pointing at it until
    /// re-attached; the tree builder always re-attaches immediately).
    pub fn detach(&mut self, node: NodeId) {
        if let Some(parent) = self.nodes[node].parent.take() {
            let position = self.nodes[parent]
                .children
                .iter()
                .position(|&id| id == node)
                .unwrap_or(0);
            let previous = position
                .checked_sub(1)
                .and_then(|i| self.nodes[parent].children.get(i))
                .copied();
            let next = self.nodes[parent].children.get(position + 1).copied();
            self.nodes[parent].children.retain(|&id| id != node);
            if self.mutation_enabled {
                self.child_mutation(parent, Vec::new(), vec![node], previous, next);
            }
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
                let old = if self.mutation_enabled {
                    match &self.nodes[id].data {
                        NodeData::Text(s) => Some(Arc::from(s.as_str())),
                        _ => None,
                    }
                } else {
                    None
                };
                if let NodeData::Text(existing) = &mut self.nodes[id].data {
                    existing.push_str(text);
                }
                self.record_mutation(Mutation {
                    kind: MutationKind::CharacterData,
                    target: id,
                    added: Vec::new(),
                    removed: Vec::new(),
                    previous: None,
                    next: None,
                    name: None,
                    old_value: old,
                    ancestors: Vec::new(),
                });
            }
            None => {
                let id = self.create_node(NodeData::Text(text.to_owned()));
                self.append_child(parent, id);
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
        self.elements_from(self.root())
            .into_iter()
            .find(|&node| self.get_attribute(node, "id") == Some(id))
    }

    /// All elements with this tag name, in document order.
    #[must_use]
    pub fn get_elements_by_tag_name(&self, tag: &str) -> Vec<NodeId> {
        self.elements_from(self.root())
            .into_iter()
            .filter(|&id| tag == "*" || self.is_element_named(id, tag))
            .collect()
    }

    /// All elements carrying this class (whitespace-separated match).
    #[must_use]
    pub fn get_elements_by_class_name(&self, class: &str) -> Vec<NodeId> {
        let classes: Vec<_> = class.split_ascii_whitespace().collect();
        self.elements_from(self.root())
            .into_iter()
            .filter(|&id| {
                !classes.is_empty()
                    && classes.iter().all(|c| {
                        self.get_attribute(id, "class")
                            .unwrap_or("")
                            .split_ascii_whitespace()
                            .any(|v| v == *c)
                    })
            })
            .collect()
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
        match self.nodes.get(id).map(|n| &n.data) {
            Some(NodeData::Element(el)) => el.namespace == Namespace::Html && el.tag_name == tag,
            _ => false,
        }
    }

    /// Attribute value, if present.
    #[must_use]
    pub fn get_attribute(&self, id: NodeId, name: &str) -> Option<&str> {
        match self.nodes.get(id).map(|n| &n.data) {
            Some(NodeData::Element(el)) => el
                .attributes
                .iter()
                .find(|attr| attr.name == name)
                .map(|attr| attr.value.as_str()),
            _ => None,
        }
    }

    pub fn try_get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }
    pub fn elements_from(&self, root: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![root];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            if matches!(node.data, NodeData::Element(_)) {
                out.push(id);
            }
            stack.extend(node.children.iter().rev().copied());
        }
        out
    }
    pub fn contains(&self, parent: NodeId, child: NodeId) -> bool {
        let mut current = Some(child);
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = current {
            if id == parent {
                return true;
            }
            if !seen.insert(id) {
                break;
            }
            current = self.nodes.get(id).and_then(|n| n.parent);
        }
        false
    }
    pub fn set_attribute(&mut self, id: NodeId, name: &str, value: &str) -> Result<(), DomError> {
        let observing = self.mutation_enabled;
        if value.len() > crate::DEFAULT_MAX_ATTR_LEN {
            return Err(DomError("attribute value limit"));
        }
        let Some(Node {
            data: NodeData::Element(el),
            ..
        }) = self.nodes.get_mut(id)
        else {
            return Err(DomError("attribute target is not element"));
        };
        let name = if el.namespace == Namespace::Html {
            name.to_ascii_lowercase()
        } else {
            name.to_owned()
        };
        let old = if observing {
            el.attributes
                .iter()
                .find(|a| a.name == name)
                .map(|a| Arc::from(a.value.as_str()))
        } else {
            None
        };
        if let Some(attr) = el.attributes.iter_mut().find(|a| a.name == name) {
            attr.value = value.into();
        } else {
            el.attributes.push(Attribute {
                name: name.clone(),
                value: value.into(),
            });
        }
        let resizes_canvas =
            self.is_element_named(id, "canvas") && matches!(name.as_str(), "width" | "height");
        let image_source = self.is_element_named(id, "img") && name == "src";
        self.record_mutation(Mutation {
            kind: MutationKind::Attributes,
            target: id,
            added: Vec::new(),
            removed: Vec::new(),
            previous: None,
            next: None,
            name: Some(name),
            old_value: old,
            ancestors: Vec::new(),
        });
        if resizes_canvas {
            let epoch = self.canvas_epochs.entry(id).or_default();
            *epoch = epoch.wrapping_add(1);
        }
        if image_source {
            let epoch = self.image_epochs.entry(id).or_default();
            *epoch = epoch.wrapping_add(1);
        }
        Ok(())
    }
    pub fn remove_attribute(&mut self, id: NodeId, name: &str) -> Result<(), DomError> {
        let observing = self.mutation_enabled;
        let Some(Node {
            data: NodeData::Element(el),
            ..
        }) = self.nodes.get_mut(id)
        else {
            return Err(DomError("attribute target is not element"));
        };
        let name = if el.namespace == Namespace::Html {
            name.to_ascii_lowercase()
        } else {
            name.to_owned()
        };
        let old = el.attributes.iter().find(|a| a.name == name).map(|a| {
            if observing {
                Arc::from(a.value.as_str())
            } else {
                Arc::from("")
            }
        });
        el.attributes.retain(|a| a.name != name);
        let resets_canvas =
            old.is_some() && el.tag_name == "canvas" && matches!(name.as_str(), "width" | "height");
        let resets_image = old.is_some() && el.tag_name == "img" && name == "src";
        if old.is_some() {
            self.record_mutation(Mutation {
                kind: MutationKind::Attributes,
                target: id,
                added: Vec::new(),
                removed: Vec::new(),
                previous: None,
                next: None,
                name: Some(name),
                old_value: old,
                ancestors: Vec::new(),
            });
        }
        if resets_canvas {
            let epoch = self.canvas_epochs.entry(id).or_default();
            *epoch = epoch.wrapping_add(1);
        }
        if resets_image {
            let epoch = self.image_epochs.entry(id).or_default();
            *epoch = epoch.wrapping_add(1);
        }
        Ok(())
    }
    pub fn append_checked(
        &mut self,
        parent: NodeId,
        child: NodeId,
        before: Option<NodeId>,
    ) -> Result<(), DomError> {
        if parent >= self.nodes.len() || child >= self.nodes.len() {
            return Err(DomError("unknown node"));
        }
        if parent == child
            || self.contains(child, parent)
            || matches!(self.nodes[child].data, NodeData::Document)
        {
            return Err(DomError("HierarchyRequestError"));
        }
        if !matches!(
            self.nodes[parent].data,
            NodeData::Element(_) | NodeData::Document | NodeData::DocumentFragment
        ) {
            return Err(DomError("HierarchyRequestError"));
        }
        if let Some(before) = before {
            if self.nodes.get(before).and_then(|n| n.parent) != Some(parent) {
                return Err(DomError("NotFoundError"));
            }
        }
        if matches!(self.nodes[child].data, NodeData::DocumentFragment) {
            let children = self.children_of(child);
            for c in children {
                self.append_checked(parent, c, before)?;
            }
            return Ok(());
        }
        let mut depth = 0;
        let mut current = Some(parent);
        while let Some(id) = current {
            depth += 1;
            if depth > crate::DEFAULT_MAX_DEPTH {
                return Err(DomError("DOM depth limit"));
            }
            current = self.nodes[id].parent;
        }
        if let Some(before) = before {
            self.insert_before(parent, child, before)
        } else {
            self.append_child(parent, child)
        }
        Ok(())
    }
    pub fn set_text_content(&mut self, id: NodeId, value: &str) -> Result<(), DomError> {
        if value.len() > 8 * 1024 * 1024 {
            return Err(DomError("DOM text limit"));
        }
        let Some(node) = self.nodes.get_mut(id) else {
            return Err(DomError("unknown node"));
        };
        if matches!(node.data, NodeData::Text(_) | NodeData::Comment(_)) {
            let old = if self.mutation_enabled {
                match &node.data {
                    NodeData::Text(s) | NodeData::Comment(s) => Some(Arc::from(s.as_str())),
                    _ => None,
                }
            } else {
                None
            };
            node.data = if matches!(node.data, NodeData::Text(_)) {
                NodeData::Text(value.into())
            } else {
                NodeData::Comment(value.into())
            };
            self.record_mutation(Mutation {
                kind: MutationKind::CharacterData,
                target: id,
                added: Vec::new(),
                removed: Vec::new(),
                previous: None,
                next: None,
                name: None,
                old_value: old,
                ancestors: Vec::new(),
            });
            return Ok(());
        }
        let children = std::mem::take(&mut node.children);
        for child in &children {
            self.nodes[*child].parent = None;
        }
        let mut added = Vec::new();
        if !value.is_empty() {
            let child = self.create_node(NodeData::Text(value.into()));
            self.nodes[child].parent = Some(id);
            self.nodes[id].children.push(child);
            added.push(child);
        }
        if !children.is_empty() || !added.is_empty() {
            self.child_mutation(id, added, children, None, None);
        }
        Ok(())
    }
    pub fn control_value(&self, id: NodeId) -> &str {
        self.controls
            .get(&id)
            .and_then(|s| s.value.as_deref())
            .or_else(|| self.get_attribute(id, "value"))
            .unwrap_or("")
    }
    pub fn control_checked(&self, id: NodeId) -> bool {
        self.controls
            .get(&id)
            .and_then(|s| s.checked)
            .unwrap_or_else(|| self.get_attribute(id, "checked").is_some())
    }
    pub fn set_control_value(&mut self, id: NodeId, value: String) {
        self.controls.entry(id).or_default().value = Some(value);
    }
    pub fn set_control_checked(&mut self, id: NodeId, value: bool) {
        self.controls.entry(id).or_default().checked = Some(value);
    }
    pub fn control_state_mut(&mut self, id: NodeId) -> &mut ControlState {
        self.controls.entry(id).or_default()
    }
    pub fn control_state(&self, id: NodeId) -> Option<&ControlState> {
        self.controls.get(&id)
    }
    /// Copy fragment/nodes into this arena while retaining detached ownership.
    pub fn import_node(
        &mut self,
        other: &Document,
        source: NodeId,
        deep: bool,
    ) -> Result<NodeId, DomError> {
        if self.node_count() >= crate::DEFAULT_MAX_NODES {
            return Err(DomError("DOM node limit"));
        }
        let node = other
            .try_get(source)
            .ok_or(DomError("unknown source node"))?;
        let root = self.create_node(node.data.clone());
        let mut stack = vec![(source, root, 0usize)];
        while let Some((old, new, depth)) = stack.pop() {
            if !deep {
                break;
            }
            if depth >= crate::DEFAULT_MAX_DEPTH {
                return Err(DomError("DOM depth limit"));
            }
            for &child in &other.get(old).children {
                if self.node_count() >= crate::DEFAULT_MAX_NODES {
                    return Err(DomError("DOM node limit"));
                }
                let id = self.create_node(other.get(child).data.clone());
                self.append_child(new, id);
                stack.push((child, id, depth + 1));
            }
        }
        Ok(root)
    }
    pub fn sweep_detached(&mut self, roots: &std::collections::HashSet<NodeId>) {
        let mut marked = std::collections::HashSet::new();
        let mut work = vec![self.root()];
        work.extend(roots.iter().copied());
        while let Some(id) = work.pop() {
            if !marked.insert(id) {
                continue;
            }
            if let Some(n) = self.nodes.get(id) {
                work.extend(n.children.iter().copied());
                if let Some(p) = n.parent {
                    work.push(p);
                }
                if let Some(fragment) = self.template_contents(id) {
                    work.push(fragment);
                }
            }
        }
        let free = self
            .free_nodes
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        for id in 1..self.nodes.len() {
            if !marked.contains(&id) && !free.contains(&id) {
                self.nodes[id] = Node {
                    data: NodeData::DocumentFragment,
                    parent: None,
                    children: Vec::new(),
                };
                self.controls.remove(&id);
                self.canvases.remove(&id);
                self.canvas_epochs.remove(&id);
                self.image_epochs.remove(&id);
                self.template_contents.remove(&id);
                self.free_nodes.push(id);
            }
        }
        self.id_index.retain(|_, id| marked.contains(id));
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
    pub fn canvas_bitmap(&self, node: NodeId) -> Option<&CanvasBitmap> {
        self.canvases.get(&node)
    }
    pub fn canvas_epoch(&self, node: NodeId) -> u64 {
        self.canvas_epochs.get(&node).copied().unwrap_or(0)
    }
    pub fn set_canvas_bitmap(
        &mut self,
        node: NodeId,
        width: u32,
        height: u32,
        pixels: Arc<[u8]>,
    ) -> Result<(), DomError> {
        if !self.is_element_named(node, "canvas")
            || width > 4096
            || height > 4096
            || pixels.len() != width as usize * height as usize * 4
        {
            return Err(DomError("canvas dimensions"));
        }
        let retained = self
            .canvases
            .iter()
            .filter(|(id, _)| **id != node)
            .map(|(_, c)| c.pixels.len())
            .sum::<usize>();
        if retained.saturating_add(pixels.len()) > 32 * 1024 * 1024 {
            return Err(DomError("page canvas quota"));
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        self.canvases.insert(
            node,
            CanvasBitmap {
                id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                width,
                height,
                pixels,
            },
        );
        Ok(())
    }
    pub fn image_epoch(&self, node: NodeId) -> u64 {
        self.image_epochs.get(&node).copied().unwrap_or(0)
    }
    pub fn observe_mutations(&mut self, enabled: bool) {
        self.mutation_enabled = enabled;
        if !enabled {
            self.mutations.clear();
            self.mutation_bytes = 0;
            self.mutation_overflow = false;
        }
    }
    pub fn mutation_roots(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.mutations.iter().flat_map(Mutation::nodes)
    }
    pub fn take_mutations(&mut self) -> Result<Vec<Mutation>, DomError> {
        if self.mutation_overflow {
            return Err(DomError("mutation queue quota"));
        }
        self.mutation_bytes = 0;
        Ok(std::mem::take(&mut self.mutations))
    }
    fn child_mutation(
        &mut self,
        target: NodeId,
        added: Vec<NodeId>,
        removed: Vec<NodeId>,
        previous: Option<NodeId>,
        next: Option<NodeId>,
    ) {
        self.record_mutation(Mutation {
            kind: MutationKind::ChildList,
            target,
            added,
            removed,
            previous,
            next,
            name: None,
            old_value: None,
            ancestors: Vec::new(),
        });
    }
    fn record_mutation(&mut self, mut mutation: Mutation) {
        if !self.mutation_enabled || self.mutation_overflow {
            return;
        }
        let mut parent = self.nodes.get(mutation.target).and_then(|n| n.parent);
        while let Some(id) = parent {
            if mutation.ancestors.len() >= crate::DEFAULT_MAX_DEPTH {
                self.mutation_overflow = true;
                return;
            }
            mutation.ancestors.push(id);
            parent = self.nodes.get(id).and_then(|n| n.parent);
        }
        let bytes = mutation.retained_bytes();
        if self.mutations.len() >= 1024
            || self.mutation_bytes.saturating_add(bytes) > 16 * 1024 * 1024
        {
            self.mutation_overflow = true;
            return;
        }
        self.mutation_bytes += bytes;
        self.mutations.push(mutation);
    }

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
