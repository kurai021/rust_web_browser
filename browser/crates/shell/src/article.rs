//! Structured article view (plan/05 + plan/10 §10.2).
//!
//! Phase 2 rendering: the DOM becomes a flat list of text blocks
//! (headings, paragraphs, preformatted text, list items, rules, images)
//! with byte-offset link spans. Styling/boxes land with CSS in Phase 3+.

use html::{Document, Namespace, NodeData, NodeId};
use url::Url;

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
}

/// Build an article from a parsed document.
///
/// Skips `head` (except `title`), `script`, `style`, `noscript` and inert
/// `template` contents. Relative links resolve against `base_url`.
#[must_use]
pub fn article_from_document(doc: &Document, base_url: &Url) -> Article {
    let mut article = Article {
        title: doc.title(),
        blocks: Vec::new(),
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
    let mut walker = Walker {
        doc,
        base_url,
        article: &mut article,
        list_depth: 0,
    };
    walker.walk_children(body);
    article
}

struct Walker<'a> {
    doc: &'a Document,
    base_url: &'a Url,
    article: &'a mut Article,
    list_depth: u8,
}

impl Walker<'_> {
    fn walk_children(&mut self, id: NodeId) {
        for child in self.doc.children_of(id) {
            self.walk(child);
        }
    }

    fn walk(&mut self, id: NodeId) {
        match self.doc.get(id).data.clone() {
            NodeData::Text(text) => {
                // Stray top-level text becomes a paragraph (whitespace only
                // runs are dropped to avoid blank blocks).
                if !text.trim().is_empty() {
                    self.push_text_block(text);
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
                        let (text, links) = self.inline_text(id);
                        if !text.trim().is_empty() {
                            self.article
                                .blocks
                                .push(Block::Heading { level, text, links });
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
                            self.article.blocks.push(Block::Pre { text });
                        }
                    }
                    "ul" | "ol" | "dl" => {
                        self.list_depth = self.list_depth.saturating_add(1);
                        self.walk_children(id);
                        self.list_depth = self.list_depth.saturating_sub(1);
                    }
                    "li" | "dd" | "dt" => {
                        let (text, links) = self.inline_text(id);
                        if !text.trim().is_empty() {
                            self.article.blocks.push(Block::ListItem {
                                depth: self.list_depth.max(1),
                                text,
                                links,
                            });
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
                            self.article.blocks.push(Block::Paragraph {
                                text: cells.join(" | "),
                                links: Vec::new(),
                            });
                        }
                    }
                    "hr" => self.article.blocks.push(Block::Rule),
                    "br" => self.push_text_block("\n".to_owned()),
                    "img" => {
                        let alt = self
                            .doc
                            .get_attribute(id, "alt")
                            .unwrap_or("[image]")
                            .to_owned();
                        self.article.blocks.push(Block::Image { alt });
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
                Self::flush_items(&mut items, self, kind);
                self.walk(child);
            }
        }
        Self::flush_items(&mut items, self, kind);
    }

    /// Flush accumulated inline items as one block.
    fn flush_items(items: &mut Vec<InlineItem>, target: &mut Self, kind: BlockKind) {
        let taken = std::mem::take(items);
        if taken.is_empty() {
            return;
        }
        let (text, links) = build_run(&taken);
        if text.trim().is_empty() {
            return;
        }
        target.push_kind(kind, text, links);
    }

    /// Inline text with link spans for `node` (transparent inline content).
    /// Whitespace is normalized in the same pass, so link offsets always
    /// match the final text.
    fn inline_text(&self, id: NodeId) -> (String, Vec<LinkSpan>) {
        let mut items = Vec::new();
        self.collect_inline(id, &mut items);
        build_run(&items)
    }

    /// Collect a flat inline stream for `id` (recurses transparently).
    fn collect_inline(&self, id: NodeId, items: &mut Vec<InlineItem>) {
        match self.doc.get(id).data.clone() {
            NodeData::Text(chunk) => items.push(InlineItem::Text(chunk)),
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
    }

    fn push_text_block(&mut self, text: String) {
        let (text, _) = build_run(&[InlineItem::Text(text)]);
        if text.trim().is_empty() {
            return;
        }
        self.article.blocks.push(Block::Paragraph {
            text,
            links: Vec::new(),
        });
    }

    fn push_kind(&mut self, kind: BlockKind, text: String, links: Vec<LinkSpan>) {
        match kind {
            BlockKind::Paragraph => {
                self.article.blocks.push(Block::Paragraph { text, links });
            }
        }
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
fn build_run(items: &[InlineItem]) -> (String, Vec<LinkSpan>) {
    let mut out = String::new();
    let mut links = Vec::new();
    let mut open: Vec<(Url, usize)> = Vec::new();
    let mut pending_space = false;
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
    (out, links)
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
    links: Vec<LinkSpan>,
    overlays: Vec<Overlay>,
    /// Top offset inside the article (set by layout).
    pub y: f32,
    /// Laid-out height including trailing gap.
    pub height: f32,
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
            links,
            overlays,
            y,
            height,
        }
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
        self.buffer.draw(font_system, cache, canvas, x, base_y);
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
}
