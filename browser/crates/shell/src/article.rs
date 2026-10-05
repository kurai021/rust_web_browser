//! Structured article view (plan/05 + plan/10 §10.2).
//!
//! Phase 2 rendering: the DOM becomes a flat list of text blocks
//! (headings, paragraphs, preformatted text, list items, rules, images)
//! with byte-offset link spans. Styling/boxes land with CSS in Phase 3+.

use css::{ComputedStyle, ComputedStyles};
use html::{Document, Namespace, NodeData, NodeId};
use std::sync::Arc;
use url::Url;

#[derive(Debug, Clone, PartialEq)]
pub struct InlineStyleSpan {
    pub start: usize,
    pub end: usize,
    pub style: Arc<ComputedStyle>,
}

/// A hyperlink: byte range inside the block text plus the target URL.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkSpan {
    /// Start byte offset in the block text.
    pub start: usize,
    /// End byte offset (exclusive) in the block text.
    pub end: usize,
    /// Absolute target URL.
    pub href: Url,
}

/// One readable block.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// Heading (`level` 1–6) with inline links.
    Heading {
        level: u8,
        text: String,
        links: Vec<LinkSpan>,
    },
    /// Paragraph with inline links.
    Paragraph { text: String, links: Vec<LinkSpan> },
    /// Preformatted text (whitespace kept).
    Pre { text: String },
    /// List item (`depth` starts at 1).
    ListItem {
        depth: u8,
        text: String,
        links: Vec<LinkSpan>,
    },
    /// Horizontal rule.
    Rule,
    /// Image placeholder (`alt` text).
    Image { alt: String },
}

/// A readable page: title plus blocks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Article {
    /// `<title>` text, if any.
    pub title: Option<String>,
    /// Blocks in document order.
    pub blocks: Vec<Block>,
    /// Page/body style and styles aligned with the existing block list.
    pub page_style: Option<Arc<ComputedStyle>>,
    pub block_styles: Vec<Option<Arc<ComputedStyle>>>,
    pub inline_styles: Vec<Vec<InlineStyleSpan>>,
}

/// Build an article from a parsed document.
///
/// Skips `head` (except `title`), `script`, `style`, `noscript` and inert
/// `template` contents. Relative links resolve against `base_url`.
#[must_use]
pub fn article_from_document(doc: &Document, base_url: &Url) -> Article {
    article_with_styles(doc, base_url, None)
}

pub fn article_from_styled_document(
    doc: &Document,
    base_url: &Url,
    styles: &ComputedStyles,
) -> Article {
    article_with_styles(doc, base_url, Some(styles))
}

fn article_with_styles(doc: &Document, base_url: &Url, styles: Option<&ComputedStyles>) -> Article {
    let mut article = Article {
        title: doc.title(),
        blocks: Vec::new(),
        ..Article::default()
    };
    let root = match doc.document_element() {
        Some(root) => root,
        None => return article,
    };
    let body = doc
        .children_of(root)
        .into_iter()
        .find(|&id| doc.is_element_named(id, "body"))
        .unwrap_or(root);
    article.page_style = styles.and_then(|styles| styles.get(body)).cloned();
    if article
        .page_style
        .as_ref()
        .is_some_and(|s| s.display == css::style::Display::None)
    {
        return article;
    }
    let mut walker = Walker {
        doc,
        base_url,
        article: &mut article,
        list_depth: 0,
        styles,
    };
    walker.walk_children(body);
    article
}

struct Walker<'a> {
    doc: &'a Document,
    base_url: &'a Url,
    article: &'a mut Article,
    list_depth: u8,
    styles: Option<&'a ComputedStyles>,
}

impl Walker<'_> {
    fn walk_children(&mut self, id: NodeId) {
        for child in self.doc.children_of(id) {
            self.walk(child);
        }
    }

    fn walk(&mut self, id: NodeId) {
        if self
            .styles
            .and_then(|s| s.get(id))
            .is_some_and(|s| s.display == css::style::Display::None)
        {
            return;
        }
        match self.doc.get(id).data.clone() {
            NodeData::Text(text) => {
                // Stray top-level text becomes a paragraph (whitespace only
                // runs are dropped to avoid blank blocks).
                if !text.trim().is_empty() {
                    self.push_text_block(text, id);
                }
            }
            NodeData::Element(el) => {
                if el.namespace != Namespace::Html {
                    return;
                }
                match el.tag_name.as_str() {
                    "head" | "script" | "style" | "noscript" | "template" | "select" | "option"
                    | "input" | "button" | "video" | "audio" | "canvas" | "iframe" | "object"
                    | "embed" => {}
                    "title" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        let level = if el.tag_name == "title" {
                            1
                        } else {
                            el.tag_name.as_bytes()[1] - b'0'
                        };
                        let (text, links, spans) = self.inline_text(id);
                        if !text.trim().is_empty() {
                            self.push_block(Block::Heading { level, text, links }, id, spans);
                        }
                    }
                    "p" | "div" | "section" | "article" | "header" | "footer" | "main" | "nav"
                    | "aside" | "figure" | "figcaption" | "blockquote" | "form" | "fieldset"
                    | "table" => {
                        self.block_children(id, BlockKind::Paragraph);
                    }
                    "pre" | "listing" | "plaintext" | "xmp" => {
                        let text = self.doc.text_content(id);
                        if !text.trim().is_empty() {
                            self.push_block(Block::Pre { text }, id, Vec::new());
                        }
                    }
                    "ul" | "ol" | "dl" => {
                        self.list_depth = self.list_depth.saturating_add(1);
                        self.walk_children(id);
                        self.list_depth = self.list_depth.saturating_sub(1);
                    }
                    "li" | "dd" | "dt" => {
                        let (text, links, spans) = self.inline_text(id);
                        if !text.trim().is_empty() {
                            self.push_block(
                                Block::ListItem {
                                    depth: self.list_depth.max(1),
                                    text,
                                    links,
                                },
                                id,
                                spans,
                            );
                        }
                    }
                    "tr" => {
                        // One row per block, cells joined (Phase 2 tables).
                        let mut cells = Vec::new();
                        for child in self.doc.children_of(id) {
                            if let NodeData::Element(cell) = &self.doc.get(child).data {
                                if cell.namespace == Namespace::Html
                                    && (cell.tag_name == "td" || cell.tag_name == "th")
                                {
                                    cells.push(self.doc.text_content(child).trim().to_owned());
                                }
                            }
                        }
                        if !cells.iter().all(|cell| cell.is_empty()) {
                            self.push_block(
                                Block::Paragraph {
                                    text: cells.join(" | "),
                                    links: Vec::new(),
                                },
                                id,
                                Vec::new(),
                            );
                        }
                    }
                    "hr" => self.push_block(Block::Rule, id, Vec::new()),
                    "br" => self.push_text_block("\n".to_owned(), id),
                    "img" => {
                        let alt = self
                            .doc
                            .get_attribute(id, "alt")
                            .unwrap_or("[image]")
                            .to_owned();
                        self.push_block(Block::Image { alt }, id, Vec::new());
                    }
                    _ => self.walk_children(id),
                }
            }
            NodeData::Comment(_) | NodeData::DocumentType { .. } | NodeData::Document => {
                self.walk_children(id);
            }
            NodeData::DocumentFragment => {}
        }
    }

    /// Emit each block-level child as its own paragraph-ish block.
    fn block_children(&mut self, id: NodeId, kind: BlockKind) {
        // Group consecutive inline content into single paragraphs; nested
        // block elements recurse as their own blocks.
        let mut items: Vec<InlineItem> = Vec::new();
        for child in self.doc.children_of(id) {
            if is_inline_node(self.doc, child) {
                self.collect_inline(child, &mut items);
            } else {
                Self::flush_items(&mut items, self, kind, id);
                self.walk(child);
            }
        }
        Self::flush_items(&mut items, self, kind, id);
    }

    /// Flush accumulated inline items as one block.
    fn flush_items(items: &mut Vec<InlineItem>, target: &mut Self, kind: BlockKind, id: NodeId) {
        let taken = std::mem::take(items);
        if taken.is_empty() {
            return;
        }
        let (text, links, ranges) = build_run(&taken);
        let spans = target.style_spans(ranges);
        if text.trim().is_empty() {
            return;
        }
        target.push_kind(kind, text, links, id, spans);
    }

    /// Inline text with link spans for `node` (transparent inline content).
    /// Whitespace is normalized in the same pass, so link offsets always
    /// match the final text.
    fn inline_text(&self, id: NodeId) -> (String, Vec<LinkSpan>, Vec<InlineStyleSpan>) {
        let mut items = Vec::new();
        self.collect_inline(id, &mut items);
        let (text, links, ranges) = build_run(&items);
        (text, links, self.style_spans(ranges))
    }

    /// Collect a flat inline stream for `id` (recurses transparently).
    fn collect_inline(&self, id: NodeId, items: &mut Vec<InlineItem>) {
        if self
            .styles
            .and_then(|s| s.get(id))
            .is_some_and(|s| s.display == css::style::Display::None)
        {
            return;
        }
        let element = matches!(self.doc.get(id).data, NodeData::Element(_));
        if element && self.styles.is_some() {
            items.push(InlineItem::BeginStyle(id));
        }
        match self.doc.get(id).data.clone() {
            NodeData::Text(chunk) => {
                let style = self
                    .doc
                    .get(id)
                    .parent
                    .and_then(|p| self.styles.and_then(|s| s.get(p)));
                items.push(InlineItem::Text(transform_text(
                    &chunk,
                    style.map(|s| &**s),
                )));
            }
            NodeData::Element(el) if el.namespace == Namespace::Html => {
                match el.tag_name.as_str() {
                    "script" | "style" | "noscript" => {}
                    "br" => items.push(InlineItem::Text("\n".to_owned())),
                    "a" => {
                        let href = self
                            .doc
                            .get_attribute(id, "href")
                            .and_then(|raw| self.base_url.join(raw).ok());
                        match href {
                            Some(href) => {
                                items.push(InlineItem::BeginLink(href));
                                for child in self.doc.children_of(id) {
                                    self.collect_inline(child, items);
                                }
                                items.push(InlineItem::EndLink);
                            }
                            None => {
                                for child in self.doc.children_of(id) {
                                    self.collect_inline(child, items);
                                }
                            }
                        }
                    }
                    _ => {
                        for child in self.doc.children_of(id) {
                            self.collect_inline(child, items);
                        }
                    }
                }
            }
            _ => {}
        }
        if element && self.styles.is_some() {
            items.push(InlineItem::EndStyle);
        }
    }

    fn push_text_block(&mut self, text: String, id: NodeId) {
        let (text, _, _) = build_run(&[InlineItem::Text(text)]);
        if text.trim().is_empty() {
            return;
        }
        self.push_block(
            Block::Paragraph {
                text,
                links: Vec::new(),
            },
            id,
            Vec::new(),
        );
    }

    fn push_kind(
        &mut self,
        kind: BlockKind,
        text: String,
        links: Vec<LinkSpan>,
        id: NodeId,
        spans: Vec<InlineStyleSpan>,
    ) {
        match kind {
            BlockKind::Paragraph => {
                self.push_block(Block::Paragraph { text, links }, id, spans);
            }
        }
    }

    fn push_block(&mut self, block: Block, id: NodeId, spans: Vec<InlineStyleSpan>) {
        let source = if matches!(self.doc.get(id).data, NodeData::Element(_)) {
            Some(id)
        } else {
            self.doc.get(id).parent
        };
        self.article.block_styles.push(
            source
                .and_then(|id| self.styles.and_then(|s| s.get(id)))
                .cloned(),
        );
        self.article.inline_styles.push(spans);
        self.article.blocks.push(block);
    }

    fn style_spans(&self, ranges: Vec<(NodeId, usize, usize)>) -> Vec<InlineStyleSpan> {
        ranges
            .into_iter()
            .filter_map(|(id, start, end)| {
                Some(InlineStyleSpan {
                    start,
                    end,
                    style: self.styles?.get(id)?.clone(),
                })
            })
            .collect()
    }
}

fn transform_text(text: &str, style: Option<&ComputedStyle>) -> String {
    match style.map(|s| s.text_transform.as_str()) {
        Some("uppercase") => text.to_uppercase(),
        Some("lowercase") => text.to_lowercase(),
        Some("capitalize") => {
            let mut start = true;
            let mut out = String::new();
            for ch in text.chars() {
                if ch.is_whitespace() {
                    start = true;
                    out.push(ch);
                } else if start {
                    out.extend(ch.to_uppercase());
                    start = false;
                } else {
                    out.push(ch);
                }
            }
            out
        }
        _ => text.to_owned(),
    }
}

#[derive(Debug, Clone, Copy)]
enum BlockKind {
    Paragraph,
}

/// Flat inline stream: text chunks plus link boundaries.
#[derive(Debug, Clone)]
enum InlineItem {
    Text(String),
    BeginLink(Url),
    EndLink,
    BeginStyle(NodeId),
    EndStyle,
}

/// True for text and transparent inline elements (links, phrasing).
fn is_inline_node(doc: &Document, id: NodeId) -> bool {
    match &doc.get(id).data {
        NodeData::Text(_) => true,
        NodeData::Element(el) if el.namespace == Namespace::Html => matches!(
            el.tag_name.as_str(),
            "a" | "abbr"
                | "b"
                | "bdi"
                | "bdo"
                | "br"
                | "cite"
                | "code"
                | "data"
                | "dfn"
                | "em"
                | "i"
                | "kbd"
                | "mark"
                | "q"
                | "rp"
                | "rt"
                | "ruby"
                | "s"
                | "samp"
                | "small"
                | "span"
                | "strong"
                | "sub"
                | "sup"
                | "time"
                | "u"
                | "var"
                | "wbr"
                | "img"
                | "input"
                | "select"
                | "button"
                | "label"
        ),
        _ => false,
    }
}

/// Build normalized text plus link spans from an inline stream.
/// Whitespace collapses in the same pass, so offsets always match.
fn build_run(items: &[InlineItem]) -> (String, Vec<LinkSpan>, Vec<(NodeId, usize, usize)>) {
    let mut out = String::new();
    let mut links = Vec::new();
    let mut open: Vec<(Url, usize)> = Vec::new();
    let mut pending_space = false;
    let mut style_stack = Vec::new();
    let mut ranges = Vec::new();
    let push_char = |ch: char, out: &mut String, pending_space: &mut bool| {
        if ch == '\n' {
            // Hard break: drop pending spaces, keep one newline.
            if !out.ends_with('\n') {
                out.push('\n');
            }
            *pending_space = false;
        } else if ch == ' ' || ch == '\t' || ch == '\x0C' || ch == '\r' {
            *pending_space = true;
        } else {
            if *pending_space && !out.is_empty() && !out.ends_with('\n') {
                out.push(' ');
            }
            *pending_space = false;
            out.push(ch);
        }
    };
    for item in items {
        match item {
            InlineItem::Text(chunk) => {
                for ch in chunk.chars() {
                    push_char(ch, &mut out, &mut pending_space);
                }
            }
            InlineItem::BeginLink(href) => {
                // Inter-word space belongs outside the span.
                if pending_space && !out.is_empty() && !out.ends_with('\n') {
                    out.push(' ');
                }
                pending_space = false;
                open.push((href.clone(), out.len()));
            }
            InlineItem::EndLink => {
                if let Some((href, start)) = open.pop() {
                    if start < out.len() {
                        links.push(LinkSpan {
                            start,
                            end: out.len(),
                            href,
                        });
                    }
                }
            }
            InlineItem::BeginStyle(id) => {
                if pending_space && !out.is_empty() && !out.ends_with('\n') {
                    out.push(' ');
                }
                pending_space = false;
                style_stack.push((*id, out.len()));
            }
            InlineItem::EndStyle => {
                if let Some((id, start)) = style_stack.pop() {
                    if start < out.len() {
                        ranges.push((id, start, out.len()));
                    }
                }
            }
        }
    }
    // Unclosed links (malformed nesting) end at the run end.
    for (href, start) in open {
        if start < out.len() {
            links.push(LinkSpan {
                start,
                end: out.len(),
                href,
            });
        }
    }
    links.sort_by_key(|link| link.start);
    ranges.sort_by_key(|(_, start, end)| (*start, std::cmp::Reverse(*end)));
    (out, links, ranges)
}

// ---------- block view (rendering) ----------

use crate::text::{Canvas, TextBuffer};
use cosmic_text::{Color, FontSystem, SwashCache, Wrap};

/// Accent ink for links (matches shell chrome).
pub const LINK_INK: Color = Color::rgb(0x0B, 0x5D, 0xC2);

/// One laid-out overlay fragment for a link slice.
struct Overlay {
    buffer: TextBuffer,
    x: f32,
    y: f32,
    width: f32,
    baseline: f32,
}

/// A block plus its laid-out text, link overlays and metrics.
pub struct BlockView {
    buffer: TextBuffer,
    text: String,
    links: Vec<LinkSpan>,
    overlays: Vec<Overlay>,
    /// Top offset inside the article (set by layout).
    pub y: f32,
    /// Laid-out height including trailing gap.
    pub height: f32,
    style: Option<Arc<ComputedStyle>>,
    x_offset: f32,
    text_origin: (f32, f32),
    box_size: (f32, f32),
    decorations: Vec<(f32, f32, f32, css::values::Color)>,
    link_rects: Vec<(Url, f32, f32, f32, f32)>,
}

impl BlockView {
    /// Build from a block at `width` px (shapes immediately).
    pub fn layout(font_system: &mut FontSystem, block: &Block, width: f32, y: f32) -> Self {
        let (size, text, links) = match block {
            Block::Heading { level, text, links } => {
                let size = match level {
                    1 => 24.0,
                    2 => 21.0,
                    3 => 18.0,
                    _ => 16.0,
                };
                (size, text.clone(), links.clone())
            }
            Block::Paragraph { text, links } => (15.0, text.clone(), links.clone()),
            Block::Pre { text } => (14.0, text.clone(), Vec::new()),
            Block::ListItem { depth, text, links } => {
                let bullet = format!("{}• ", "  ".repeat(depth.saturating_sub(1) as usize));
                let mut full = bullet;
                let shift = full.len();
                full.push_str(text);
                let shifted: Vec<LinkSpan> = links
                    .iter()
                    .map(|link| LinkSpan {
                        start: link.start + shift,
                        end: link.end + shift,
                        href: link.href.clone(),
                    })
                    .collect();
                (15.0, full, shifted)
            }
            Block::Rule => (15.0, "─".repeat(40), Vec::new()),
            Block::Image { alt } => (14.0, format!("[image: {alt}]"), Vec::new()),
        };
        let mut buffer = TextBuffer::with_metrics(font_system, size, Wrap::Word);
        buffer.set_text(font_system, &text);
        buffer.set_size(font_system, width, 10_000.0);
        let text_h = buffer.full_height(font_system);
        buffer.set_size(font_system, width, text_h.max(size * 1.3));
        // Link overlays (accent slices + underline geometry).
        let mut overlays = Vec::new();
        for link in &links {
            for (fx, ftop, fbase, fwidth, slice) in
                buffer.link_fragments(font_system, &text, link.start, link.end)
            {
                let mut overlay = TextBuffer::with_metrics(font_system, size, Wrap::None);
                overlay.set_color(LINK_INK);
                overlay.set_text(font_system, &slice);
                overlay.set_size(font_system, fwidth.max(1.0), size * 1.3);
                let over_top = overlay.first_line_top(font_system).unwrap_or(0.0);
                overlays.push(Overlay {
                    buffer: overlay,
                    x: fx,
                    y: ftop - over_top,
                    width: fwidth,
                    baseline: fbase,
                });
            }
        }
        let gap = match block {
            Block::Heading { .. } => 14.0,
            Block::Rule => 12.0,
            _ => 10.0,
        };
        let height = text_h + gap;
        Self {
            buffer,
            text,
            links,
            overlays,
            y,
            height,
            style: None,
            x_offset: 0.0,
            text_origin: (0.0, 0.0),
            box_size: (width, text_h),
            decorations: Vec::new(),
            link_rects: Vec::new(),
        }
    }

    /// Phase 3 uses the same block renderer with computed typography and
    /// simple vertical boxes. Full nested flow layout belongs to Phase 4.
    pub fn layout_styled(
        font_system: &mut FontSystem,
        block: &Block,
        width: f32,
        y: f32,
        style: Arc<ComputedStyle>,
        spans: &[InlineStyleSpan],
        env: css::Environment,
    ) -> Self {
        let mut view = Self::layout(font_system, block, width, y);
        let ctx = env.length_context(style.font_size, 16.0, width);
        let margin: [f32; 4] = std::array::from_fn(|i| style.margin[i].resolve(ctx).unwrap_or(0.0));
        let padding: [f32; 4] =
            std::array::from_fn(|i| style.padding[i].resolve(ctx).unwrap_or(0.0).max(0.0));
        let border: [f32; 4] =
            std::array::from_fn(|i| style.border_width[i].resolve(ctx).unwrap_or(0.0).max(0.0));
        let horizontal = padding[1] + padding[3] + border[1] + border[3];
        let available = (width - margin[1] - margin[3]).max(1.0);
        let specified = style.width.resolve(ctx);
        let mut content_width = specified
            .map(|w| if style.border_box { w - horizontal } else { w })
            .unwrap_or(available - horizontal)
            .max(1.0);
        if let Some(max) = style.max_width.resolve(ctx) {
            content_width = content_width.min(max.max(1.0));
        }
        if let Some(min) = style.min_width.resolve(ctx) {
            content_width = content_width.max(min.max(1.0));
        }
        content_width = content_width.min(available).max(1.0);
        let box_width = content_width + horizontal;
        let remaining = (width - box_width - margin[1] - margin[3]).max(0.0);
        let left_auto = matches!(style.margin[3], css::values::SizeValue::Auto);
        let right_auto = matches!(style.margin[1], css::values::SizeValue::Auto);
        view.x_offset = margin[3]
            + if left_auto && right_auto {
                remaining / 2.0
            } else if left_auto {
                remaining
            } else {
                0.0
            };
        view.text_origin = (padding[3] + border[3], padding[0] + border[0]);
        // List bullets shift inline ranges by their UTF-8 byte length.
        let shifted = if let Block::ListItem { depth, .. } = block {
            let offset = format!("{}• ", "  ".repeat(depth.saturating_sub(1) as usize)).len();
            spans
                .iter()
                .map(|s| InlineStyleSpan {
                    start: s.start + offset,
                    end: s.end + offset,
                    style: s.style.clone(),
                })
                .collect::<Vec<_>>()
        } else {
            spans.to_vec()
        };
        view.buffer
            .set_size(font_system, content_width, 1_000_000.0);
        view.buffer
            .set_styled_text(font_system, &view.text, &style, &shifted);
        view.buffer.set_alignment(font_system, style.text_align);
        let text_height = view.buffer.full_height(font_system);
        let content_height = style.height.resolve(ctx).unwrap_or(text_height).max(0.0);
        let box_height = content_height + padding[0] + padding[2] + border[0] + border[2];
        view.buffer
            .set_size(font_system, content_width, text_height.max(0.1));
        // Metrics/style changes invalidate the Phase 2 substring overlays.
        // Accent colors now come from the actual anchor's computed style.
        view.overlays.clear();
        let mut decoration_ranges = vec![(0, view.text.len(), style.clone())];
        decoration_ranges.extend(
            shifted
                .iter()
                .map(|span| (span.start, span.end, span.style.clone())),
        );
        for (start, end, style) in decoration_ranges {
            if style.text_decoration == "none" || !style.visible {
                continue;
            }
            for (x, top, baseline, w, line_height) in
                view.buffer.range_rects(font_system, start, end)
            {
                if style.text_decoration.contains("underline") {
                    view.decorations.push((x, baseline + 2.0, w, style.color));
                }
                if style.text_decoration.contains("overline") {
                    view.decorations.push((x, top, w, style.color));
                }
                if style.text_decoration.contains("line-through") {
                    view.decorations
                        .push((x, top + line_height * 0.5, w, style.color));
                }
            }
        }
        for link in &view.links {
            for (x, top, _, w, h) in view.buffer.range_rects(font_system, link.start, link.end) {
                view.link_rects.push((link.href.clone(), x, top, w, h));
            }
        }
        view.y = y + margin[0];
        view.height = margin[0] + box_height + margin[2];
        view.box_size = (box_width, box_height);
        view.style = Some(style);
        view
    }

    /// Paint the block at `(x, y_offset)` (viewport origin).
    pub fn draw(
        &mut self,
        font_system: &mut FontSystem,
        cache: &mut SwashCache,
        canvas: &mut Canvas<'_>,
        x: i32,
        y_offset: i32,
    ) {
        let base_y = y_offset + self.y as i32;
        let x = x + self.x_offset as i32;
        if let Some(style) = &self.style {
            if style.visible {
                let bg = style.background_color;
                let (w, h) = self.box_size;
                canvas.blend_rect(
                    x,
                    base_y,
                    w.max(0.0) as u32,
                    h.max(0.0) as u32,
                    (bg.r, bg.g, bg.b, (bg.a as f32 * style.opacity) as u8),
                );
                let ctx = css::values::LengthContext {
                    percentage_basis: w,
                    font_size: style.font_size,
                    ..Default::default()
                };
                let borders: [i32; 4] = std::array::from_fn(|i| {
                    style.border_width[i].resolve(ctx).unwrap_or(0.0).max(0.0) as i32
                });
                for (i, (rx, ry, rw, rh)) in [
                    (x, base_y, w as i32, borders[0]),
                    (x + w as i32 - borders[1], base_y, borders[1], h as i32),
                    (x, base_y + h as i32 - borders[2], w as i32, borders[2]),
                    (x, base_y, borders[3], h as i32),
                ]
                .into_iter()
                .enumerate()
                {
                    let c = style.border_color[i];
                    canvas.blend_rect(
                        rx,
                        ry,
                        rw.max(0) as u32,
                        rh.max(0) as u32,
                        (c.r, c.g, c.b, c.a),
                    );
                }
            }
        }
        let text_x = x + self.text_origin.0 as i32;
        let text_y = base_y + self.text_origin.1 as i32;
        self.buffer.draw(font_system, cache, canvas, text_x, text_y);
        for (dx, dy, w, color) in &self.decorations {
            canvas.blend_rect(
                text_x + *dx as i32,
                text_y + *dy as i32,
                w.max(1.0) as u32,
                1,
                (color.r, color.g, color.b, color.a),
            );
        }
        for overlay in &mut self.overlays {
            overlay.buffer.draw(
                font_system,
                cache,
                canvas,
                x + overlay.x as i32,
                base_y + overlay.y as i32,
            );
            // Underline under the slice baseline.
            canvas.fill_rect(
                x + overlay.x as i32,
                base_y + overlay.baseline as i32 + 2,
                overlay.width.max(1.0) as i32,
                2,
                0x0B5DC2,
            );
        }
    }

    /// Link whose span contains buffer-local `(x, y)`, if any.
    pub fn hit_link(&mut self, font_system: &mut FontSystem, x: f32, y: f32) -> Option<Url> {
        if self.style.is_some() {
            let x = x - self.x_offset - self.text_origin.0;
            let y = y - self.text_origin.1;
            return self
                .link_rects
                .iter()
                .find(|(_, left, top, w, h)| {
                    x >= *left && x < *left + *w && y >= *top && y < *top + *h
                })
                .map(|(url, _, _, _, _)| url.clone());
        }
        let offset = self.buffer.hit_offset(font_system, x, y)?;
        self.links
            .iter()
            .find(|link| offset >= link.start && offset < link.end)
            .map(|link| link.href.clone())
    }

    /// Test hook: laid-out fragments of one link span in `text`.
    #[cfg(test)]
    pub fn link_fragments_for_test(
        &mut self,
        font_system: &mut FontSystem,
        text: &str,
        start: usize,
        end: usize,
    ) -> Vec<(f32, f32, f32, f32, String)> {
        self.buffer.link_fragments(font_system, text, start, end)
    }

    /// Test hook: the view's link spans.
    #[cfg(test)]
    pub fn links_for_test(&self) -> &[LinkSpan] {
        &self.links
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn article(html: &str) -> Article {
        let url = Url::parse("https://example.com/dir/page").unwrap();
        let doc = html::parse_full(html.as_bytes(), &url, html::ParseOpts::default());
        article_from_document(&doc, &url)
    }

    #[test]
    fn headings_paragraphs_and_title() {
        let article =
            article("<title>T</title><h1>Hi</h1><p>Hello <b>world</b></p><script>evil()</script>");
        assert_eq!(article.title.as_deref(), Some("T"));
        assert_eq!(article.blocks.len(), 2);
        assert!(matches!(
            &article.blocks[0],
            Block::Heading { level: 1, text, .. } if text == "Hi"
        ));
        assert!(matches!(
            &article.blocks[1],
            Block::Paragraph { text, .. } if text == "Hello world"
        ));
    }

    #[test]
    fn links_resolve_relative() {
        let article = article("<p>Go <a href=\"/other\">there</a> now</p>");
        match &article.blocks[0] {
            Block::Paragraph { text, links } => {
                assert_eq!(text, "Go there now");
                assert_eq!(links.len(), 1);
                assert_eq!(links[0].href.as_str(), "https://example.com/other");
                assert_eq!(&text[links[0].start..links[0].end], "there");
            }
            other => panic!("unexpected block {other:?}"),
        }
    }

    #[test]
    fn lists_and_pre() {
        let article = article("<ul><li>a</li><li>b</li></ul><pre>x  y</pre>");
        assert!(matches!(
            &article.blocks[0],
            Block::ListItem { depth: 1, text, .. } if text == "a"
        ));
        assert!(matches!(&article.blocks[2], Block::Pre { text } if text == "x  y"));
    }

    #[test]
    fn tables_become_row_blocks() {
        let article = article("<table><tr><td>a<td>b</table>");
        assert!(matches!(
            &article.blocks[0],
            Block::Paragraph { text, .. } if text == "a | b"
        ));
    }

    #[test]
    fn link_overlay_roundtrip() {
        use cosmic_text::{FontSystem, SwashCache};

        let url = Url::parse("https://example.com/").unwrap();
        let doc = html::parse_full(
            b"<h1>T</h1><p>Go to <a href=\"/s\">second page</a> now, and a wrapped <a href=\"/t\">link spanning several words here</a> end.</p>",
            &url,
            html::ParseOpts::default(),
        );
        let article = article_from_document(&doc, &url);
        let mut font_system = FontSystem::new();
        let _cache = SwashCache::new();
        // Narrow width forces wrapping (multi-line link fragments).
        let width = 260.0;
        let mut views: Vec<BlockView> = article
            .blocks
            .iter()
            .map(|block| BlockView::layout(&mut font_system, block, width, 0.0))
            .collect();
        assert_eq!(views.len(), 2);
        // Every link span must be clickable at its own fragment centers.
        for (block, view) in article.blocks.iter().zip(views.iter_mut()) {
            let text = match block {
                Block::Heading { text, .. } | Block::Paragraph { text, .. } => text.clone(),
                _ => String::new(),
            };
            let spans: Vec<(Url, usize, usize)> = view
                .links_for_test()
                .iter()
                .map(|link| (link.href.clone(), link.start, link.end))
                .collect();
            for (href, start, end) in spans {
                let fragments = view.link_fragments_for_test(&mut font_system, &text, start, end);
                assert!(!fragments.is_empty(), "link has fragments");
                for (fx, ftop, _, fwidth, _) in fragments {
                    let hit = view.hit_link(&mut font_system, fx + fwidth / 2.0, ftop + 8.0);
                    assert_eq!(hit.as_ref(), Some(&href));
                }
            }
        }
    }

    #[test]
    fn computed_styles_change_pixels_spacing_and_keep_shaped_links() {
        let url = Url::parse("https://example.com/").unwrap();
        let doc=html::parse_full(b"<p id='p' style='color:#135;font-size:24px;padding:8px;margin:12px;border:2px solid red'>Text <a href='/next'>Link</a><span style='display:none'>SECRET</span></p>",&url,html::ParseOpts::default());
        let sheet=css::parse_stylesheet("a{color:green;text-decoration:underline} @media(max-width:450px){#p{font-size:12px!important}}",None);
        let mut cascade = css::Cascade::default();
        let styles = cascade.compute(&doc, std::slice::from_ref(&sheet));
        let article = article_from_styled_document(&doc, &url, &styles);
        assert_eq!(article.blocks.len(), 1);
        assert!(matches!(&article.blocks[0],Block::Paragraph {text,..} if text=="Text Link"));
        let style = article.block_styles[0].as_ref().unwrap();
        assert_eq!(style.font_size, 24.0);
        let mut fonts = FontSystem::new();
        let mut view = BlockView::layout_styled(
            &mut fonts,
            &article.blocks[0],
            350.0,
            0.0,
            style.clone(),
            &article.inline_styles[0],
            cascade.environment,
        );
        assert_eq!(view.text_origin, (10.0, 10.0));
        assert_eq!(view.y, 12.0);
        assert!(!view.link_rects.is_empty());
        let (url, left, top, w, h) = view.link_rects[0].clone();
        let hit = view.hit_link(
            &mut fonts,
            view.x_offset + view.text_origin.0 + left + w / 2.0,
            view.text_origin.1 + top + h / 2.0,
        );
        assert_eq!(hit.as_ref(), Some(&url));
        let mut pixels = vec![0xffffff; 400 * 200];
        let mut canvas = Canvas {
            w: 400,
            h: 200,
            pixels: &mut pixels,
        };
        view.draw(&mut fonts, &mut SwashCache::new(), &mut canvas, 0, 0);
        assert_eq!(canvas.pixels[12 * 400 + 12], 0xff0000); // actual border pixel
        cascade.environment.width = 400.0;
        let narrow_styles = cascade.compute(&doc, &[sheet]);
        let narrow = article_from_styled_document(&doc, &url, &narrow_styles);
        assert_eq!(narrow.block_styles[0].as_ref().unwrap().font_size, 12.0);
        let smaller = BlockView::layout_styled(
            &mut fonts,
            &narrow.blocks[0],
            350.0,
            0.0,
            narrow.block_styles[0].clone().unwrap(),
            &narrow.inline_styles[0],
            cascade.environment,
        );
        assert!(smaller.height < view.height);
    }
}
