//! WHATWG tree construction (13.2.6, plan/05 §5.2).
//!
//! All insertion modes, foster parenting, the adoption agency and foreign
//! content. Driven token-by-token by `parser.rs`, which also feeds the
//! tokenizer state switches.

use crate::dom::{Attribute, Document, ElementData, Namespace, NodeData, NodeId, QuirksMode};
use crate::tokenizer::Token;

/// Insertion modes (spec 13.2.6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    InHeadNoscript,
    AfterHead,
    InBody,
    Text,
    InTable,
    InTableText,
    InCaption,
    InColumnGroup,
    InTableBody,
    InRow,
    InCell,
    InSelect,
    InSelectInTable,
    InTemplate,
    AfterBody,
    InFrameset,
    AfterFrameset,
    AfterAfterBody,
    AfterAfterFrameset,
}

/// What `process_token` asks the driver to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Keep feeding tokens.
    Continue,
    /// A blocking script just closed (scripting enabled): run it, then resume.
    SuspendForScript,
}

/// Active formatting entries (spec list with markers).
#[derive(Debug, Clone)]
enum FormatEntry {
    Marker,
    Element(NodeId),
}

/// The tree builder: stack of open elements plus all spec pointers.
pub struct TreeBuilder {
    doc: Document,
    open_elements: Vec<NodeId>,
    active_formatting: Vec<FormatEntry>,
    template_modes: Vec<Mode>,
    head_pointer: Option<NodeId>,
    form_pointer: Option<NodeId>,
    frameset_ok: bool,
    scripting: bool,
    mode: Mode,
    original_mode: Mode,
    /// Pending character collector for InTableText.
    table_text: String,
    /// Fragment parsing context element (None = full document).
    fragment_context: Option<String>,
    /// Drop one leading LF (right after `<pre>`/`<listing>`).
    skip_first_lf: bool,
    /// Stop flag after a fatal limit.
    stopped: bool,
    /// Node budget (from `ParseOpts`).
    max_nodes: usize,
    /// Nesting budget (from `ParseOpts`).
    max_depth: usize,
}

impl TreeBuilder {
    /// New builder for a full document.
    #[must_use]
    pub fn new(mut doc: Document, scripting: bool) -> Self {
        doc.quirks_mode = QuirksMode::NoQuirks;
        Self {
            doc,
            open_elements: Vec::new(),
            active_formatting: Vec::new(),
            template_modes: Vec::new(),
            head_pointer: None,
            form_pointer: None,
            frameset_ok: true,
            scripting,
            mode: Mode::Initial,
            original_mode: Mode::InBody,
            table_text: String::new(),
            fragment_context: None,
            skip_first_lf: false,
            stopped: false,
            max_nodes: crate::DEFAULT_MAX_NODES,
            max_depth: crate::DEFAULT_MAX_DEPTH,
        }
    }

    /// Apply the configured resource budgets.
    pub fn set_limits(&mut self, max_nodes: usize, max_depth: usize) {
        self.max_nodes = max_nodes;
        self.max_depth = max_depth;
    }

    /// New builder for fragment parsing under `context_tag`.
    ///
    /// Follows the spec fragment setup in simplified form: a root `<html>`
    /// plus the context element, with the context-dependent insertion mode.
    /// Good enough for `innerHTML`-style callers (full fidelity with the
    /// spec's reset algorithm is Phase 5+ work).
    #[must_use]
    pub fn new_fragment(doc: Document, context_tag: &str, scripting: bool) -> Self {
        let mut builder = Self::new(doc, scripting);
        let context = context_tag.to_ascii_lowercase();
        builder.fragment_context = Some(context.clone());
        let html = builder
            .insert_foreign_adjusted("html", Namespace::Html, &[])
            .expect("fresh arena accepts html");
        builder.open_elements.push(html);
        builder.mode = match context.as_str() {
            "title" | "textarea" | "style" | "xmp" | "iframe" | "noembed" | "noframes"
            | "script" => {
                let el = builder
                    .insert_foreign_adjusted(&context, Namespace::Html, &[])
                    .expect("fresh arena accepts context");
                builder.open_elements.push(el);
                builder.original_mode = Mode::InBody;
                Mode::Text
            }
            "plaintext" => {
                builder.insert_foreign_adjusted("plaintext", Namespace::Html, &[]);
                Mode::InBody
            }
            "table" | "tbody" | "tfoot" | "thead" | "tr" => Mode::InTable,
            "td" | "th" | "caption" | "colgroup" => Mode::InTable,
            "select" => Mode::InSelect,
            _ => Mode::InBody,
        };
        builder
    }

    /// Finish and take the document.
    #[must_use]
    pub fn finish(self) -> Document {
        self.doc
    }

    /// Finish a fragment parse: transplant the context element's children
    /// to the document root (the `<html>` wrapper is dropped), so the
    /// result serializes like an `innerHTML` fragment.
    #[must_use]
    pub fn finish_fragment(mut self) -> Document {
        let context = self.fragment_context.clone();
        if context.is_none() {
            return self.doc;
        }
        // The context element is the second stack entry (after <html>).
        let context_id = self.open_elements.get(1).copied();
        match context_id {
            Some(id) => {
                let children = self.doc.children_of(id);
                for child in children {
                    self.doc.detach(child);
                    let root = self.doc.root();
                    self.doc.append_child(root, child);
                }
                // Drop the wrapper stack.
                if let Some(&html) = self.open_elements.first() {
                    self.doc.remove_subtree(html);
                }
                self.doc
            }
            None => self.doc,
        }
    }

    /// True after a fatal resource limit stopped parsing.
    #[must_use]
    pub fn stopped(&self) -> bool {
        self.stopped
    }

    /// Mutable document access (for error merging).
    pub fn document_mut(&mut self) -> &mut Document {
        &mut self.doc
    }

    /// Read-only document access (for tokenizer sync + tests).
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// Current open element, if any (for tokenizer sync).
    #[must_use]
    pub fn current_node(&self) -> Option<NodeId> {
        self.open_elements.last().copied()
    }

    /// Whether script execution (and thus noscript-as-rawtext) is on.
    #[must_use]
    pub fn scripting_enabled(&self) -> bool {
        self.scripting
    }

    /// True when the current node is a foreign (SVG/MathML) element.
    #[must_use]
    pub fn current_node_is_foreign(&self) -> bool {
        self.current_node().is_some_and(|id| {
            matches!(&self.doc.get(id).data, NodeData::Element(el)
                if el.namespace != Namespace::Html)
        })
    }

    /// Process one token. Returns `SuspendForScript` for blocking scripts.
    pub fn process_token(&mut self, token: Token) -> Step {
        if self.stopped {
            return Step::Continue;
        }
        // Foreign content check happens inside InBody; modes dispatch here.
        match self.mode {
            Mode::Initial => self.mode_initial(token),
            Mode::BeforeHtml => self.mode_before_html(token),
            Mode::BeforeHead => self.mode_before_head(token),
            Mode::InHead => self.mode_in_head(token),
            Mode::InHeadNoscript => self.mode_in_head_noscript(token),
            Mode::AfterHead => self.mode_after_head(token),
            Mode::InBody => self.mode_in_body(token),
            Mode::Text => self.mode_text(token),
            Mode::InTable => self.mode_in_table(token),
            Mode::InTableText => self.mode_in_table_text(token),
            Mode::InCaption => self.mode_in_caption(token),
            Mode::InColumnGroup => self.mode_in_column_group(token),
            Mode::InTableBody => self.mode_in_table_body(token),
            Mode::InRow => self.mode_in_row(token),
            Mode::InCell => self.mode_in_cell(token),
            Mode::InSelect => self.mode_in_select(token),
            Mode::InSelectInTable => self.mode_in_select_in_table(token),
            Mode::InTemplate => self.mode_in_template(token),
            Mode::AfterBody => self.mode_after_body(token),
            Mode::InFrameset => self.mode_in_frameset(token),
            Mode::AfterFrameset => self.mode_after_frameset(token),
            Mode::AfterAfterBody => self.mode_after_after_body(token),
            Mode::AfterAfterFrameset => self.mode_after_after_frameset(token),
        }
    }

    // ================= element helpers =================

    /// Adjusted insertion location target: when the current node itself
    /// is a `<template>` element, insertions land in its inert contents
    /// fragment; deeper descendants take nodes normally. Foster-parented
    /// insertions bypass this function (they call `foster_parent`).
    fn insertion_target(&self) -> NodeId {
        if let Some(current) = self.current_node() {
            if let Some(fragment) = self.doc.template_contents(current) {
                return fragment;
            }
        }
        self.current_node().unwrap_or_else(|| self.doc.root())
    }

    /// Insert an HTML element at the adjusted insertion location.
    fn insert_element(&mut self, tag: &str, attributes: &[(String, String)]) -> Option<NodeId> {
        self.insert_namespaced(tag, Namespace::Html, attributes)
    }

    fn insert_namespaced(
        &mut self,
        tag: &str,
        namespace: Namespace,
        attributes: &[(String, String)],
    ) -> Option<NodeId> {
        if self.doc.node_count() + 1 > self.max_nodes {
            self.doc.truncated = Some(crate::dom::Truncated::TooManyNodes);
            self.stopped = true;
            return None;
        }
        if self.open_elements.len() > self.max_depth {
            self.doc.truncated = Some(crate::dom::Truncated::TooDeep);
            self.stopped = true;
            return None;
        }
        let id = self.doc.create_element(tag, namespace);
        if let NodeData::Element(ElementData {
            attributes: ref mut attrs,
            ..
        }) = &mut self.doc.nodes_mut(id).data
        {
            for (name, value) in attributes {
                attrs.push(Attribute {
                    name: name.clone(),
                    value: value.clone(),
                });
            }
        }
        let target = self.insertion_target();
        self.doc.append_child(target, id);
        Some(id)
    }

    /// Insert without pushing on the stack (void/empty elements).
    fn insert_void(&mut self, tag: &str, attributes: &[(String, String)]) {
        self.insert_element(tag, attributes);
    }

    /// Insert and push on the stack of open elements.
    fn insert_and_push(&mut self, tag: &str, attributes: &[(String, String)]) -> Option<NodeId> {
        let id = self.insert_element(tag, attributes)?;
        self.open_elements.push(id);
        Some(id)
    }

    fn insert_text(&mut self, text: &str) {
        if text.is_empty() || self.stopped {
            return;
        }
        let target = self.insertion_target();
        self.doc.append_text(target, text);
    }

    fn insert_comment(&mut self, text: &str) {
        let target = self.insertion_target();
        let id = self.doc.create_node(NodeData::Comment(text.to_owned()));
        self.doc.append_child(target, id);
    }

    fn foster_parent(&mut self, text: Option<&str>, node: Option<NodeId>) {
        // Last table in the stack; insert before it (or at the foster parent).
        let table = self.open_elements.iter().rev().find(|&&id| {
            matches!(&self.doc.get(id).data, NodeData::Element(el)
                if el.namespace == Namespace::Html && el.tag_name == "table")
        });
        match table {
            Some(&table_id) => {
                let parent = self.doc.get(table_id).parent;
                match parent {
                    Some(parent_id) => {
                        if let Some(text) = text {
                            let id = self.doc.create_node(NodeData::Text(text.to_owned()));
                            self.doc.insert_before(parent_id, id, table_id);
                        } else if let Some(node) = node {
                            self.doc.insert_before(parent_id, node, table_id);
                        }
                    }
                    None => {
                        // Table is detached (fragment case): append to previous element.
                        let foster = self.open_elements.iter().rev().nth(1).copied();
                        match foster {
                            Some(foster_id) => {
                                if let Some(text) = text {
                                    self.doc.append_text(foster_id, text);
                                } else if let Some(node) = node {
                                    self.doc.append_child(foster_id, node);
                                }
                            }
                            None => {
                                if let Some(text) = text {
                                    self.doc.append_text(self.doc.root(), text);
                                } else if let Some(node) = node {
                                    self.doc.append_child(self.doc.root(), node);
                                }
                            }
                        }
                    }
                }
            }
            None => {
                // No table: fall back to normal insertion.
                if let Some(text) = text {
                    self.insert_text(text);
                } else if let Some(node) = node {
                    let target = self.insertion_target();
                    self.doc.append_child(target, node);
                }
            }
        }
    }

    fn pop_until(&mut self, tag: &str) {
        while let Some(id) = self.open_elements.pop() {
            if self.doc.is_element_named(id, tag) {
                break;
            }
            if matches!(&self.doc.get(id).data, NodeData::Element(el)
                if el.namespace == Namespace::Html
                    && (el.tag_name == "html" || el.tag_name == "template"))
            {
                break;
            }
        }
    }

    fn has_in_scope(&self, tag: &str) -> bool {
        self.has_in_scope_list(tag, false)
    }

    fn has_in_button_scope(&self, tag: &str) -> bool {
        self.has_in_scope_list(tag, true)
    }

    fn has_in_scope_list(&self, tag: &str, button_scope: bool) -> bool {
        for &id in self.open_elements.iter().rev() {
            let (is_target, terminates) = match &self.doc.get(id).data {
                NodeData::Element(el) if el.namespace != Namespace::Html => (false, false),
                NodeData::Element(el) => {
                    let terminates = matches!(
                        el.tag_name.as_str(),
                        "applet"
                            | "caption"
                            | "html"
                            | "table"
                            | "td"
                            | "th"
                            | "marquee"
                            | "object"
                            | "template"
                    ) || (button_scope && el.tag_name == "button");
                    (el.tag_name == tag, terminates)
                }
                _ => (false, true),
            };
            if is_target {
                return true;
            }
            if terminates {
                return false;
            }
        }
        false
    }

    fn has_numbered_header_in_scope(&self) -> bool {
        for &id in self.open_elements.iter().rev() {
            match &self.doc.get(id).data {
                NodeData::Element(el) if el.namespace != Namespace::Html => {}
                NodeData::Element(el) => {
                    if matches!(
                        el.tag_name.as_str(),
                        "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                    ) {
                        return true;
                    }
                    if matches!(
                        el.tag_name.as_str(),
                        "applet"
                            | "caption"
                            | "html"
                            | "table"
                            | "td"
                            | "th"
                            | "marquee"
                            | "object"
                            | "template"
                    ) {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        false
    }

    fn generate_implied_end_tags(&mut self, except: Option<&str>) {
        loop {
            let implied = self.current_node().is_some_and(|id| {
                matches!(&self.doc.get(id).data, NodeData::Element(el)
                    if el.namespace == Namespace::Html
                        && matches!(
                            el.tag_name.as_str(),
                            "dd" | "dt" | "li" | "optgroup" | "option" | "p" | "rp" | "rt"
                        )
                        && Some(el.tag_name.as_str()) != except)
            });
            if !implied {
                break;
            }
            self.open_elements.pop();
        }
    }

    fn close_p_in_button_scope(&mut self) {
        if self.has_in_button_scope("p") {
            self.generate_implied_end_tags(Some("p"));
            while let Some(id) = self.current_node() {
                if self.doc.is_element_named(id, "p") {
                    self.open_elements.pop();
                    break;
                }
                self.open_elements.pop();
            }
        }
    }

    // ---------- active formatting ----------

    fn is_formatting_tag(tag: &str) -> bool {
        matches!(
            tag,
            "a" | "b"
                | "big"
                | "code"
                | "em"
                | "font"
                | "i"
                | "s"
                | "small"
                | "strike"
                | "strong"
                | "tt"
                | "u"
        )
    }

    fn push_formatting(&mut self, id: NodeId) {
        // Noah's Ark clause: max 3 identical (tag + attributes) elements.
        let (tag, attrs) = match &self.doc.get(id).data {
            NodeData::Element(el) => (el.tag_name.clone(), el.attributes.clone()),
            _ => return,
        };
        let mut identical = 0;
        let mut first_identical = None;
        for (index, entry) in self.active_formatting.iter().enumerate().rev() {
            match entry {
                FormatEntry::Marker => break,
                FormatEntry::Element(other) => {
                    if let NodeData::Element(other_el) = &self.doc.get(*other).data {
                        if other_el.tag_name == tag && other_el.attributes == attrs {
                            identical += 1;
                            if first_identical.is_none() {
                                first_identical = Some(index);
                            }
                        }
                    }
                }
            }
        }
        if identical >= 3 {
            if let Some(index) = first_identical {
                self.active_formatting.remove(index);
            }
        }
        self.active_formatting.push(FormatEntry::Element(id));
    }

    fn reconstruct_formatting(&mut self) {
        // Find last marker or already-open entry.
        let mut start = None;
        for (index, entry) in self.active_formatting.iter().enumerate().rev() {
            match entry {
                FormatEntry::Marker => {
                    start = Some(index + 1);
                    break;
                }
                FormatEntry::Element(id) => {
                    if !self.open_elements.contains(id) {
                        start = Some(index);
                        break;
                    }
                }
            }
        }
        let Some(mut index) = start else { return };
        // Spec: if the entry IS on the stack, start after it.
        if index < self.active_formatting.len() {
            if let FormatEntry::Element(id) = self.active_formatting[index] {
                if self.open_elements.contains(&id) {
                    index += 1;
                }
            }
        }
        while index < self.active_formatting.len() {
            let id = match self.active_formatting[index] {
                FormatEntry::Element(id) => id,
                FormatEntry::Marker => {
                    index += 1;
                    continue;
                }
            };
            let (tag, attrs) = match &self.doc.get(id).data {
                NodeData::Element(el) => (el.tag_name.clone(), el.attributes.clone()),
                _ => {
                    index += 1;
                    continue;
                }
            };
            let attr_refs: Vec<(String, String)> =
                attrs.into_iter().map(|a| (a.name, a.value)).collect();
            let new_id = match self.insert_element(&tag, &attr_refs) {
                Some(new_id) => new_id,
                None => return,
            };
            self.open_elements.push(new_id);
            self.active_formatting[index] = FormatEntry::Element(new_id);
            index += 1;
        }
    }

    fn clear_formatting_to_marker(&mut self) {
        while let Some(entry) = self.active_formatting.pop() {
            if matches!(entry, FormatEntry::Marker) {
                break;
            }
        }
    }

    /// Adoption agency algorithm for end tags like `</a>`, `</b>`.
    /// Returns true when the token was handled.
    fn adoption_agency(&mut self, subject: &str) -> bool {
        // Step 1-2: current node check.
        if self
            .current_node()
            .is_some_and(|id| self.doc.is_element_named(id, "html"))
            || !self.active_formatting.iter().any(|entry| {
                matches!(entry, FormatEntry::Element(id) if self.doc.is_element_named(*id, subject))
            })
        {
            return false;
        }
        for _ in 0..8 {
            // Step 3-4: formatting element = last matching entry.
            let formatting_pos = self
                .active_formatting
                .iter()
                .rposition(|entry| {
                    matches!(entry, FormatEntry::Element(id) if self.doc.is_element_named(*id, subject))
                });
            let Some(formatting_pos) = formatting_pos else {
                return true;
            };
            let formatting_id = match self.active_formatting[formatting_pos] {
                FormatEntry::Element(id) => id,
                FormatEntry::Marker => return true,
            };
            // Step 5: stack position of the formatting element.
            let stack_pos = match self
                .open_elements
                .iter()
                .rposition(|&id| id == formatting_id)
            {
                Some(pos) => pos,
                None => {
                    // Spec parse error: entry without an open element.
                    self.doc.error(0, "unexpected-end-tag");
                    self.active_formatting.remove(formatting_pos);
                    return true;
                }
            };
            // Step 6: furthest block = topmost special element below it.
            let mut furthest = None;
            for &id in self.open_elements.iter().skip(stack_pos + 1) {
                if is_special(id, &self.doc) {
                    furthest = Some(id);
                    break;
                }
            }
            if furthest.is_none() {
                // No furthest block: pop to formatting element, drop entry.
                while let Some(id) = self.open_elements.pop() {
                    if id == formatting_id {
                        break;
                    }
                }
                self.active_formatting.remove(formatting_pos);
                return true;
            }
            let furthest_id = furthest.expect("checked");
            // Common ancestor (element above the formatting element).
            let common_ancestor = self.open_elements[stack_pos.saturating_sub(1)];
            // Step 8-12: bookmark + inner loop over nodes between.
            // Only nodes in the formatting list are cloned; any other node
            // between is dropped from the stack (spec inner loop).
            let mut bookmark = formatting_pos;
            let mut last_node = furthest_id;
            let mut node_id = furthest_id;
            let mut node_pos = self.open_elements.iter().position(|&id| id == furthest_id);
            let mut inner = 0;
            loop {
                inner += 1;
                // Move to the element above (or above its old position).
                node_pos = match self.open_elements.iter().position(|&id| id == node_id) {
                    Some(pos) => pos.checked_sub(1),
                    None => node_pos.and_then(|pos| pos.checked_sub(1)),
                };
                let Some(pos) = node_pos else { break };
                node_id = self.open_elements[pos];
                if node_id == formatting_id {
                    break;
                }
                let in_list = self
                    .active_formatting
                    .iter()
                    .any(|entry| matches!(entry, FormatEntry::Element(id) if *id == node_id));
                if inner > 3 && in_list {
                    if let Some(pos) = self.active_formatting.iter().position(
                        |entry| matches!(entry, FormatEntry::Element(id) if *id == node_id),
                    ) {
                        self.active_formatting.remove(pos);
                        if pos < bookmark {
                            bookmark = bookmark.saturating_sub(1);
                        }
                    }
                }
                let still_listed = self
                    .active_formatting
                    .iter()
                    .any(|entry| matches!(entry, FormatEntry::Element(id) if *id == node_id));
                if !still_listed {
                    self.open_elements.retain(|&id| id != node_id);
                    continue;
                }
                // Clone the node (same tag/attrs, fresh children).
                let (tag, attrs) = match &self.doc.get(node_id).data {
                    NodeData::Element(el) => (el.tag_name.clone(), el.attributes.clone()),
                    _ => continue,
                };
                if self.doc.node_count() >= self.max_nodes {
                    self.doc.truncated = Some(crate::dom::Truncated::TooManyNodes);
                    self.stopped = true;
                    return true;
                }
                let clone_id = self.doc.create_element(&tag, Namespace::Html);
                if let NodeData::Element(ElementData {
                    attributes: ref mut target,
                    ..
                }) = &mut self.doc.nodes_mut(clone_id).data
                {
                    *target = attrs;
                }
                // Replace in formatting list and stack.
                for entry in self.active_formatting.iter_mut() {
                    if matches!(entry, FormatEntry::Element(id) if *id == node_id) {
                        *entry = FormatEntry::Element(clone_id);
                    }
                }
                if let Some(pos) = self.open_elements.iter().position(|&id| id == node_id) {
                    self.open_elements[pos] = clone_id;
                }
                if last_node == furthest_id {
                    bookmark = self
                        .active_formatting
                        .iter()
                        .position(
                            |entry| matches!(entry, FormatEntry::Element(id) if *id == clone_id),
                        )
                        .map(|pos| pos + 1)
                        .unwrap_or(formatting_pos + 1);
                }
                self.doc.detach(last_node);
                self.doc.append_child(clone_id, last_node);
                last_node = clone_id;
                node_id = clone_id;
                node_pos = self.open_elements.iter().position(|&id| id == clone_id);
            }
            // Step 13-15: move the last node under the common ancestor
            // (or foster-parent it out of table scope).
            let common_id = common_ancestor;
            self.doc.detach(last_node);
            match self.appropriate_for(common_id) {
                FosterTarget::Before(table_id) => {
                    let parent = self.doc.get(table_id).parent;
                    match parent {
                        Some(parent_id) => self.doc.insert_before(parent_id, last_node, table_id),
                        None => self.doc.append_child(self.doc.root(), last_node),
                    }
                }
                FosterTarget::Child(parent_id) => self.doc.append_child(parent_id, last_node),
            }
            // Step 16: clone the formatting element for the leftovers.
            let (tag, attrs) = match &self.doc.get(formatting_id).data {
                NodeData::Element(el) => (el.tag_name.clone(), el.attributes.clone()),
                _ => return true,
            };
            if self.doc.node_count() >= self.max_nodes {
                self.doc.truncated = Some(crate::dom::Truncated::TooManyNodes);
                self.stopped = true;
                return true;
            }
            let new_format = self.doc.create_element(&tag, Namespace::Html);
            if let NodeData::Element(ElementData {
                attributes: ref mut target,
                ..
            }) = &mut self.doc.nodes_mut(new_format).data
            {
                *target = attrs;
            }
            // Move foster children (nodes after furthest) under the clone.
            let foster_children: Vec<NodeId> = {
                let furthest_children = self.doc.get(furthest_id).children.clone();
                furthest_children
            };
            for child in foster_children {
                self.doc.detach(child);
                self.doc.append_child(new_format, child);
            }
            self.doc.append_child(furthest_id, new_format);
            // Step 17: fix the lists.
            // Inner-loop removals shift positions: re-find the stable NodeId.
            if let Some(pos) = self
                .active_formatting
                .iter()
                .position(|entry| matches!(entry, FormatEntry::Element(id) if *id == formatting_id))
            {
                self.active_formatting.remove(pos);
                if pos < bookmark {
                    bookmark = bookmark.saturating_sub(1);
                }
            }
            let bookmark = bookmark.min(self.active_formatting.len());
            self.active_formatting
                .insert(bookmark, FormatEntry::Element(new_format));
            if let Some(pos) = self
                .open_elements
                .iter()
                .position(|&id| id == formatting_id)
            {
                self.open_elements.remove(pos);
            }
            let insert_at = self
                .open_elements
                .iter()
                .position(|&id| id == furthest_id)
                .map(|pos| pos + 1)
                .unwrap_or(self.open_elements.len());
            self.open_elements
                .insert(insert_at.min(self.open_elements.len()), new_format);
        }
        true
    }

    /// Appropriate insertion target under `intended` (spec appropriate
    /// place, simplified): foster-parent before the last table when the
    /// intended parent is table-internal, template or root; plain child
    /// otherwise.
    fn appropriate_for(&self, intended: NodeId) -> FosterTarget {
        let table_boundary = matches!(&self.doc.get(intended).data, NodeData::Element(el)
        if el.namespace == Namespace::Html
            && matches!(
                el.tag_name.as_str(),
                "table" | "tbody" | "tfoot" | "thead" | "tr" | "template" | "html"
            ));
        if !table_boundary {
            return FosterTarget::Child(intended);
        }
        match self.open_elements.iter().rev().find(|&&id| {
            matches!(&self.doc.get(id).data, NodeData::Element(el)
                if el.namespace == Namespace::Html && el.tag_name == "table")
        }) {
            Some(&table_id) => FosterTarget::Before(table_id),
            None => FosterTarget::Child(intended),
        }
    }

    // ================= insertion modes =================

    fn mode_initial(&mut self, token: Token) -> Step {
        match token {
            Token::Characters(text)
                if text.chars().all(|ch| {
                    ch == ' ' || ch == '\t' || ch == '\n' || ch == '\x0C' || ch == '\r'
                }) =>
            {
                Step::Continue
            }
            Token::Comment(text) => {
                let id = self.doc.create_node(NodeData::Comment(text));
                self.doc.append_child(self.doc.root(), id);
                Step::Continue
            }
            Token::Doctype {
                name,
                public_id,
                system_id,
                force_quirks,
            } => {
                let name = name.unwrap_or_default();
                let doctype = self.doc.create_node(NodeData::DocumentType {
                    name: name.clone(),
                    public_id: public_id.unwrap_or_default(),
                    system_id: system_id.unwrap_or_default(),
                });
                self.doc.append_child(self.doc.root(), doctype);
                self.doc.quirks_mode = quirks_for_doctype(&name, force_quirks);
                self.mode = Mode::BeforeHtml;
                Step::Continue
            }
            _ => {
                self.doc.quirks_mode = QuirksMode::Quirks;
                self.mode = Mode::BeforeHtml;
                self.process_token(token)
            }
        }
    }

    fn mode_before_html(&mut self, token: Token) -> Step {
        match token {
            Token::Doctype { .. } => {
                self.doc.error(0, "unexpected-doctype");
                Step::Continue
            }
            Token::Comment(text) => {
                let id = self.doc.create_node(NodeData::Comment(text));
                self.doc.append_child(self.doc.root(), id);
                Step::Continue
            }
            Token::Characters(text)
                if text.chars().all(|ch| {
                    ch == ' ' || ch == '\t' || ch == '\n' || ch == '\x0C' || ch == '\r'
                }) =>
            {
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "html" => {
                let id = self.insert_element("html", &attributes);
                if let Some(id) = id {
                    self.open_elements.push(id);
                }
                self.mode = Mode::BeforeHead;
                Step::Continue
            }
            Token::EndTag { ref name }
                if matches!(name.as_str(), "head" | "body" | "html" | "br") =>
            {
                self.mode = Mode::BeforeHead;
                self.process_token(token)
            }
            Token::Eof => {
                let id = self.insert_element("html", &[]);
                if let Some(id) = id {
                    self.open_elements.push(id);
                }
                self.mode = Mode::BeforeHead;
                self.process_token(token)
            }
            _ => {
                let id = self.insert_element("html", &[]);
                if let Some(id) = id {
                    self.open_elements.push(id);
                }
                self.mode = Mode::BeforeHead;
                self.process_token(token)
            }
        }
    }

    fn mode_before_head(&mut self, token: Token) -> Step {
        match token {
            Token::Characters(text)
                if text.chars().all(|ch| {
                    ch == ' ' || ch == '\t' || ch == '\n' || ch == '\x0C' || ch == '\r'
                }) =>
            {
                self.insert_text(&text);
                Step::Continue
            }
            Token::Comment(text) => {
                self.insert_comment(&text);
                Step::Continue
            }
            Token::Doctype { .. } => {
                self.doc.error(0, "unexpected-doctype");
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "html" => {
                self.mode_in_body(token_with(name, attributes));
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "head" => {
                if let Some(id) = self.insert_and_push("head", &attributes) {
                    self.head_pointer = Some(id);
                }
                self.mode = Mode::InHead;
                Step::Continue
            }
            Token::EndTag { ref name }
                if matches!(name.as_str(), "head" | "body" | "html" | "br") =>
            {
                self.insert_and_push("head", &[]);
                self.mode = Mode::InHead;
                self.process_token(token)
            }
            Token::Eof => {
                self.insert_and_push("head", &[]);
                self.mode = Mode::InHead;
                self.process_token(token)
            }
            _ => {
                self.insert_and_push("head", &[]);
                self.mode = Mode::InHead;
                self.process_token(token)
            }
        }
    }

    fn mode_in_head(&mut self, token: Token) -> Step {
        match token {
            Token::Characters(text)
                if text.chars().all(|ch| {
                    ch == ' ' || ch == '\t' || ch == '\n' || ch == '\x0C' || ch == '\r'
                }) =>
            {
                self.insert_text(&text);
                Step::Continue
            }
            Token::Comment(text) => {
                self.insert_comment(&text);
                Step::Continue
            }
            Token::Doctype { .. } => {
                self.doc.error(0, "unexpected-doctype");
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "html" => {
                self.mode_in_body(token_with(name, attributes));
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(
                name.as_str(),
                "base" | "basefont" | "bgsound" | "link" | "meta"
            ) =>
            {
                self.insert_void(&name, &attributes);
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "title" => {
                self.insert_and_push("title", &attributes);
                self.original_mode = Mode::InHead;
                self.mode = Mode::Text;
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "noframes" | "script" | "style") => {
                self.insert_and_push(&name, &attributes);
                self.original_mode = Mode::InHead;
                self.mode = Mode::Text;
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "noscript" => {
                // With scripting off, noscript holds normal content.
                if self.scripting {
                    self.insert_and_push("noscript", &attributes);
                    self.original_mode = Mode::InHead;
                    self.mode = Mode::InHeadNoscript;
                } else {
                    self.insert_and_push("noscript", &attributes);
                }
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "template" => {
                self.insert_and_push("template", &attributes);
                let fragment = self.doc.create_node(NodeData::DocumentFragment);
                if let Some(&template) = self.open_elements.last() {
                    self.doc.set_template_contents(template, fragment);
                }
                self.active_formatting.push(FormatEntry::Marker);
                self.frameset_ok = false;
                self.template_modes.push(Mode::InTemplate);
                self.mode = Mode::InTemplate;
                Step::Continue
            }
            Token::StartTag { name, .. } if name == "head" => {
                self.doc.error(0, "unexpected-start-tag");
                Step::Continue
            }
            Token::EndTag { name } if name == "head" => {
                self.open_elements.pop();
                self.mode = Mode::AfterHead;
                Step::Continue
            }
            Token::EndTag { ref name } if matches!(name.as_str(), "body" | "html" | "br") => {
                self.open_elements.pop();
                self.mode = Mode::AfterHead;
                self.process_token(token)
            }
            Token::EndTag { name } if name == "template" => {
                if self.has_in_scope("template") {
                    self.generate_implied_end_tags(None);
                    while let Some(id) = self.open_elements.pop() {
                        if self.doc.is_element_named(id, "template") {
                            break;
                        }
                    }
                    self.clear_formatting_to_marker();
                    self.template_modes.pop();
                    self.mode = self.reset_insertion_mode();
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                }
                Step::Continue
            }
            Token::Eof => {
                self.open_elements.pop();
                self.mode = Mode::AfterHead;
                self.process_token(token)
            }
            _ => {
                self.open_elements.pop();
                self.mode = Mode::AfterHead;
                self.process_token(token)
            }
        }
    }

    fn mode_in_head_noscript(&mut self, token: Token) -> Step {
        match token {
            Token::Doctype { .. } => {
                self.doc.error(0, "unexpected-doctype");
                Step::Continue
            }
            Token::StartTag { name, .. } if name == "html" => {
                self.mode_in_body(Token::StartTag {
                    name,
                    attributes: Vec::new(),
                    self_closing: false,
                });
                Step::Continue
            }
            Token::EndTag { name } if name == "noscript" => {
                self.open_elements.pop();
                self.mode = Mode::InHead;
                Step::Continue
            }
            Token::Characters(text)
                if text.chars().all(|ch| {
                    ch == ' ' || ch == '\t' || ch == '\n' || ch == '\x0C' || ch == '\r'
                }) =>
            {
                self.mode_in_body(Token::Characters(text));
                Step::Continue
            }
            Token::Comment(text) => {
                self.mode_in_body(Token::Comment(text));
                Step::Continue
            }
            Token::Characters(text) => {
                self.mode_in_body(Token::Characters(text));
                Step::Continue
            }
            Token::StartTag { .. } => {
                self.doc.error(0, "unexpected-start-tag-in-noscript");
                self.open_elements.pop();
                self.mode = Mode::InHead;
                self.process_token(token)
            }
            Token::EndTag { ref name } if name == "br" => {
                self.open_elements.pop();
                self.mode = Mode::InHead;
                self.process_token(token)
            }
            _ => {
                self.doc.error(0, "unexpected-token-in-noscript");
                self.open_elements.pop();
                self.mode = Mode::InHead;
                self.process_token(token)
            }
        }
    }

    fn mode_after_head(&mut self, token: Token) -> Step {
        match token {
            Token::Characters(text)
                if text.chars().all(|ch| {
                    ch == ' ' || ch == '\t' || ch == '\n' || ch == '\x0C' || ch == '\r'
                }) =>
            {
                self.insert_text(&text);
                Step::Continue
            }
            Token::Comment(text) => {
                self.insert_comment(&text);
                Step::Continue
            }
            Token::Doctype { .. } => {
                self.doc.error(0, "unexpected-doctype");
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "html" => {
                self.mode_in_body(token_with(name, attributes));
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "body" => {
                self.insert_and_push("body", &attributes);
                self.frameset_ok = false;
                self.mode = Mode::InBody;
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "frameset" => {
                self.insert_and_push("frameset", &attributes);
                self.mode = Mode::InFrameset;
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(
                name.as_str(),
                "base"
                    | "basefont"
                    | "bgsound"
                    | "link"
                    | "meta"
                    | "noframes"
                    | "script"
                    | "style"
                    | "template"
                    | "title"
            ) =>
            {
                self.doc.error(0, "unexpected-start-tag-after-head");
                if let Some(head) = self.head_pointer {
                    self.open_elements.push(head);
                }
                self.mode_in_head(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                });
                self.open_elements
                    .retain(|&id| Some(id) != self.head_pointer);
                Step::Continue
            }
            Token::EndTag { ref name } if name == "template" => {
                self.mode_in_head(token);
                Step::Continue
            }
            Token::EndTag { ref name } if matches!(name.as_str(), "body" | "html" | "br") => {
                self.insert_and_push("body", &[]);
                self.mode = Mode::InBody;
                self.process_token(token)
            }
            Token::StartTag { name, .. } if name == "head" => {
                self.doc.error(0, "unexpected-start-tag");
                Step::Continue
            }
            Token::Eof => {
                // An empty body is still created at EOF (reference: html5lib
                // yields html+head+body even for empty input).
                self.insert_and_push("body", &[]);
                self.mode = Mode::InBody;
                self.process_token(token)
            }
            _ => {
                self.insert_and_push("body", &[]);
                self.mode = Mode::InBody;
                self.process_token(token)
            }
        }
    }

    // ================= in body =================

    fn mode_in_body(&mut self, token: Token) -> Step {
        // The pre/listing newline skip applies to the next token only.
        if !matches!(token, Token::Characters(_)) {
            self.skip_first_lf = false;
        }
        // Foreign content (SVG/MathML) has its own insertion rules, except
        // at integration points, which process as plain HTML.
        if self.foreign_namespace().is_some() {
            return self.mode_in_body_foreign(token);
        }
        match token {
            Token::Characters(_) | Token::Eof => self.mode_in_body_textish(token),
            Token::Comment(text) => {
                self.insert_comment(&text);
                Step::Continue
            }
            Token::Doctype { .. } => {
                self.doc.error(0, "unexpected-doctype");
                Step::Continue
            }
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } => self.mode_in_body_start_tag(name, attributes, self_closing),
            Token::EndTag { name } => self.mode_in_body_end_tag(name),
        }
    }

    fn mode_in_body_textish(&mut self, token: Token) -> Step {
        match token {
            Token::Characters(mut text) => {
                if self.skip_first_lf {
                    self.skip_first_lf = false;
                    if let Some(stripped) = text.strip_prefix('\n') {
                        text = stripped.to_owned();
                    }
                }
                self.reconstruct_formatting();
                self.insert_text(&text);
                Step::Continue
            }
            Token::Eof => {
                if !self.template_modes.is_empty() {
                    self.mode_in_template(token);
                } else {
                    self.stop_if_fragment_or_ignore();
                }
                Step::Continue
            }
            _ => unreachable!("textish"),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn mode_in_body_start_tag(
        &mut self,
        name: String,
        attributes: Vec<(String, String)>,
        self_closing: bool,
    ) -> Step {
        // Foreign elements (SVG/MathML) switch to foreign content rules.
        if name == "svg" {
            self.reconstruct_formatting();
            if let Some(id) = self.insert_foreign_adjusted("svg", Namespace::Svg, &attributes) {
                self.open_elements.push(id);
            }
            if self_closing {
                self.open_elements.pop();
            }
            return Step::Continue;
        }
        if name == "math" {
            self.reconstruct_formatting();
            if let Some(id) = self.insert_foreign_adjusted("math", Namespace::MathMl, &attributes) {
                self.open_elements.push(id);
            }
            if self_closing {
                self.open_elements.pop();
            }
            return Step::Continue;
        }
        match name.as_str() {
            "html" => {
                self.doc.error(0, "unexpected-html-start-tag");
                Step::Continue
            }
            "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script" | "style"
            | "template" | "title" => {
                self.mode_in_head(Token::StartTag {
                    name,
                    attributes,
                    self_closing,
                });
                Step::Continue
            }
            "body" => {
                self.doc.error(0, "unexpected-body-start-tag");
                Step::Continue
            }
            "frameset" => {
                self.doc.error(0, "unexpected-frameset-start-tag");
                Step::Continue
            }
            "address" | "article" | "aside" | "blockquote" | "center" | "details" | "dialog"
            | "dir" | "div" | "dl" | "fieldset" | "figcaption" | "figure" | "footer" | "header"
            | "hgroup" | "main" | "menu" | "nav" | "ol" | "p" | "search" | "section"
            | "summary" | "ul" => {
                if self.has_in_button_scope("p") {
                    self.close_p_in_button_scope();
                }
                self.insert_and_push(&name, &attributes);
                Step::Continue
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                if self.has_in_button_scope("p") {
                    self.close_p_in_button_scope();
                }
                if self.current_node().is_some_and(|id| {
                    matches!(&self.doc.get(id).data, NodeData::Element(el)
                        if el.namespace == Namespace::Html
                            && matches!(el.tag_name.as_str(), "h1"|"h2"|"h3"|"h4"|"h5"|"h6"))
                }) {
                    self.open_elements.pop();
                }
                self.insert_and_push(&name, &attributes);
                Step::Continue
            }
            "pre" | "listing" => {
                if self.has_in_button_scope("p") {
                    self.close_p_in_button_scope();
                }
                self.insert_and_push(&name, &attributes);
                // The newline right after <pre>/<listing> is dropped (spec).
                self.skip_first_lf = true;
                self.frameset_ok = false;
                Step::Continue
            }
            "form" => {
                if self.form_pointer.is_some() {
                    self.doc.error(0, "unexpected-form-start-tag");
                } else {
                    if self.has_in_button_scope("p") {
                        self.close_p_in_button_scope();
                    }
                    if let Some(id) = self.insert_and_push("form", &attributes) {
                        self.form_pointer = Some(id);
                    }
                }
                Step::Continue
            }
            "li" => {
                self.frameset_ok = false;
                // Spec loop: a pending li closes; address/div/p stop the
                // search; any other special element stops it too.
                let mut found_li = false;
                for &id in self.open_elements.iter().rev() {
                    match &self.doc.get(id).data {
                        NodeData::Element(el)
                            if el.namespace == Namespace::Html && el.tag_name == "li" =>
                        {
                            found_li = true;
                            break;
                        }
                        NodeData::Element(el)
                            if el.namespace == Namespace::Html
                                && matches!(el.tag_name.as_str(), "address" | "div" | "p") =>
                        {
                            break;
                        }
                        _ => {
                            if is_special(id, &self.doc) {
                                break;
                            }
                        }
                    }
                }
                if found_li {
                    self.generate_implied_end_tags(Some("li"));
                    self.pop_until("li");
                }
                if self.has_in_button_scope("p") {
                    self.close_p_in_button_scope();
                }
                self.insert_and_push("li", &attributes);
                Step::Continue
            }
            "dd" | "dt" => {
                self.frameset_ok = false;
                let mut found = None;
                for &id in self.open_elements.iter().rev() {
                    match &self.doc.get(id).data {
                        NodeData::Element(el)
                            if el.namespace == Namespace::Html
                                && (el.tag_name == "dd" || el.tag_name == "dt") =>
                        {
                            found = Some(el.tag_name.clone());
                            break;
                        }
                        NodeData::Element(el)
                            if el.namespace == Namespace::Html
                                && matches!(el.tag_name.as_str(), "address" | "div" | "p") =>
                        {
                            break;
                        }
                        _ => {
                            if is_special(id, &self.doc) {
                                break;
                            }
                        }
                    }
                }
                if let Some(tag) = found {
                    self.generate_implied_end_tags(Some(&tag));
                    self.pop_until(&tag);
                }
                if self.has_in_button_scope("p") {
                    self.close_p_in_button_scope();
                }
                self.insert_and_push(&name, &attributes);
                Step::Continue
            }
            "iframe" | "noembed" | "noscript" | "xmp" | "textarea" | "plaintext" => {
                // Raw-text elements not covered above; tree side mirrors the
                // tokenizer switch (parser.rs syncs the model).
                self.insert_and_push(&name, &attributes);
                self.original_mode = Mode::InBody;
                self.mode = Mode::Text;
                Step::Continue
            }
            "button" => {
                if self.has_in_scope("button") {
                    self.generate_implied_end_tags(None);
                    while !self
                        .current_node()
                        .is_some_and(|id| self.doc.is_element_named(id, "button"))
                    {
                        self.open_elements.pop();
                    }
                    self.open_elements.pop();
                }
                self.reconstruct_formatting();
                self.insert_and_push("button", &attributes);
                self.frameset_ok = false;
                Step::Continue
            }
            "a" => {
                if self
                    .active_formatting
                    .iter()
                    .any(|entry| matches!(entry, FormatEntry::Element(id) if self.doc.is_element_named(*id, "a")))
                {
                    self.adoption_agency("a");
                }
                self.reconstruct_formatting();
                if let Some(id) = self.insert_and_push("a", &attributes) {
                    self.push_formatting(id);
                }
                Step::Continue
            }
            tag if Self::is_formatting_tag(tag) => {
                self.reconstruct_formatting();
                if let Some(id) = self.insert_and_push(tag, &attributes) {
                    self.push_formatting(id);
                }
                Step::Continue
            }
            "nobr" => {
                self.reconstruct_formatting();
                if self.has_in_scope("nobr") {
                    self.adoption_agency("nobr");
                    self.reconstruct_formatting();
                }
                if let Some(id) = self.insert_and_push("nobr", &attributes) {
                    self.push_formatting(id);
                }
                Step::Continue
            }
            "br" => {
                self.reconstruct_formatting();
                self.insert_void("br", &attributes);
                self.frameset_ok = false;
                Step::Continue
            }
            "wbr" => {
                self.reconstruct_formatting();
                self.insert_void("wbr", &attributes);
                Step::Continue
            }
            "img" => {
                self.reconstruct_formatting();
                self.insert_void("img", &attributes);
                self.frameset_ok = false;
                Step::Continue
            }
            "hr" => {
                if self.has_in_button_scope("p") {
                    self.close_p_in_button_scope();
                }
                self.insert_void("hr", &attributes);
                self.frameset_ok = false;
                Step::Continue
            }
            "area" | "embed" | "keygen" | "param" | "source" | "track" => {
                self.reconstruct_formatting();
                self.insert_void(&name, &attributes);
                self.frameset_ok = false;
                Step::Continue
            }
            "input" => {
                self.reconstruct_formatting();
                self.insert_void("input", &attributes);
                self.frameset_ok = false;
                Step::Continue
            }
            "image" => {
                // Renamed to <img> (silent per spec); reprocess as img.
                self.mode_in_body_start_tag("img".to_owned(), attributes, self_closing)
            }
            "menuitem" => {
                self.reconstruct_formatting();
                self.insert_void("menuitem", &attributes);
                Step::Continue
            }
            "table" => {
                // Quirks carve-out (spec): with no usable doctype, <table>
                // does NOT close an open <p>.
                if self.doc.quirks_mode != QuirksMode::Quirks && self.has_in_button_scope("p") {
                    self.close_p_in_button_scope();
                }
                self.insert_and_push("table", &attributes);
                self.frameset_ok = false;
                self.mode = Mode::InTable;
                Step::Continue
            }
            "caption" | "col" | "colgroup" | "frame" | "head" | "tbody" | "td" | "tfoot" | "th"
            | "thead" | "tr" => {
                self.doc.error(0, "unexpected-table-structure-start-tag");
                Step::Continue
            }
            "select" => {
                self.reconstruct_formatting();
                self.insert_and_push("select", &attributes);
                self.frameset_ok = false;
                self.mode = if matches!(
                    self.mode,
                    Mode::InTable
                        | Mode::InCaption
                        | Mode::InTableBody
                        | Mode::InRow
                        | Mode::InCell
                ) {
                    Mode::InSelectInTable
                } else {
                    Mode::InSelect
                };
                Step::Continue
            }
            "optgroup" | "option" => {
                if self
                    .current_node()
                    .is_some_and(|id| self.doc.is_element_named(id, "option"))
                {
                    self.open_elements.pop();
                }
                self.reconstruct_formatting();
                self.insert_and_push(&name, &attributes);
                Step::Continue
            }
            "rp" | "rt" => {
                if self.has_numbered_header_in_scope() {
                    // Ruby inside ruby: implied ends, then insert.
                }
                self.generate_implied_end_tags(Some("rtc"));
                self.reconstruct_formatting();
                self.insert_and_push(&name, &attributes);
                Step::Continue
            }
            "rtc" => {
                self.generate_implied_end_tags(Some("rtc"));
                self.reconstruct_formatting();
                self.insert_and_push("rtc", &attributes);
                Step::Continue
            }
            "rb" => {
                self.reconstruct_formatting();
                self.insert_and_push("rb", &attributes);
                Step::Continue
            }
            _ => {
                self.reconstruct_formatting();
                self.insert_and_push(&name, &attributes);
                if self_closing {
                    // Parse error per spec, but the element stays open.
                    self.doc
                        .error(0, "non-void-html-element-start-tag-with-trailing-solidus");
                }
                Step::Continue
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    /// Foreign namespace ruling the current node, if tokens must follow
    /// foreign-content rules. `None` at HTML integration points (which
    /// process as plain HTML) and in plain HTML content.
    fn foreign_namespace(&self) -> Option<Namespace> {
        let id = self.current_node()?;
        let (tag, namespace) = match &self.doc.get(id).data {
            NodeData::Element(el) => (el.tag_name.clone(), el.namespace),
            _ => return None,
        };
        if namespace == Namespace::Html {
            return None;
        }
        // MathML text integration points.
        if namespace == Namespace::MathMl
            && matches!(tag.as_str(), "mi" | "mo" | "mn" | "ms" | "mtext")
        {
            return None;
        }
        // HTML integration points.
        if namespace == Namespace::MathMl && tag == "annotation-xml" {
            let htmlish = self.doc.get_attribute(id, "encoding").is_some_and(|enc| {
                enc.eq_ignore_ascii_case("text/html")
                    || enc.eq_ignore_ascii_case("application/xhtml+xml")
            });
            if htmlish {
                return None;
            }
        }
        if namespace == Namespace::Svg && matches!(tag.as_str(), "foreignObject" | "desc" | "title")
        {
            return None;
        }
        Some(namespace)
    }

    /// Insert a foreign element, breaking out of `<font>`/`<s>`-style
    /// wrappers is handled by the caller (HTML breakout set).
    fn mode_in_body_foreign(&mut self, token: Token) -> Step {
        match token {
            Token::Characters(text) => {
                self.reconstruct_formatting();
                self.insert_text(&text);
                Step::Continue
            }
            Token::Comment(text) => {
                self.insert_comment(&text);
                Step::Continue
            }
            Token::Doctype { .. } => {
                self.doc.error(0, "unexpected-doctype");
                Step::Continue
            }
            Token::Eof => self.mode_in_body_textish(token),
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } => {
                // HTML breakout set: pop to HTML/integration, reprocess.
                let breakout = matches!(
                    name.as_str(),
                    "b" | "big"
                        | "blockquote"
                        | "body"
                        | "br"
                        | "center"
                        | "code"
                        | "dd"
                        | "div"
                        | "dl"
                        | "dt"
                        | "em"
                        | "embed"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "head"
                        | "hr"
                        | "i"
                        | "img"
                        | "li"
                        | "listing"
                        | "menu"
                        | "meta"
                        | "nobr"
                        | "ol"
                        | "p"
                        | "pre"
                        | "ruby"
                        | "s"
                        | "small"
                        | "span"
                        | "strong"
                        | "strike"
                        | "sub"
                        | "sup"
                        | "table"
                        | "tt"
                        | "u"
                        | "ul"
                        | "var"
                ) || (name == "font"
                    && attributes
                        .iter()
                        .any(|(attr, _)| matches!(attr.as_str(), "color" | "face" | "size")));
                if breakout {
                    while let Some(id) = self.current_node() {
                        let pop = match &self.doc.get(id).data {
                            NodeData::Element(el) if el.namespace == Namespace::Html => true,
                            NodeData::Element(el) => matches!(
                                (el.namespace, el.tag_name.as_str()),
                                (Namespace::MathMl, "mi" | "mo" | "mn" | "ms" | "mtext")
                                    | (Namespace::MathMl, "annotation-xml")
                                    | (Namespace::Svg, "foreignObject" | "desc" | "title")
                            ),
                            _ => true,
                        };
                        if pop {
                            break;
                        }
                        self.open_elements.pop();
                    }
                    return self.mode_in_body(Token::StartTag {
                        name,
                        attributes,
                        self_closing,
                    });
                }
                let namespace = self.foreign_namespace().unwrap_or(Namespace::Svg);
                if name == "script" && namespace == Namespace::Svg {
                    // SVG script: same tree slot, execution is Phase 5+.
                    if let Some(id) = self.insert_foreign_adjusted("script", namespace, &attributes)
                    {
                        self.open_elements.push(id);
                    }
                    if self_closing {
                        self.open_elements.pop();
                    }
                    return Step::Continue;
                }
                if let Some(id) = self.insert_foreign_adjusted(&name, namespace, &attributes) {
                    self.open_elements.push(id);
                }
                if self_closing {
                    self.open_elements.pop();
                }
                Step::Continue
            }
            Token::EndTag { name } => {
                // SVG script end: pop the script element (no execution yet).
                if name == "script"
                    && self.current_node().is_some_and(|id| {
                        matches!(&self.doc.get(id).data, NodeData::Element(el)
                            if el.namespace == Namespace::Svg && el.tag_name == "script")
                    })
                {
                    self.open_elements.pop();
                    return Step::Continue;
                }
                // Everything else uses the shared walk (fixed below).
                self.any_other_end_tag(name)
            }
        }
    }

    /// Shared "any other end tag" walk (spec): match by tag name in any
    /// namespace; stop at special elements (HTML specials plus the foreign
    /// breakouts: MathML mi/mo/mn/ms/mtext/annotation-xml, SVG
    /// foreignObject/desc/title). Foreign tag names compare
    /// ASCII-case-insensitively (the tokenizer lowercases end tags, while
    /// foreign elements keep camelCase per the fixup table).
    fn any_other_end_tag(&mut self, name: String) -> Step {
        let mut cursor = self.open_elements.len();
        let mut found = false;
        while cursor > 0 {
            cursor -= 1;
            let id = self.open_elements[cursor];
            let same_tag = match &self.doc.get(id).data {
                NodeData::Element(el) if el.namespace == Namespace::Html => el.tag_name == name,
                NodeData::Element(el) => el.tag_name.eq_ignore_ascii_case(&name),
                _ => false,
            };
            if same_tag {
                found = true;
                break;
            }
            if is_special(id, &self.doc) {
                break;
            }
        }
        if found {
            self.generate_implied_end_tags(Some(&name));
            self.pop_until_tag(&name);
        } else {
            self.doc.error(0, "unexpected-end-tag");
        }
        Step::Continue
    }

    /// Pop until an element with this tag name (any namespace) is popped.
    /// Foreign names compare ASCII-case-insensitively (see above).
    fn pop_until_tag(&mut self, tag: &str) {
        while let Some(id) = self.open_elements.pop() {
            if let NodeData::Element(el) = &self.doc.get(id).data {
                let same = if el.namespace == Namespace::Html {
                    el.tag_name == tag
                } else {
                    el.tag_name.eq_ignore_ascii_case(tag)
                };
                if same {
                    break;
                }
            }
            if matches!(&self.doc.get(id).data, NodeData::Element(el)
                if el.namespace == Namespace::Html
                    && (el.tag_name == "html" || el.tag_name == "template"))
            {
                break;
            }
        }
    }
    #[allow(clippy::too_many_lines)]
    fn mode_in_body_end_tag(&mut self, name: String) -> Step {
        match name.as_str() {
            "body" => {
                if self.has_in_scope("body") {
                    self.mode = Mode::AfterBody;
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                }
                Step::Continue
            }
            "html" => {
                if self.has_in_scope("body") {
                    self.mode = Mode::AfterBody;
                    self.process_token(Token::EndTag {
                        name: "body".to_owned(),
                    })
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                    Step::Continue
                }
            }
            "address" | "article" | "aside" | "blockquote" | "button" | "center" | "details"
            | "dialog" | "dir" | "div" | "dl" | "fieldset" | "figcaption" | "figure" | "footer"
            | "header" | "hgroup" | "listing" | "main" | "menu" | "nav" | "ol" | "pre"
            | "search" | "section" | "summary" | "ul" => {
                if self.has_in_scope(&name) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(&name);
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                }
                Step::Continue
            }
            "form" => {
                if self.form_pointer.is_none() {
                    self.doc.error(0, "unexpected-end-tag");
                    return Step::Continue;
                }
                self.generate_implied_end_tags(None);
                if !self
                    .current_node()
                    .is_some_and(|id| self.doc.is_element_named(id, "form"))
                {
                    self.doc.error(0, "unexpected-end-tag");
                }
                let form_id = self.form_pointer.take();
                if let Some(form_id) = form_id {
                    self.open_elements.retain(|&id| id != form_id);
                }
                Step::Continue
            }
            "p" => {
                if !self.has_in_button_scope("p") {
                    // The implied start tag must enter the open-element stack.
                    // Inserting an unattached-to-stack <p> and reprocessing
                    // forever was exposed by the Phase 4 streaming fuzzer.
                    self.doc.error(0, "unmatched-p-end-tag");
                    if self.insert_and_push("p", &[]).is_none() {
                        return Step::Continue;
                    }
                }
                self.close_p_in_button_scope();
                Step::Continue
            }
            "li" => {
                if self.has_in_scope("li") {
                    self.generate_implied_end_tags(Some("li"));
                    self.pop_until("li");
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                }
                Step::Continue
            }
            "dd" | "dt" => {
                if self.has_in_scope(&name) {
                    self.generate_implied_end_tags(Some(&name));
                    self.pop_until(&name);
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                }
                Step::Continue
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                if self.has_numbered_header_in_scope() {
                    self.generate_implied_end_tags(None);
                    while !self.current_node().is_some_and(|id| {
                        matches!(&self.doc.get(id).data, NodeData::Element(el)
                            if el.namespace == Namespace::Html
                                && matches!(el.tag_name.as_str(), "h1"|"h2"|"h3"|"h4"|"h5"|"h6"))
                    }) {
                        if self.open_elements.pop().is_none() {
                            break;
                        }
                    }
                    self.open_elements.pop();
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                }
                Step::Continue
            }
            "a" | "b" | "big" | "code" | "em" | "font" | "i" | "s" | "small" | "strike"
            | "strong" | "tt" | "u" => {
                if !self.adoption_agency(&name) {
                    return self.any_other_end_tag(name);
                }
                Step::Continue
            }
            "nobr" => {
                if !self.adoption_agency("nobr") {
                    return self.any_other_end_tag("nobr".to_owned());
                }
                Step::Continue
            }
            "applet" | "marquee" | "object" => {
                if self.has_in_scope(&name) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(&name);
                    self.clear_formatting_to_marker();
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                }
                Step::Continue
            }
            "br" => {
                self.doc.error(0, "unexpected-end-tag-br");
                self.mode_in_body_start_tag("br".to_owned(), Vec::new(), false);
                Step::Continue
            }
            "template" => {
                self.mode_in_head(Token::EndTag { name });
                Step::Continue
            }
            _ => self.any_other_end_tag(name),
        }
    }

    // ================= text mode =================

    fn mode_text(&mut self, token: Token) -> Step {
        match token {
            Token::Characters(text) => {
                self.insert_text(&text);
                Step::Continue
            }
            Token::Eof => {
                self.doc.error(0, "eof-in-text");
                self.open_elements.pop();
                self.mode = self.original_mode;
                self.process_token(token)
            }
            Token::EndTag { name } => {
                let current_is = self.current_node().is_some_and(|id| {
                    matches!(&self.doc.get(id).data, NodeData::Element(el)
                        if el.namespace == Namespace::Html && el.tag_name == name)
                });
                if current_is {
                    self.open_elements.pop();
                    self.mode = self.original_mode;
                    // Blocking script execution point (scripting enabled).
                    if name == "script" && self.scripting {
                        return Step::SuspendForScript;
                    }
                    Step::Continue
                } else {
                    // Mismatched end tag in text: ignore per spec-ish.
                    self.doc.error(0, "unexpected-end-tag-in-text");
                    Step::Continue
                }
            }
            _ => {
                self.doc.error(0, "unexpected-token-in-text");
                Step::Continue
            }
        }
    }

    // ================= table modes =================

    fn mode_in_table(&mut self, token: Token) -> Step {
        match token {
            Token::Characters(text) => {
                // Table text segregation happens in InTableText.
                self.table_text.clear();
                self.mode = Mode::InTableText;
                self.original_mode = Mode::InTable;
                self.mode_in_table_text(Token::Characters(text))
            }
            Token::Comment(text) => {
                self.insert_comment(&text);
                Step::Continue
            }
            Token::Doctype { .. } => {
                self.doc.error(0, "unexpected-doctype");
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "caption" => {
                self.clear_stack_to_table_context();
                self.active_formatting.push(FormatEntry::Marker);
                self.insert_and_push("caption", &attributes);
                self.mode = Mode::InCaption;
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "colgroup" => {
                self.clear_stack_to_table_context();
                self.insert_and_push("colgroup", &attributes);
                self.mode = Mode::InColumnGroup;
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "col" => {
                self.clear_stack_to_table_context();
                self.insert_and_push("colgroup", &[]);
                self.mode = Mode::InColumnGroup;
                self.process_token(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                })
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "tbody" | "tfoot" | "thead") => {
                self.clear_stack_to_table_context();
                self.insert_and_push(&name, &attributes);
                self.mode = Mode::InTableBody;
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "td" | "th" | "tr") => {
                self.clear_stack_to_table_context();
                self.insert_and_push("tbody", &[]);
                self.mode = Mode::InTableBody;
                self.process_token(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                })
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "table" => {
                self.doc.error(0, "unexpected-table-in-table");
                if !self.has_in_scope("table") {
                    return Step::Continue;
                }
                self.open_elements.pop();
                self.reset_insertion_mode();
                self.process_token(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                })
            }
            Token::EndTag { ref name } if name == "table" => {
                if !self.has_in_scope("table") {
                    self.doc.error(0, "unexpected-end-tag");
                    return Step::Continue;
                }
                self.pop_until("table");
                self.reset_insertion_mode();
                Step::Continue
            }
            Token::EndTag { name }
                if matches!(
                    name.as_str(),
                    "body"
                        | "caption"
                        | "col"
                        | "colgroup"
                        | "html"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) =>
            {
                self.doc.error(0, "unexpected-end-tag-in-table");
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "style" | "script" | "template") => {
                self.mode_in_head(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                });
                Step::Continue
            }
            Token::EndTag { ref name } if name == "template" => {
                self.mode_in_head(token);
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "input" => {
                // `type=hidden` quirk: treat as in-head-ish; else foster parent.
                let hidden = attributes
                    .iter()
                    .any(|(n, v)| n == "type" && v.eq_ignore_ascii_case("hidden"));
                if hidden {
                    self.insert_void("input", &attributes);
                } else {
                    self.foster_parent(None, None);
                    self.in_body_fostered(Token::StartTag {
                        name,
                        attributes,
                        self_closing: false,
                    });
                }
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "form" => {
                self.doc.error(0, "unexpected-form-in-table");
                if self.form_pointer.is_some() || self.has_in_scope("template") {
                    return Step::Continue;
                }
                if let Some(id) = self.insert_and_push("form", &attributes) {
                    self.form_pointer = Some(id);
                    self.open_elements.pop();
                }
                Step::Continue
            }
            Token::Eof => {
                self.mode_in_body(token);
                Step::Continue
            }
            _ => {
                // Anything else: foster parent via in-body rules.
                self.foster_parented_in_body(token);
                Step::Continue
            }
        }
    }

    fn clear_stack_to_table_context(&mut self) {
        while let Some(id) = self.current_node() {
            let stop = matches!(&self.doc.get(id).data, NodeData::Element(el)
                if el.namespace == Namespace::Html
                    && matches!(el.tag_name.as_str(), "table" | "template" | "html"));
            if stop {
                break;
            }
            self.open_elements.pop();
        }
    }

    fn foster_parented_in_body(&mut self, token: Token) {
        // Foster-parent the token's effects: temporarily route insertion.
        self.foster_mode_in_body(token);
    }

    fn foster_mode_in_body(&mut self, token: Token) {
        // Implemented by moving the would-be node: process in body, then
        // relocate the last inserted top-level node(s) before the table.
        // Simplification: text goes foster; elements are created then moved.
        match token {
            Token::Characters(text) if !text.trim().is_empty() => {
                self.foster_parent(Some(&text), None);
            }
            Token::Characters(_) => {}
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } => {
                let before = self.doc.node_count();
                self.mode_in_body_start_tag(name, attributes, self_closing);
                self.relocate_fostered(before);
            }
            Token::EndTag { name } => {
                let before = self.doc.node_count();
                self.mode_in_body_end_tag(name);
                self.relocate_fostered(before);
            }
            Token::Comment(text) => self.insert_comment(&text),
            _ => {
                self.in_body_fostered(token);
            }
        }
    }

    fn relocate_fostered(&mut self, before: usize) {
        if self.doc.node_count() <= before || self.stopped {
            return;
        }
        // Move nodes created by the in-body step to the foster location.
        let new_ids: Vec<NodeId> = (before..self.doc.node_count()).collect();
        for id in new_ids {
            // Only relocate top-level insertions (parent == current node).
            let parent = self.doc.get(id).parent;
            if parent == self.current_node() {
                self.foster_parent(None, Some(id));
            }
        }
    }

    fn in_body_fostered(&mut self, token: Token) {
        // Direct in-body processing for tokens needing no relocation
        // (comments, EOF, doctype already handled by callers).
        match token {
            Token::Comment(text) => self.insert_comment(&text),
            Token::Eof => {
                self.mode_in_body(token);
            }
            _ => {
                self.mode_in_body(token);
            }
        }
    }

    fn mode_in_table_text(&mut self, token: Token) -> Step {
        match token {
            Token::Characters(text) => {
                self.table_text.push_str(&text);
                Step::Continue
            }
            _ => {
                // Flush: whitespace stays, anything else is foster-parented.
                let pending = std::mem::take(&mut self.table_text);
                if !pending.is_empty() {
                    if pending.chars().all(|ch| {
                        ch == ' ' || ch == '\t' || ch == '\n' || ch == '\x0C' || ch == '\r'
                    }) {
                        self.insert_text(&pending);
                    } else {
                        self.foster_parent(Some(&pending), None);
                    }
                }
                self.mode = self.original_mode;
                self.process_token(token)
            }
        }
    }

    fn mode_in_caption(&mut self, token: Token) -> Step {
        match token {
            Token::EndTag { name } if name == "caption" => {
                if self.has_in_scope("caption") {
                    self.generate_implied_end_tags(None);
                    self.pop_until("caption");
                    self.clear_formatting_to_marker();
                    self.mode = Mode::InTable;
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                }
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(
                name.as_str(),
                "caption" | "col" | "colgroup" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr"
            ) =>
            {
                self.doc.error(0, "unexpected-table-part-in-caption");
                if self.has_in_scope("caption") {
                    self.generate_implied_end_tags(None);
                    self.pop_until("caption");
                    self.clear_formatting_to_marker();
                    self.mode = Mode::InTable;
                    self.process_token(Token::StartTag {
                        name,
                        attributes,
                        self_closing: false,
                    })
                } else {
                    Step::Continue
                }
            }
            Token::EndTag { ref name } if name == "table" => {
                if self.has_in_scope("caption") {
                    self.generate_implied_end_tags(None);
                    self.pop_until("caption");
                    self.clear_formatting_to_marker();
                    self.mode = Mode::InTable;
                    self.process_token(Token::EndTag { name: name.clone() })
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                    Step::Continue
                }
            }
            Token::EndTag { name }
                if matches!(
                    name.as_str(),
                    "body"
                        | "col"
                        | "colgroup"
                        | "html"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) =>
            {
                self.doc.error(0, "unexpected-end-tag");
                Step::Continue
            }
            _ => self.mode_in_body(token),
        }
    }

    fn mode_in_column_group(&mut self, token: Token) -> Step {
        match token {
            Token::Characters(text)
                if text.chars().all(|ch| {
                    ch == ' ' || ch == '\t' || ch == '\n' || ch == '\x0C' || ch == '\r'
                }) =>
            {
                self.insert_text(&text);
                Step::Continue
            }
            Token::Comment(text) => {
                self.insert_comment(&text);
                Step::Continue
            }
            Token::Doctype { .. } => {
                self.doc.error(0, "unexpected-doctype");
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "col" => {
                self.insert_void("col", &attributes);
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "template" => {
                self.mode_in_head(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                });
                Step::Continue
            }
            Token::EndTag { ref name } if name == "template" => {
                self.mode_in_head(token);
                Step::Continue
            }
            Token::EndTag { name } if name == "colgroup" => {
                if self
                    .current_node()
                    .is_some_and(|id| self.doc.is_element_named(id, "colgroup"))
                {
                    self.open_elements.pop();
                    self.mode = Mode::InTable;
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                }
                Step::Continue
            }
            Token::EndTag { name } if name == "col" => {
                self.doc.error(0, "unexpected-end-tag");
                Step::Continue
            }
            Token::Eof => {
                self.mode_in_body(token);
                Step::Continue
            }
            _ => {
                if self
                    .current_node()
                    .is_some_and(|id| self.doc.is_element_named(id, "colgroup"))
                {
                    self.open_elements.pop();
                    self.mode = Mode::InTable;
                    self.process_token(token)
                } else {
                    self.doc.error(0, "unexpected-token-in-colgroup");
                    Step::Continue
                }
            }
        }
    }

    fn mode_in_table_body(&mut self, token: Token) -> Step {
        match token {
            Token::StartTag {
                name, attributes, ..
            } if name == "tr" => {
                self.clear_stack_to_table_body_context();
                self.insert_and_push("tr", &attributes);
                self.mode = Mode::InRow;
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "td" | "th") => {
                self.clear_stack_to_table_body_context();
                self.insert_and_push("tr", &[]);
                self.mode = Mode::InRow;
                self.process_token(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                })
            }
            Token::EndTag { name } if matches!(name.as_str(), "tbody" | "tfoot" | "thead") => {
                if self.has_in_scope(&name) {
                    self.clear_stack_to_table_body_context();
                    self.open_elements.pop();
                    self.mode = Mode::InTable;
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                }
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(
                name.as_str(),
                "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead"
            ) =>
            {
                if self.has_in_table_scope(&name) {
                    self.clear_stack_to_table_body_context();
                    self.open_elements.pop();
                    self.mode = Mode::InTable;
                    self.process_token(Token::StartTag {
                        name,
                        attributes,
                        self_closing: false,
                    })
                } else {
                    self.doc.error(0, "unexpected-start-tag");
                    Step::Continue
                }
            }
            Token::EndTag { ref name } if name == "table" => {
                if self.has_in_table_scope("tbody")
                    || self.has_in_table_scope("thead")
                    || self.has_in_table_scope("tfoot")
                {
                    self.clear_stack_to_table_body_context();
                    self.open_elements.pop();
                    self.mode = Mode::InTable;
                    self.process_token(token)
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                    Step::Continue
                }
            }
            Token::EndTag { name }
                if matches!(
                    name.as_str(),
                    "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th" | "tr"
                ) =>
            {
                self.doc.error(0, "unexpected-end-tag");
                Step::Continue
            }
            _ => self.mode_in_table(token),
        }
    }

    fn has_in_table_scope(&self, tag: &str) -> bool {
        for &id in self.open_elements.iter().rev() {
            match &self.doc.get(id).data {
                NodeData::Element(el) if el.namespace != Namespace::Html => {}
                NodeData::Element(el) => {
                    if el.tag_name == tag {
                        return true;
                    }
                    if !matches!(el.tag_name.as_str(), "html" | "table" | "template") {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        false
    }

    fn clear_stack_to_table_body_context(&mut self) {
        while let Some(id) = self.current_node() {
            let stop = matches!(&self.doc.get(id).data, NodeData::Element(el)
                if el.namespace == Namespace::Html
                    && matches!(el.tag_name.as_str(), "tbody" | "tfoot" | "thead" | "template" | "html"));
            if stop {
                break;
            }
            self.open_elements.pop();
        }
    }

    fn clear_stack_to_table_row_context(&mut self) {
        while let Some(id) = self.current_node() {
            let stop = matches!(&self.doc.get(id).data, NodeData::Element(el)
                if el.namespace == Namespace::Html
                    && matches!(el.tag_name.as_str(), "tr" | "template" | "html"));
            if stop {
                break;
            }
            self.open_elements.pop();
        }
    }

    fn mode_in_row(&mut self, token: Token) -> Step {
        match token {
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "td" | "th") => {
                self.clear_stack_to_table_row_context();
                self.insert_and_push(&name, &attributes);
                self.mode = Mode::InCell;
                self.active_formatting.push(FormatEntry::Marker);
                Step::Continue
            }
            Token::EndTag { name } if name == "tr" => {
                if self.has_in_scope("tr") {
                    self.clear_stack_to_table_row_context();
                    self.open_elements.pop();
                    self.mode = Mode::InTableBody;
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                }
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(
                name.as_str(),
                "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead" | "tr"
            ) =>
            {
                if self.has_in_scope("tr") {
                    self.clear_stack_to_table_row_context();
                    self.open_elements.pop();
                    self.mode = Mode::InTableBody;
                    self.process_token(Token::StartTag {
                        name,
                        attributes,
                        self_closing: false,
                    })
                } else {
                    self.doc.error(0, "unexpected-start-tag");
                    Step::Continue
                }
            }
            Token::EndTag { ref name } if name == "table" => {
                if self.has_in_scope("tr") {
                    self.clear_stack_to_table_row_context();
                    self.open_elements.pop();
                    self.mode = Mode::InTableBody;
                    self.process_token(token)
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                    Step::Continue
                }
            }
            Token::EndTag { ref name } if matches!(name.as_str(), "tbody" | "tfoot" | "thead") => {
                if self.has_in_scope(name) {
                    if self.has_in_scope("tr") {
                        self.clear_stack_to_table_row_context();
                        self.open_elements.pop();
                        self.mode = Mode::InTableBody;
                        self.process_token(token)
                    } else {
                        self.doc.error(0, "unexpected-end-tag");
                        Step::Continue
                    }
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                    Step::Continue
                }
            }
            Token::EndTag { name }
                if matches!(
                    name.as_str(),
                    "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th"
                ) =>
            {
                self.doc.error(0, "unexpected-end-tag");
                Step::Continue
            }
            _ => self.mode_in_table(token),
        }
    }

    fn mode_in_cell(&mut self, token: Token) -> Step {
        match token {
            Token::StartTag {
                name, attributes, ..
            } if matches!(
                name.as_str(),
                "caption" | "col" | "colgroup" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr"
            ) =>
            {
                if self.has_in_scope("td") || self.has_in_scope("th") {
                    self.close_cell();
                    self.process_token(Token::StartTag {
                        name,
                        attributes,
                        self_closing: false,
                    })
                } else {
                    self.doc.error(0, "unexpected-start-tag");
                    Step::Continue
                }
            }
            Token::EndTag { name } if matches!(name.as_str(), "td" | "th") => {
                if self.has_in_scope(&name) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(&name);
                    self.clear_formatting_to_marker();
                    self.mode = Mode::InRow;
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                }
                Step::Continue
            }
            Token::EndTag { name }
                if matches!(name.as_str(), "table" | "tbody" | "tfoot" | "thead" | "tr") =>
            {
                // Spec in-cell rule: a cell in scope is enough; the cell
                // closes first and the tag reprocesses (no tr/table check).
                if !(self.has_in_scope("td") || self.has_in_scope("th")) {
                    self.doc.error(0, "unexpected-end-tag");
                    Step::Continue
                } else {
                    self.close_cell();
                    self.process_token(Token::EndTag { name })
                }
            }
            Token::EndTag { name }
                if matches!(
                    name.as_str(),
                    "body" | "caption" | "col" | "colgroup" | "html"
                ) =>
            {
                self.doc.error(0, "unexpected-end-tag");
                Step::Continue
            }
            _ => self.mode_in_body(token),
        }
    }

    fn close_cell(&mut self) {
        self.generate_implied_end_tags(None);
        let in_td = self.has_in_scope("td");
        let target = if in_td { "td" } else { "th" };
        self.pop_until(target);
        self.clear_formatting_to_marker();
        self.mode = Mode::InRow;
    }

    // ================= select =================

    fn mode_in_select(&mut self, token: Token) -> Step {
        match token {
            Token::Characters(text) => {
                self.insert_text(&text);
                Step::Continue
            }
            Token::Comment(text) => {
                self.insert_comment(&text);
                Step::Continue
            }
            Token::Doctype { .. } => {
                self.doc.error(0, "unexpected-doctype");
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "html" => {
                self.mode_in_body(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                });
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "option" => {
                if self
                    .current_node()
                    .is_some_and(|id| self.doc.is_element_named(id, "option"))
                {
                    self.open_elements.pop();
                }
                self.insert_and_push("option", &attributes);
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "optgroup" => {
                if self
                    .current_node()
                    .is_some_and(|id| self.doc.is_element_named(id, "option"))
                {
                    self.open_elements.pop();
                }
                if self
                    .current_node()
                    .is_some_and(|id| self.doc.is_element_named(id, "optgroup"))
                {
                    self.open_elements.pop();
                }
                self.insert_and_push("optgroup", &attributes);
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "hr" => {
                if self
                    .current_node()
                    .is_some_and(|id| self.doc.is_element_named(id, "option"))
                {
                    self.open_elements.pop();
                }
                if self
                    .current_node()
                    .is_some_and(|id| self.doc.is_element_named(id, "optgroup"))
                {
                    self.open_elements.pop();
                }
                self.insert_void("hr", &attributes);
                Step::Continue
            }
            Token::EndTag { name } if name == "optgroup" => {
                if self
                    .current_node()
                    .is_some_and(|id| self.doc.is_element_named(id, "option"))
                    && self
                        .open_elements
                        .iter()
                        .rev()
                        .nth(1)
                        .is_some_and(|&id| self.doc.is_element_named(id, "optgroup"))
                {
                    self.open_elements.pop();
                }
                if self
                    .current_node()
                    .is_some_and(|id| self.doc.is_element_named(id, "optgroup"))
                {
                    self.open_elements.pop();
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                }
                Step::Continue
            }
            Token::EndTag { name } if name == "option" => {
                if self
                    .current_node()
                    .is_some_and(|id| self.doc.is_element_named(id, "option"))
                {
                    self.open_elements.pop();
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                }
                Step::Continue
            }
            Token::EndTag { name } if name == "select" => {
                if !self.has_in_scope("select") {
                    self.doc.error(0, "unexpected-end-tag");
                    return Step::Continue;
                }
                self.pop_until("select");
                self.reset_insertion_mode();
                Step::Continue
            }
            Token::EndTag { ref name } if name == "template" => {
                self.mode_in_head(token);
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(
                name.as_str(),
                "caption" | "table" | "tbody" | "tfoot" | "thead" | "tr" | "td" | "th"
            ) =>
            {
                // Table parts close the select and reprocess outside it.
                self.doc.error(0, "unexpected-table-part-in-select");
                if self.has_in_scope("select") {
                    self.pop_until("select");
                    self.reset_insertion_mode();
                }
                self.process_token(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                })
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "input" | "keygen" | "textarea") => {
                self.doc.error(0, "unexpected-start-tag-in-select");
                if self.has_in_scope("select") {
                    self.pop_until("select");
                    self.reset_insertion_mode();
                    self.process_token(Token::StartTag {
                        name,
                        attributes,
                        self_closing: false,
                    })
                } else {
                    Step::Continue
                }
            }
            Token::StartTag { name, .. } if name == "select" => {
                self.doc.error(0, "unexpected-select-in-select");
                if self.has_in_scope("select") {
                    self.pop_until("select");
                    self.reset_insertion_mode();
                }
                Step::Continue
            }
            Token::Eof => {
                self.mode_in_body(token);
                Step::Continue
            }
            _ => {
                self.doc.error(0, "unexpected-token-in-select");
                Step::Continue
            }
        }
    }

    fn mode_in_select_in_table(&mut self, token: Token) -> Step {
        match token {
            Token::StartTag {
                name, attributes, ..
            } if matches!(
                name.as_str(),
                "caption" | "table" | "tbody" | "tfoot" | "thead" | "tr" | "td" | "th"
            ) =>
            {
                // Table parts close the select and reprocess outside it.
                self.doc.error(0, "unexpected-table-part-in-select");
                if self.has_in_scope("select") {
                    self.pop_until("select");
                    self.reset_insertion_mode();
                }
                self.process_token(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                })
            }
            Token::EndTag { ref name }
                if matches!(
                    name.as_str(),
                    "caption" | "table" | "tbody" | "tfoot" | "thead" | "tr" | "td" | "th"
                ) =>
            {
                self.doc.error(0, "unexpected-table-part-end-in-select");
                if self.has_in_table_scope(name) {
                    self.pop_until("select");
                    self.reset_insertion_mode();
                    self.process_token(token)
                } else {
                    Step::Continue
                }
            }
            _ => self.mode_in_select(token),
        }
    }

    // ================= template / body tail / frameset =================

    fn mode_in_template(&mut self, token: Token) -> Step {
        match token {
            Token::Characters(_) | Token::Comment(_) | Token::Doctype { .. } => {
                self.mode_in_body(token)
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(
                name.as_str(),
                "base"
                    | "basefont"
                    | "bgsound"
                    | "link"
                    | "meta"
                    | "noframes"
                    | "script"
                    | "style"
                    | "template"
                    | "title"
            ) =>
            {
                self.mode_in_head(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                });
                Step::Continue
            }
            Token::EndTag { name } if name == "template" => {
                if self.has_in_scope("template") {
                    self.generate_implied_end_tags(None);
                    while let Some(id) = self.open_elements.pop() {
                        if self.doc.is_element_named(id, "template") {
                            break;
                        }
                    }
                    self.clear_formatting_to_marker();
                    self.template_modes.pop();
                    self.reset_insertion_mode();
                } else {
                    self.doc.error(0, "unexpected-end-tag");
                }
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(
                name.as_str(),
                "caption" | "colgroup" | "tbody" | "tfoot" | "thead"
            ) =>
            {
                self.template_modes.pop();
                self.template_modes.push(Mode::InTable);
                self.mode = Mode::InTable;
                self.process_token(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                })
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "col" => {
                self.template_modes.pop();
                self.template_modes.push(Mode::InColumnGroup);
                self.mode = Mode::InColumnGroup;
                self.process_token(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                })
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "tr" => {
                self.template_modes.pop();
                self.template_modes.push(Mode::InTableBody);
                self.mode = Mode::InTableBody;
                self.process_token(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                })
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "td" | "th") => {
                self.template_modes.pop();
                self.template_modes.push(Mode::InRow);
                self.mode = Mode::InRow;
                self.process_token(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                })
            }
            Token::StartTag { .. } => {
                self.template_modes.pop();
                self.template_modes.push(Mode::InBody);
                self.mode = Mode::InBody;
                self.process_token(token)
            }
            Token::EndTag { .. } => {
                self.doc.error(0, "unexpected-end-tag-in-template");
                Step::Continue
            }
            Token::Eof => {
                if !self.has_in_scope("template") {
                    // Stop parsing (spec: stop).
                    self.stopped = true;
                    return Step::Continue;
                }
                self.generate_implied_end_tags(None);
                while let Some(id) = self.open_elements.pop() {
                    if self.doc.is_element_named(id, "template") {
                        break;
                    }
                }
                self.clear_formatting_to_marker();
                self.template_modes.pop();
                self.reset_insertion_mode();
                self.process_token(token)
            }
        }
    }

    fn mode_after_body(&mut self, token: Token) -> Step {
        match token {
            Token::Characters(text)
                if text.chars().all(|ch| {
                    ch == ' ' || ch == '\t' || ch == '\n' || ch == '\x0C' || ch == '\r'
                }) =>
            {
                self.mode_in_body(Token::Characters(text));
                Step::Continue
            }
            Token::Comment(text) => {
                let html = self
                    .open_elements
                    .first()
                    .copied()
                    .unwrap_or_else(|| self.doc.root());
                let id = self.doc.create_node(NodeData::Comment(text));
                self.doc.append_child(html, id);
                Step::Continue
            }
            Token::Doctype { .. } => {
                self.doc.error(0, "unexpected-doctype");
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "html" => {
                self.mode_in_body(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                });
                Step::Continue
            }
            Token::EndTag { name } if name == "html" => {
                self.mode = Mode::AfterAfterBody;
                Step::Continue
            }
            Token::Eof => Step::Continue,
            _ => {
                self.doc.error(0, "unexpected-token-after-body");
                self.mode = Mode::InBody;
                self.process_token(token)
            }
        }
    }

    fn mode_in_frameset(&mut self, token: Token) -> Step {
        match token {
            Token::Characters(text)
                if text.chars().all(|ch| {
                    ch == ' ' || ch == '\t' || ch == '\n' || ch == '\x0C' || ch == '\r'
                }) =>
            {
                self.insert_text(&text);
                Step::Continue
            }
            Token::Comment(text) => {
                self.insert_comment(&text);
                Step::Continue
            }
            Token::Doctype { .. } => {
                self.doc.error(0, "unexpected-doctype");
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "html" => {
                self.mode_in_body(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                });
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "frameset" => {
                self.insert_and_push("frameset", &attributes);
                Step::Continue
            }
            Token::EndTag { name } if name == "frameset" => {
                if self
                    .current_node()
                    .is_some_and(|id| self.doc.is_element_named(id, "html"))
                {
                    self.doc.error(0, "unexpected-end-tag");
                    Step::Continue
                } else {
                    self.open_elements.pop();
                    self.mode = Mode::AfterFrameset;
                    Step::Continue
                }
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "frame" => {
                self.insert_void("frame", &attributes);
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "noframes" => {
                self.mode_in_head(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                });
                Step::Continue
            }
            Token::Eof => Step::Continue,
            _ => {
                self.doc.error(0, "unexpected-token-in-frameset");
                Step::Continue
            }
        }
    }

    fn mode_after_frameset(&mut self, token: Token) -> Step {
        match token {
            Token::Characters(text)
                if text.chars().all(|ch| {
                    ch == ' ' || ch == '\t' || ch == '\n' || ch == '\x0C' || ch == '\r'
                }) =>
            {
                self.insert_text(&text);
                Step::Continue
            }
            Token::Comment(text) => {
                self.insert_comment(&text);
                Step::Continue
            }
            Token::Doctype { .. } => {
                self.doc.error(0, "unexpected-doctype");
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "html" => {
                self.mode_in_body(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                });
                Step::Continue
            }
            Token::EndTag { name } if name == "html" => {
                self.mode = Mode::AfterAfterFrameset;
                Step::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "noframes" => {
                self.mode_in_head(Token::StartTag {
                    name,
                    attributes,
                    self_closing: false,
                });
                Step::Continue
            }
            Token::Eof => Step::Continue,
            _ => {
                self.doc.error(0, "unexpected-token-after-frameset");
                Step::Continue
            }
        }
    }

    fn mode_after_after_body(&mut self, token: Token) -> Step {
        match token {
            Token::Comment(text) => {
                let id = self.doc.create_node(NodeData::Comment(text));
                self.doc.append_child(self.doc.root(), id);
                Step::Continue
            }
            Token::Doctype { .. } => {
                self.doc.error(0, "unexpected-doctype");
                Step::Continue
            }
            Token::Characters(text) => {
                self.mode_in_body(Token::Characters(text));
                Step::Continue
            }
            Token::StartTag { ref name, .. } if name == "html" => {
                self.mode_in_body(token);
                Step::Continue
            }
            Token::Eof => Step::Continue,
            _ => {
                self.doc.error(0, "unexpected-token-after-after-body");
                self.mode = Mode::InBody;
                self.process_token(token)
            }
        }
    }

    fn mode_after_after_frameset(&mut self, token: Token) -> Step {
        match token {
            Token::Comment(text) => {
                let id = self.doc.create_node(NodeData::Comment(text));
                self.doc.append_child(self.doc.root(), id);
                Step::Continue
            }
            Token::Doctype { .. } => {
                self.doc.error(0, "unexpected-doctype");
                Step::Continue
            }
            Token::Characters(text) => {
                self.mode_in_body(Token::Characters(text));
                Step::Continue
            }
            Token::StartTag { ref name, .. } if name == "html" => {
                self.mode_in_body(token);
                Step::Continue
            }
            Token::Eof => Step::Continue,
            _ => {
                self.doc.error(0, "unexpected-token-after-after-frameset");
                Step::Continue
            }
        }
    }

    // ================= misc =================

    fn reset_insertion_mode(&mut self) -> Mode {
        // Walk the stack for the appropriate mode (spec 13.2.6.4.3).
        let mut last = false;
        for &id in self.open_elements.iter().rev() {
            match &self.doc.get(id).data {
                NodeData::Element(el) if el.namespace == Namespace::Html => {
                    let mode = match el.tag_name.as_str() {
                        "select" => {
                            // Ancestor table check.
                            let mut ancestor = None;
                            for &outer in self.open_elements.iter().rev() {
                                if outer == id {
                                    continue;
                                }
                                match &self.doc.get(outer).data {
                                    NodeData::Element(outer_el)
                                        if outer_el.namespace == Namespace::Html =>
                                    {
                                        ancestor = Some(outer_el.tag_name.clone());
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            match ancestor.as_deref() {
                                Some(
                                    "table" | "tbody" | "tfoot" | "thead" | "tr" | "td" | "th"
                                    | "caption",
                                ) => Mode::InSelectInTable,
                                _ => Mode::InSelect,
                            }
                        }
                        "td" | "th" => Mode::InCell,
                        "tr" => Mode::InRow,
                        "tbody" | "thead" | "tfoot" => Mode::InTableBody,
                        "caption" => Mode::InCaption,
                        "colgroup" => Mode::InColumnGroup,
                        "table" => Mode::InTable,
                        "template" => match self.template_modes.last() {
                            Some(mode) => *mode,
                            None => Mode::InBody,
                        },
                        "head" if !last => Mode::InHead,
                        "body" => Mode::InBody,
                        "frameset" => Mode::InFrameset,
                        "html" => Mode::BeforeHead,
                        _ => continue,
                    };
                    self.mode = mode;
                    return mode;
                }
                _ => {}
            }
            last = true;
        }
        self.mode = Mode::InBody;
        Mode::InBody
    }

    fn stop_if_fragment_or_ignore(&mut self) {
        // EOF in body: fragment case stops, document case ends parsing.
        if self.fragment_context.is_some() {
            self.stopped = true;
        }
    }

    /// Insert a foreign (SVG/MathML) element with attribute fixups.
    fn insert_foreign_adjusted(
        &mut self,
        tag: &str,
        namespace: Namespace,
        attributes: &[(String, String)],
    ) -> Option<NodeId> {
        let fixed_tag = match namespace {
            Namespace::Svg => svg_tag_fixup(tag),
            _ => tag.to_owned(),
        };
        let fixed_attrs: Vec<(String, String)> = attributes
            .iter()
            .map(|(name, value)| {
                let fixed = match namespace {
                    Namespace::Svg => svg_attr_fixup(name),
                    _ => name.clone(),
                };
                (fixed, value.clone())
            })
            .collect();
        // Foreign elements break out of <font>/<s>... wrappers per spec
        // (breakout handled by caller for the svg/math entry points).
        self.insert_namespaced(&fixed_tag, namespace, &fixed_attrs)
    }
}

// ================= free helpers =================

fn token_with(name: String, attributes: Vec<(String, String)>) -> Token {
    Token::StartTag {
        name,
        attributes,
        self_closing: false,
    }
}

/// Quirks determination from the doctype (simplified WHATWG table).
fn quirks_for_doctype(name: &str, force_quirks: bool) -> QuirksMode {
    if force_quirks {
        return QuirksMode::Quirks;
    }
    if name != "html" {
        return QuirksMode::Quirks;
    }
    QuirksMode::NoQuirks
}

/// True for spec "special" elements (used by adoption agency + end tags).
fn is_special(id: NodeId, doc: &Document) -> bool {
    match &doc.get(id).data {
        NodeData::Element(el) if el.namespace == Namespace::Html => matches!(
            el.tag_name.as_str(),
            "address"
                | "applet"
                | "area"
                | "article"
                | "aside"
                | "base"
                | "basefont"
                | "bgsound"
                | "blockquote"
                | "body"
                | "br"
                | "button"
                | "caption"
                | "center"
                | "col"
                | "colgroup"
                | "dd"
                | "details"
                | "dir"
                | "div"
                | "dl"
                | "fieldset"
                | "figcaption"
                | "figure"
                | "footer"
                | "form"
                | "frame"
                | "frameset"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "head"
                | "header"
                | "hgroup"
                | "hr"
                | "html"
                | "iframe"
                | "img"
                | "input"
                | "keygen"
                | "li"
                | "link"
                | "listing"
                | "main"
                | "marquee"
                | "menu"
                | "meta"
                | "nav"
                | "noembed"
                | "noframes"
                | "noscript"
                | "object"
                | "ol"
                | "p"
                | "param"
                | "plaintext"
                | "pre"
                | "script"
                | "search"
                | "section"
                | "select"
                | "source"
                | "style"
                | "summary"
                | "table"
                | "tbody"
                | "td"
                | "textarea"
                | "tfoot"
                | "th"
                | "thead"
                | "title"
                | "tr"
                | "track"
                | "ul"
                | "wbr"
                | "xmp"
        ),
        NodeData::Element(el) if el.namespace == Namespace::MathMl => {
            matches!(
                el.tag_name.as_str(),
                "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml"
            )
        }
        NodeData::Element(el) if el.namespace == Namespace::Svg => {
            matches!(el.tag_name.as_str(), "foreignObject" | "desc" | "title")
        }
        _ => false,
    }
}

fn svg_tag_fixup(tag: &str) -> String {
    match tag {
        "altglyph" => "altGlyph".to_owned(),
        "altglyphdef" => "altGlyphDef".to_owned(),
        "altglyphitem" => "altGlyphItem".to_owned(),
        "animatecolor" => "animateColor".to_owned(),
        "animatemotion" => "animateMotion".to_owned(),
        "animatetransform" => "animateTransform".to_owned(),
        "clippath" => "clipPath".to_owned(),
        "feblend" => "feBlend".to_owned(),
        "fecolormatrix" => "feColorMatrix".to_owned(),
        "fecomponenttransfer" => "feComponentTransfer".to_owned(),
        "fecomposite" => "feComposite".to_owned(),
        "feconvolvematrix" => "feConvolveMatrix".to_owned(),
        "fediffuselighting" => "feDiffuseLighting".to_owned(),
        "fedisplacementmap" => "feDisplacementMap".to_owned(),
        "fedistantlight" => "feDistantLight".to_owned(),
        "fedropshadow" => "feDropShadow".to_owned(),
        "feflood" => "feFlood".to_owned(),
        "fefunca" => "feFuncA".to_owned(),
        "fefuncb" => "feFuncB".to_owned(),
        "fefuncg" => "feFuncG".to_owned(),
        "fefuncr" => "feFuncR".to_owned(),
        "fegaussianblur" => "feGaussianBlur".to_owned(),
        "feimage" => "feImage".to_owned(),
        "femerge" => "feMerge".to_owned(),
        "femergenode" => "feMergeNode".to_owned(),
        "femorphology" => "feMorphology".to_owned(),
        "feoffset" => "feOffset".to_owned(),
        "fepointlight" => "fePointLight".to_owned(),
        "fespecularlighting" => "feSpecularLighting".to_owned(),
        "fespotlight" => "feSpotLight".to_owned(),
        "fetile" => "feTile".to_owned(),
        "feturbulence" => "feTurbulence".to_owned(),
        "foreignobject" => "foreignObject".to_owned(),
        "glyphref" => "glyphRef".to_owned(),
        "lineargradient" => "linearGradient".to_owned(),
        "radialgradient" => "radialGradient".to_owned(),
        "textpath" => "textPath".to_owned(),
        _ => tag.to_owned(),
    }
}

/// SVG attribute fixups (spec table).
fn svg_attr_fixup(name: &str) -> String {
    match name {
        "attributename" => "attributeName".to_owned(),
        "attributetype" => "attributeType".to_owned(),
        "basefrequency" => "baseFrequency".to_owned(),
        "baseprofile" => "baseProfile".to_owned(),
        "calcmode" => "calcMode".to_owned(),
        "clippathunits" => "clipPathUnits".to_owned(),
        "diffuseconstant" => "diffuseConstant".to_owned(),
        "edgemode" => "edgeMode".to_owned(),
        "filterunits" => "filterUnits".to_owned(),
        "glyphref" => "glyphRef".to_owned(),
        "gradienttransform" => "gradientTransform".to_owned(),
        "gradientunits" => "gradientUnits".to_owned(),
        "kernelmatrix" => "kernelMatrix".to_owned(),
        "kernelunitlength" => "kernelUnitLength".to_owned(),
        "keypoints" => "keyPoints".to_owned(),
        "keysplines" => "keySplines".to_owned(),
        "keytimes" => "keyTimes".to_owned(),
        "lengthadjust" => "lengthAdjust".to_owned(),
        "limitingconeangle" => "limitingConeAngle".to_owned(),
        "markerheight" => "markerHeight".to_owned(),
        "markerunits" => "markerUnits".to_owned(),
        "markerwidth" => "markerWidth".to_owned(),
        "maskcontentunits" => "maskContentUnits".to_owned(),
        "maskunits" => "maskUnits".to_owned(),
        "numoctaves" => "numOctaves".to_owned(),
        "pathlength" => "pathLength".to_owned(),
        "patterncontentunits" => "patternContentUnits".to_owned(),
        "patterntransform" => "patternTransform".to_owned(),
        "patternunits" => "patternUnits".to_owned(),
        "pointsatx" => "pointsAtX".to_owned(),
        "pointsaty" => "pointsAtY".to_owned(),
        "pointsatz" => "pointsAtZ".to_owned(),
        "preservealpha" => "preserveAlpha".to_owned(),
        "preserveaspectratio" => "preserveAspectRatio".to_owned(),
        "primitiveunits" => "primitiveUnits".to_owned(),
        "refx" => "refX".to_owned(),
        "refy" => "refY".to_owned(),
        "repeatcount" => "repeatCount".to_owned(),
        "repeatdur" => "repeatDur".to_owned(),
        "requiredextensions" => "requiredExtensions".to_owned(),
        "requiredfeatures" => "requiredFeatures".to_owned(),
        "specularconstant" => "specularConstant".to_owned(),
        "specularexponent" => "specularExponent".to_owned(),
        "spreadmethod" => "spreadMethod".to_owned(),
        "startoffset" => "startOffset".to_owned(),
        "stddeviation" => "stdDeviation".to_owned(),
        "stitchtiles" => "stitchTiles".to_owned(),
        "surfacescale" => "surfaceScale".to_owned(),
        "systemlanguage" => "systemLanguage".to_owned(),
        "tablevalues" => "tableValues".to_owned(),
        "targetx" => "targetX".to_owned(),
        "targety" => "targetY".to_owned(),
        "textlength" => "textLength".to_owned(),
        "viewbox" => "viewBox".to_owned(),
        "viewtarget" => "viewTarget".to_owned(),
        "xchannelselector" => "xChannelSelector".to_owned(),
        "ychannelselector" => "yChannelSelector".to_owned(),
        "zoomandpan" => "zoomAndPan".to_owned(),
        _ => name.to_owned(),
    }
}

enum FosterTarget {
    Before(NodeId),
    Child(NodeId),
}
