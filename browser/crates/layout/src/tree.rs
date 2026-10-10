//! Render tree generation and anonymous block/inline splitting.

use crate::MAX_BOX_DEPTH;
use css::selectors::PseudoElement;
use css::style::Display;
use css::{ComputedStyle, ComputedStyles};
use html::{Document, NodeData, NodeId};
use std::sync::Arc;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ElementId {
    pub node: NodeId,
    pub pseudo: Option<PseudoElement>,
}

#[derive(Debug, Clone)]
pub enum Replaced {
    Image {
        url: Option<Url>,
        alt: String,
        width: Option<f32>,
        height: Option<f32>,
    },
    Video {
        width: Option<f32>,
        height: Option<f32>,
    },
    Input {
        label: String,
    },
}

#[derive(Debug, Clone)]
pub enum RenderKind {
    Block,
    Inline,
    InlineBlock,
    AnonymousBlock,
    Text(String),
    Break,
    Replaced(Replaced),
}

#[derive(Debug, Clone)]
pub struct RenderNode {
    pub element: ElementId,
    pub kind: RenderKind,
    pub style: Arc<ComputedStyle>,
    pub children: Vec<usize>,
    pub href: Option<Url>,
}
impl RenderNode {
    pub fn is_block(&self) -> bool {
        matches!(self.kind, RenderKind::Block | RenderKind::AnonymousBlock)
            || (matches!(self.kind, RenderKind::Replaced(_))
                && matches!(self.style.display, Display::Block | Display::ListItem))
    }
}

#[derive(Debug, Clone, Default)]
pub struct RenderTree {
    pub nodes: Vec<RenderNode>,
    pub roots: Vec<usize>,
    pub depth_fallbacks: usize,
}

pub fn build_render_tree(dom: &Document, styles: &ComputedStyles) -> RenderTree {
    let base = dom
        .get_elements_by_tag_name("base")
        .into_iter()
        .find_map(|id| {
            dom.get_attribute(id, "href")
                .and_then(|raw| dom.url.as_ref()?.join(raw).ok())
        })
        .or_else(|| dom.url.clone());
    let mut builder = Builder {
        dom,
        styles,
        base,
        tree: RenderTree::default(),
    };
    let root_style = Arc::new(ComputedStyle::default());
    if let Some(root) = dom.document_element() {
        builder.tree.roots = builder.walk(root, root_style, None, 0);
    }
    builder.tree
}

struct Builder<'a> {
    dom: &'a Document,
    styles: &'a ComputedStyles,
    base: Option<Url>,
    tree: RenderTree,
}
enum Walk {
    Enter {
        id: NodeId,
        inherited: Arc<ComputedStyle>,
        href: Option<Url>,
        depth: usize,
    },
    Finish {
        node: RenderNode,
        child_count: usize,
    },
}
impl Builder<'_> {
    fn insert(&mut self, node: RenderNode) -> usize {
        let id = self.tree.nodes.len();
        self.tree.nodes.push(node);
        id
    }
    fn walk(
        &mut self,
        id: NodeId,
        inherited: Arc<ComputedStyle>,
        href: Option<Url>,
        depth: usize,
    ) -> Vec<usize> {
        let mut work = vec![Walk::Enter {
            id,
            inherited,
            href,
            depth,
        }];
        let mut results: Vec<Vec<usize>> = Vec::new();
        while let Some(frame) = work.pop() {
            match frame {
                Walk::Enter {
                    id,
                    inherited,
                    href,
                    depth,
                } => {
                    let Some(mut node) = self.prepare(id, inherited, href) else {
                        results.push(Vec::new());
                        continue;
                    };
                    let children = if depth >= MAX_BOX_DEPTH - 2 {
                        self.tree.depth_fallbacks += 1;
                        node.kind = RenderKind::Block;
                        let child = self.insert(RenderNode {
                            element: node.element,
                            kind: RenderKind::Text(self.dom.text_content(id)),
                            style: node.style.clone(),
                            href: node.href.clone(),
                            children: Vec::new(),
                        });
                        node.children.push(child);
                        Vec::new()
                    } else if matches!(
                        node.kind,
                        RenderKind::Text(_) | RenderKind::Replaced(_) | RenderKind::Break
                    ) {
                        Vec::new()
                    } else {
                        if node.style.display == Display::ListItem {
                            let marker = self
                                .styles
                                .pseudo(id, PseudoElement::Marker)
                                .cloned()
                                .unwrap_or(node.style.clone());
                            let text = if marker.content.is_empty() {
                                "• ".into()
                            } else {
                                marker.content.clone()
                            };
                            let child = self.insert(RenderNode {
                                element: ElementId {
                                    node: id,
                                    pseudo: Some(PseudoElement::Marker),
                                },
                                kind: RenderKind::Text(text),
                                style: marker,
                                href: None,
                                children: Vec::new(),
                            });
                            node.children.push(child);
                        }
                        self.pseudo(&mut node, PseudoElement::Before);
                        self.dom.get(id).children.clone()
                    };
                    let inherited = node.style.clone();
                    let href = node.href.clone();
                    work.push(Walk::Finish {
                        node,
                        child_count: children.len(),
                    });
                    for child in children.into_iter().rev() {
                        work.push(Walk::Enter {
                            id: child,
                            inherited: inherited.clone(),
                            href: href.clone(),
                            depth: depth + 1,
                        });
                    }
                }
                Walk::Finish {
                    mut node,
                    child_count,
                } => {
                    let from = results.len().saturating_sub(child_count);
                    for children in results.drain(from..) {
                        node.children.extend(children);
                    }
                    if !matches!(
                        node.kind,
                        RenderKind::Text(_) | RenderKind::Replaced(_) | RenderKind::Break
                    ) {
                        self.pseudo(&mut node, PseudoElement::After);
                    }
                    let normalized = if node.style.display == Display::Contents
                        && matches!(node.kind, RenderKind::Inline)
                    {
                        node.children
                    } else {
                        self.normalize(node)
                    };
                    results.push(normalized);
                }
            }
        }
        results.pop().unwrap_or_default()
    }
    fn prepare(
        &self,
        id: NodeId,
        inherited: Arc<ComputedStyle>,
        href: Option<Url>,
    ) -> Option<RenderNode> {
        let element = ElementId {
            node: id,
            pseudo: None,
        };
        let mut node = RenderNode {
            element,
            kind: RenderKind::Inline,
            style: inherited,
            children: Vec::new(),
            href,
        };
        match &self.dom.get(id).data {
            NodeData::Text(text) => {
                if text.is_empty() {
                    return None;
                }
                node.kind = RenderKind::Text(text.clone());
            }
            NodeData::Element(el) => {
                if matches!(
                    el.tag_name.as_str(),
                    "template" | "script" | "style" | "head"
                ) {
                    return None;
                }
                node.style = self.styles.get(id).cloned().unwrap_or(node.style);
                if node.style.display == Display::None {
                    return None;
                }
                if el.tag_name == "a" {
                    node.href = self
                        .dom
                        .get_attribute(id, "href")
                        .and_then(|s| self.base.as_ref()?.join(s).ok());
                }
                let dim = |name| {
                    self.dom
                        .get_attribute(id, name)
                        .and_then(|s| s.trim().parse::<f32>().ok())
                        .filter(|n| n.is_finite() && *n >= 0.0)
                        .map(|n| n.min(crate::MAX_EXTENT))
                };
                let replaced = match el.tag_name.as_str() {
                    "img" => Some(Replaced::Image {
                        url: self
                            .dom
                            .get_attribute(id, "src")
                            .and_then(|s| self.base.as_ref()?.join(s).ok()),
                        alt: self
                            .dom
                            .get_attribute(id, "alt")
                            .unwrap_or("[image]")
                            .to_owned(),
                        width: dim("width"),
                        height: dim("height"),
                    }),
                    "video" => Some(Replaced::Video {
                        width: dim("width"),
                        height: dim("height"),
                    }),
                    "input" => {
                        if self.dom.get_attribute(id, "type") == Some("hidden") {
                            return None;
                        }
                        let value = self.dom.control_value(id);
                        Some(Replaced::Input {
                            label: if value.is_empty() {
                                self.dom.get_attribute(id, "placeholder").unwrap_or("")
                            } else {
                                value
                            }
                            .to_owned(),
                        })
                    }
                    _ => None,
                };
                node.kind = if let Some(replaced) = replaced {
                    RenderKind::Replaced(replaced)
                } else if el.tag_name == "br" {
                    RenderKind::Break
                } else {
                    match node.style.display {
                        Display::Block
                        | Display::ListItem
                        | Display::Table
                        | Display::Flex
                        | Display::Grid => RenderKind::Block,
                        Display::InlineBlock => RenderKind::InlineBlock,
                        _ => RenderKind::Inline,
                    }
                };
            }
            _ => return None,
        }
        Some(node)
    }
    fn pseudo(&mut self, node: &mut RenderNode, pseudo: PseudoElement) {
        if let Some(style) = self.styles.pseudo(node.element.node, pseudo) {
            let value = style.get_property_value("content");
            if style.display != Display::None && !matches!(value.as_str(), "normal" | "none" | "") {
                let element = ElementId {
                    node: node.element.node,
                    pseudo: Some(pseudo),
                };
                let text = self.insert(RenderNode {
                    element,
                    kind: RenderKind::Text(style.content.clone()),
                    style: style.clone(),
                    href: node.href.clone(),
                    children: Vec::new(),
                });
                let generated = RenderNode {
                    element,
                    kind: if style.display == Display::Block {
                        RenderKind::Block
                    } else {
                        RenderKind::Inline
                    },
                    style: style.clone(),
                    href: node.href.clone(),
                    children: vec![text],
                };
                node.children.extend(self.normalize(generated));
            }
        }
    }
    fn normalize(&mut self, mut node: RenderNode) -> Vec<usize> {
        let children = std::mem::take(&mut node.children);
        if (node.is_block() || matches!(node.kind, RenderKind::InlineBlock))
            && children.iter().any(|&id| self.tree.nodes[id].is_block())
        {
            let mut inline = Vec::new();
            for id in children {
                if self.tree.nodes[id].is_block() {
                    self.flush_anonymous(&mut node, &mut inline);
                    node.children.push(id);
                } else {
                    inline.push(id);
                }
            }
            self.flush_anonymous(&mut node, &mut inline);
        } else if matches!(node.kind, RenderKind::Inline)
            && children.iter().any(|&id| self.tree.nodes[id].is_block())
        {
            let mut result = Vec::new();
            for id in children {
                if self.tree.nodes[id].is_block() {
                    if !node.children.is_empty() {
                        result.push(self.insert(node.clone()));
                        node.children.clear();
                    }
                    result.push(id);
                } else {
                    node.children.push(id);
                }
            }
            if !node.children.is_empty() {
                result.push(self.insert(node));
            }
            return result;
        } else {
            node.children = children;
        }
        vec![self.insert(node)]
    }
    fn flush_anonymous(&mut self, node: &mut RenderNode, inline: &mut Vec<usize>) {
        if inline.is_empty() {
            return;
        }
        let id = self.insert(RenderNode {
            element: node.element,
            kind: RenderKind::AnonymousBlock,
            style: node.style.clone(),
            href: node.href.clone(),
            children: std::mem::take(inline),
        });
        node.children.push(id);
    }
}
