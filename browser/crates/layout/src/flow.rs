//! CSS 2.1 normal flow: nested boxes, collapsed margins and baseline lines.

use crate::text::{Shaped, Shaper};
use crate::*;
use css::style::{BackgroundImage, TextAlign, WhiteSpace};
use css::values::{Length, LengthContext, SizeValue};
use std::sync::Arc;

#[derive(Clone, Copy, Default)]
struct MarginSet {
    positive: f32,
    negative: f32,
}
impl MarginSet {
    fn new(value: f32) -> Self {
        Self {
            positive: value.max(0.0),
            negative: value.min(0.0),
        }
    }
    fn merge(self, other: Self) -> Self {
        Self {
            positive: self.positive.max(other.positive),
            negative: self.negative.min(other.negative),
        }
    }
    fn value(self) -> f32 {
        self.positive + self.negative
    }
}
#[derive(Clone, Copy, Default)]
struct Margins {
    before: MarginSet,
    after: MarginSet,
    through: bool,
    top_child: bool,
    bottom_child: bool,
}

pub fn layout(viewport: Size, tree: &mut RenderTree, opts: LayoutOpts<'_>) -> LayoutResult {
    let viewport = Size::new(
        finite(viewport.width).max(1.0),
        finite(viewport.height).max(1.0),
    );
    let mut engine = Engine {
        tree,
        images: opts.images,
        shaper: Shaper::new(opts.font_system),
        viewport,
        result: LayoutResult {
            viewport,
            ..LayoutResult::default()
        },
        margins: Vec::new(),
    };
    engine.result.stats.depth_fallbacks = engine.tree.depth_fallbacks;
    engine
        .margins
        .resize(engine.tree.nodes.len(), Margins::default());
    let mut cursor = 0.0;
    let mut pending = MarginSet::default();
    for root in engine.tree.roots.clone() {
        engine.compute_margins(root, viewport.width, true);
        let m = engine.margins[root];
        let top = cursor + pending.merge(m.before).value();
        let index = engine.block(
            root,
            Rect::new(0.0, 0.0, viewport.width, viewport.height),
            top,
            Some(viewport.height),
            &[],
            false,
        );
        if !m.through {
            cursor = engine.result.boxes[index].border_box.bottom();
            pending = m.after;
        } else {
            pending = pending.merge(m.before).merge(m.after);
        }
        engine.result.roots.push(index);
    }
    let mut right = viewport.width;
    let mut bottom = cursor + pending.value();
    // Visible overflow contributes to document scroll extents, clipped content
    // stays local to its scroll frame (it must not grow the outer scrollbar).
    for root in &engine.result.roots {
        let b = &engine.result.boxes[*root];
        right = right
            .max(b.border_box.right())
            .max(b.padding_box.x + b.scroll_size.width);
        bottom = bottom
            .max(b.border_box.bottom())
            .max(b.padding_box.y + b.scroll_size.height);
    }
    engine.result.content_size = Size::new(finite(right).max(0.0), finite(bottom).max(0.0));
    engine.result.stats.boxes = engine.result.boxes.len();
    engine.result
}

fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-MAX_EXTENT, MAX_EXTENT)
    } else {
        0.0
    }
}
fn overflow(style: &ComputedStyle) -> Overflow {
    match style
        .get_property_value("overflow")
        .split_ascii_whitespace()
        .next()
    {
        Some("hidden" | "clip") => Overflow::Hidden,
        Some("scroll") => Overflow::Scroll,
        Some("auto") => Overflow::Auto,
        _ => Overflow::Visible,
    }
}

struct Engine<'a> {
    tree: &'a RenderTree,
    images: &'a Images,
    shaper: Shaper<'a>,
    viewport: Size,
    result: LayoutResult,
    margins: Vec<Margins>,
}
impl Engine<'_> {
    fn ctx(&self, style: &ComputedStyle, basis: f32) -> LengthContext {
        LengthContext {
            font_size: style.font_size,
            root_font_size: self
                .tree
                .roots
                .first()
                .map_or(16.0, |&i| self.tree.nodes[i].style.font_size),
            viewport_width: self.viewport.width,
            viewport_height: self.viewport.height,
            percentage_basis: basis,
        }
    }
    fn edges(&self, node: &RenderNode, width: f32) -> (Edges, Edges, Edges) {
        if matches!(node.kind, RenderKind::AnonymousBlock) {
            return (Edges::default(), Edges::default(), Edges::default());
        }
        let ctx = self.ctx(&node.style, width);
        (
            Edges::from_array(std::array::from_fn(|i| {
                node.style.margin[i].resolve(ctx).unwrap_or(0.0)
            })),
            Edges::from_array(std::array::from_fn(|i| {
                node.style.padding[i].resolve(ctx).unwrap_or(0.0).max(0.0)
            })),
            Edges::from_array(std::array::from_fn(|i| {
                node.style.border_width[i]
                    .resolve(ctx)
                    .unwrap_or(0.0)
                    .max(0.0)
            })),
        )
    }
    fn content_width(
        &self,
        node: &RenderNode,
        width: f32,
        shrink: Option<f32>,
    ) -> (f32, Edges, Edges, Edges) {
        let (mut margin, padding, border) = self.edges(node, width);
        let ctx = self.ctx(&node.style, width);
        let inner = padding.horizontal() + border.horizontal();
        let specified = if matches!(node.kind, RenderKind::AnonymousBlock) {
            None
        } else {
            node.style.width.resolve(ctx)
        };
        let adjust = |n: f32| {
            if node.style.border_box {
                (n - inner).max(0.0)
            } else {
                n.max(0.0)
            }
        };
        let mut content = specified
            .map(adjust)
            .or(shrink)
            .unwrap_or(width - margin.horizontal() - inner)
            .max(0.0);
        if !matches!(node.kind, RenderKind::AnonymousBlock) {
            if let Some(max) = node.style.max_width.resolve(ctx) {
                content = content.min(adjust(max));
            }
            if let Some(min) = node.style.min_width.resolve(ctx) {
                content = content.max(adjust(min));
            }
        }
        let extra = width - content - inner - margin.horizontal();
        let left_auto = matches!(node.style.margin[3], SizeValue::Auto);
        let right_auto = matches!(node.style.margin[1], SizeValue::Auto);
        if specified.is_some() || shrink.is_some() || content + inner + margin.horizontal() < width
        {
            match (left_auto, right_auto) {
                (true, true) if extra >= 0.0 => {
                    margin.left += extra / 2.0;
                    margin.right += extra / 2.0;
                }
                (true, false) if extra >= 0.0 => margin.left += extra,
                _ => margin.right += extra,
            }
        }
        (finite(content), margin, padding, border)
    }
    fn has_inline_ink(&self, index: usize) -> bool {
        let node = &self.tree.nodes[index];
        match &node.kind {
            RenderKind::Text(text) => {
                if matches!(
                    node.style.white_space,
                    WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces
                ) {
                    !text.is_empty()
                } else {
                    !text
                        .trim_matches([' ', '\n', '\r', '\t', '\x0c'])
                        .is_empty()
                }
            }
            RenderKind::Break | RenderKind::Replaced(_) | RenderKind::InlineBlock => true,
            _ => node.children.iter().any(|&i| self.has_inline_ink(i)),
        }
    }
    fn compute_margins(&mut self, index: usize, width: f32, root: bool) {
        let node = &self.tree.nodes[index];
        let (content_width, margin, padding, border) = self.content_width(node, width, None);
        for &child in &node.children {
            if self.tree.nodes[child].is_block() {
                self.compute_margins(child, content_width, false);
            }
        }
        let ctx = self.ctx(&node.style, width);
        let auto_height = matches!(node.kind, RenderKind::AnonymousBlock)
            || matches!(node.style.height, SizeValue::Auto);
        let min_zero = node.style.min_height.resolve(ctx).unwrap_or(0.0) <= 0.0;
        let block_children = node.children.iter().all(|&i| self.tree.nodes[i].is_block());
        let mut m = Margins {
            before: MarginSet::new(margin.top),
            after: MarginSet::new(margin.bottom),
            through: auto_height
                && min_zero
                && padding.vertical() + border.vertical() == 0.0
                && !self.has_inline_ink(index)
                && node
                    .children
                    .iter()
                    .filter(|&&i| self.tree.nodes[i].is_block())
                    .all(|&i| self.margins[i].through),
            top_child: !root
                && overflow(&node.style) == Overflow::Visible
                && padding.top + border.top == 0.0
                && block_children,
            bottom_child: !root
                && overflow(&node.style) == Overflow::Visible
                && padding.bottom + border.bottom == 0.0
                && auto_height
                && min_zero
                && block_children,
        };
        if m.top_child {
            for &child in &node.children {
                m.before = m.before.merge(self.margins[child].before);
                if !self.margins[child].through {
                    break;
                }
                m.before = m.before.merge(self.margins[child].after);
            }
        }
        if m.bottom_child {
            for &child in node.children.iter().rev() {
                m.after = m.after.merge(self.margins[child].after);
                if !self.margins[child].through {
                    break;
                }
                m.after = m.after.merge(self.margins[child].before);
            }
        }
        if m.through {
            let both = m.before.merge(m.after);
            m.before = both;
            m.after = both;
        }
        self.margins[index] = m;
    }

    fn block(
        &mut self,
        index: usize,
        containing: Rect,
        top: f32,
        parent_height: Option<f32>,
        ancestors: &[usize],
        atomic: bool,
    ) -> usize {
        let node = &self.tree.nodes[index];
        let shrink = if atomic
            && node
                .style
                .width
                .resolve(self.ctx(&node.style, containing.width))
                .is_none()
        {
            Some(self.intrinsic_width(index).min(containing.width))
        } else {
            None
        };
        let (mut width, mut margin, padding, border) =
            self.content_width(node, containing.width, shrink);
        let inset = padding.add_edges(border);
        let height_ctx = self.ctx(&node.style, parent_height.unwrap_or(0.0));
        let height_value = |value: &SizeValue| {
            if parent_height.is_none()
                && matches!(value, SizeValue::Length(l) if l.0.iter().any(|t| matches!(t, css::Token::Percentage(_))))
            {
                None
            } else {
                value.resolve(height_ctx).map(|v| {
                    if node.style.border_box {
                        (v - inset.vertical()).max(0.0)
                    } else {
                        v.max(0.0)
                    }
                })
            }
        };
        let mut specified_height = if matches!(node.kind, RenderKind::AnonymousBlock) {
            None
        } else {
            height_value(&node.style.height)
        };
        let replaced_size = if let RenderKind::Replaced(replaced) = &node.kind {
            let size = self.replaced_size(node, replaced, containing.width, parent_height);
            width = size.width;
            specified_height = Some(size.height);
            if !atomic {
                let (raw_margin, _, _) = self.edges(node, containing.width);
                margin = raw_margin;
                let remaining = containing.width - width - inset.horizontal() - margin.horizontal();
                if remaining > 0.0 {
                    match (
                        matches!(node.style.margin[3], SizeValue::Auto),
                        matches!(node.style.margin[1], SizeValue::Auto),
                    ) {
                        (true, true) => margin.left += remaining / 2.0,
                        (true, false) => margin.left += remaining,
                        _ => {}
                    }
                }
            }
            Some(size)
        } else {
            None
        };
        let x = finite(containing.x + margin.left);
        let y = finite(top);
        let content = Rect::new(
            x + inset.left,
            y + inset.top,
            width,
            replaced_size.map_or(0.0, |s| s.height),
        );
        let box_index = self.result.boxes.len();
        let radius = Length::parse(&css::token::tokenize_all(
            &node.style.get_property_value("border-radius"),
        ))
        .and_then(|l| l.resolve(self.ctx(&node.style, width)))
        .unwrap_or(0.0)
        .max(0.0);
        let background_image = if let Some(BackgroundImage::Url(url)) = &node.style.background_image
        {
            self.images.get(url).cloned()
        } else {
            None
        };
        self.result.boxes.push(LayoutBox {
            element: node.element,
            anonymous: matches!(node.kind, RenderKind::AnonymousBlock),
            style: node.style.clone(),
            border_box: Rect::new(x, y, width + inset.horizontal(), 0.0),
            padding_box: Rect::default(),
            content_box: content,
            border,
            content: Vec::new(),
            overflow: if matches!(node.kind, RenderKind::AnonymousBlock) {
                Overflow::Visible
            } else if matches!(node.kind, RenderKind::Replaced(_)) {
                // Replaced content (including failed-image alt text) stays
                // inside its own box, rather than overlapping adjacent words.
                Overflow::Hidden
            } else {
                overflow(&node.style)
            },
            scroll_size: Size::default(),
            radius,
            background_image,
        });
        let mut child_ancestors = ancestors.to_vec();
        child_ancestors.push(box_index);
        let mut cursor = content.y;
        let mut pending = MarginSet::default();
        let m = self.margins[index];
        if replaced_size.is_some() {
            self.emit_replaced(index, box_index, content, &child_ancestors);
        } else if node.children.iter().any(|&i| self.tree.nodes[i].is_block()) {
            let mut first = true;
            for &child in &node.children {
                if self.result.stats.truncated {
                    break;
                }
                let cm = self.margins[child];
                let child_top = if first && m.top_child {
                    cursor
                } else {
                    cursor + pending.merge(cm.before).value()
                };
                let child_box = self.block(
                    child,
                    Rect::new(
                        content.x,
                        content.y,
                        content.width,
                        specified_height.unwrap_or(0.0),
                    ),
                    child_top,
                    specified_height,
                    &child_ancestors,
                    false,
                );
                self.result.boxes[box_index]
                    .content
                    .push(Content::Box(child_box));
                if cm.through {
                    if !(first && m.top_child) {
                        pending = pending.merge(cm.before).merge(cm.after);
                    }
                } else {
                    cursor = self.result.boxes[child_box].border_box.bottom();
                    pending = cm.after;
                    first = false;
                }
            }
            if !m.bottom_child {
                cursor += pending.value();
            }
        } else {
            cursor += self.inline(index, box_index, content, &child_ancestors);
        }
        let natural_height = (cursor - content.y).max(0.0);
        let mut height = specified_height.unwrap_or(natural_height);
        if !matches!(node.kind, RenderKind::AnonymousBlock) {
            if let Some(max) = height_value(&node.style.max_height) {
                height = height.min(max);
            }
            if let Some(min) = height_value(&node.style.min_height) {
                height = height.max(min);
            }
        }
        height = finite(height).max(0.0);
        let border_box = Rect::new(x, y, width + inset.horizontal(), height + inset.vertical());
        let padding_box = border_box.inset(border);
        let mut right = padding_box.right();
        let mut bottom = padding_box.bottom();
        for child in &self.result.boxes[box_index].content {
            let rect = match child {
                Content::Box(i) => {
                    let b = &self.result.boxes[*i];
                    if b.overflow == Overflow::Visible {
                        Rect::new(
                            b.border_box.x,
                            b.border_box.y,
                            (b.padding_box.x + b.scroll_size.width - b.border_box.x)
                                .max(b.border_box.width),
                            (b.padding_box.y + b.scroll_size.height - b.border_box.y)
                                .max(b.border_box.height),
                        )
                    } else {
                        b.border_box
                    }
                }
                Content::Text(t) => t.rect,
                Content::Image { rect, .. } | Content::Decoration { rect, .. } => *rect,
            };
            right = right.max(rect.right());
            bottom = bottom.max(rect.bottom() + padding.bottom);
        }
        let b = &mut self.result.boxes[box_index];
        b.border_box = border_box;
        b.padding_box = padding_box;
        b.content_box.height = height;
        b.scroll_size = Size::new(
            finite(right - padding_box.x).max(0.0),
            finite(bottom - padding_box.y).max(0.0),
        );
        box_index
    }

    fn intrinsic_width(&mut self, index: usize) -> f32 {
        let node = &self.tree.nodes[index];
        match &node.kind {
            RenderKind::Text(text) => self.shaper.shape(text, &node.style).width,
            RenderKind::Replaced(r) => self.replaced_size(node, r, self.viewport.width, None).width,
            _ => node
                .children
                .iter()
                .map(|&i| self.intrinsic_width(i))
                .sum::<f32>()
                .clamp(0.0, MAX_EXTENT),
        }
    }
    fn replaced_size(
        &mut self,
        node: &RenderNode,
        replaced: &Replaced,
        width: f32,
        parent_height: Option<f32>,
    ) -> Size {
        let ctx = self.ctx(&node.style, width);
        let (attrs_w, attrs_h, intrinsic) = match replaced {
            Replaced::Image {
                url,
                width,
                height,
                alt,
            } => (
                *width,
                *height,
                url.as_ref()
                    .and_then(|u| self.images.get(u))
                    .map(|i| i.size())
                    .unwrap_or_else(|| {
                        Size::new(
                            self.shaper.shape(alt, &node.style).width.max(32.0),
                            node.style
                                .line_height
                                .pixels(node.style.font_size)
                                .max(24.0),
                        )
                    }),
            ),
            Replaced::Video { width, height } => (*width, *height, Size::new(300.0, 150.0)),
            Replaced::Input { .. } => (
                None,
                None,
                Size::new(
                    160.0,
                    node.style.line_height.pixels(node.style.font_size) + 8.0,
                ),
            ),
        };
        let css_w = node.style.width.resolve(ctx);
        let h_ctx = self.ctx(&node.style, parent_height.unwrap_or(0.0));
        let css_h = if parent_height.is_none()
            && matches!(&node.style.height, SizeValue::Length(l) if l.0.iter().any(|t| matches!(t, css::Token::Percentage(_))))
        {
            None
        } else {
            node.style.height.resolve(h_ctx)
        };
        let (_, p, b) = self.edges(node, width);
        let inset = p.add_edges(b);
        let w = css_w.map(|w| {
            if node.style.border_box {
                (w - inset.horizontal()).max(0.0)
            } else {
                w
            }
        });
        let h = css_h.map(|h| {
            if node.style.border_box {
                (h - inset.vertical()).max(0.0)
            } else {
                h
            }
        });
        let natural = matches!(replaced, Replaced::Image { url: Some(url), .. } if self.images.contains_key(url));
        let intrinsic_ratio = if natural && intrinsic.height > 0.0 {
            intrinsic.width / intrinsic.height
        } else if let (Some(w), Some(h)) = (attrs_w, attrs_h) {
            if h > 0.0 {
                w / h
            } else {
                2.0
            }
        } else if intrinsic.height > 0.0 {
            intrinsic.width / intrinsic.height
        } else {
            2.0
        };
        let ratio_value = node.style.get_property_value("aspect-ratio");
        let prefer_natural = natural && ratio_value.trim_start().starts_with("auto");
        let ratio_value = ratio_value.trim_start_matches("auto").trim();
        let ratio = ratio_value
            .split_once('/')
            .and_then(|(n, d)| Some(n.trim().parse::<f32>().ok()? / d.trim().parse::<f32>().ok()?))
            .or_else(|| ratio_value.parse::<f32>().ok())
            .filter(|n| n.is_finite() && *n > 0.0)
            .unwrap_or(intrinsic_ratio);
        let ratio = if prefer_natural {
            intrinsic_ratio
        } else {
            ratio
        };
        let (mut w, mut h) = match (w, h) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w / ratio.max(0.001)),
            (None, Some(h)) => (h * ratio, h),
            _ => (intrinsic.width, intrinsic.height),
        };
        if let Some(max) = node.style.max_width.resolve(ctx) {
            if w > max {
                if css_h.is_none() {
                    h *= max.max(0.0) / w.max(0.001);
                }
                w = max.max(0.0);
            }
        }
        if let Some(min) = node.style.min_width.resolve(ctx) {
            if w < min {
                if css_h.is_none() {
                    h *= min / w.max(0.001);
                }
                w = min;
            }
        }
        if let Some(max) = node.style.max_height.resolve(h_ctx) {
            h = h.min(max);
        }
        if let Some(min) = node.style.min_height.resolve(h_ctx) {
            h = h.max(min);
        }
        Size::new(finite(w).max(0.0), finite(h).max(0.0))
    }
    fn emit_replaced(&mut self, index: usize, box_index: usize, rect: Rect, ancestors: &[usize]) {
        let node = &self.tree.nodes[index];
        if !node.style.visible {
            return;
        }
        self.result.targets.push(TargetRegion {
            element: node.element,
            rect,
            ancestors: ancestors.to_vec(),
        });
        let RenderKind::Replaced(ref replaced) = node.kind else {
            return;
        };
        match replaced {
            Replaced::Image { url, alt, .. } => {
                if let Some(image) = url.as_ref().and_then(|u| self.images.get(u)) {
                    self.result.boxes[box_index].content.push(Content::Image {
                        rect,
                        image: image.clone(),
                    });
                } else {
                    self.placeholder(box_index, rect, alt, node.style.clone());
                }
            }
            Replaced::Video { .. } => {
                self.placeholder(box_index, rect, "[video]", node.style.clone())
            }
            Replaced::Input { label } => {
                let shaped = self.shaper.shape(label, &node.style);
                self.emit_text(
                    box_index,
                    label.clone(),
                    &shaped,
                    (rect.x + 3.0, rect.y + shaped.ascent + 3.0),
                    rect,
                    &node.style.clone(),
                );
            }
        }
        if let Some(href) = &node.href {
            self.result.hits.push(HitRegion {
                rect,
                href: href.clone(),
                ancestors: ancestors.to_vec(),
            });
        }
    }
    fn placeholder(
        &mut self,
        box_index: usize,
        rect: Rect,
        label: &str,
        style: Arc<ComputedStyle>,
    ) {
        let color = css::values::Color::rgb(220, 224, 230);
        self.result.boxes[box_index]
            .content
            .push(Content::Decoration { rect, color });
        let shaped = self.shaper.shape(label, &style);
        self.emit_text(
            box_index,
            label.into(),
            &shaped,
            (rect.x + 3.0, rect.y + shaped.ascent + 3.0),
            rect,
            &style,
        );
    }
    fn emit_text(
        &mut self,
        box_index: usize,
        text: String,
        shaped: &Shaped,
        origin: (f32, f32),
        rect: Rect,
        style: &ComputedStyle,
    ) {
        let (x, baseline) = origin;
        let count = shaped
            .glyphs
            .len()
            .min(MAX_GLYPHS.saturating_sub(self.result.stats.glyphs));
        self.result.stats.glyphs += count;
        if count < shaped.glyphs.len() {
            self.result.stats.truncated = true;
        }
        let color = if style.visible && style.font_size > 0.0 {
            style.color
        } else {
            css::values::Color::TRANSPARENT
        };
        let glyphs = shaped
            .glyphs
            .iter()
            .take(count)
            .map(|g| {
                let p = g.physical((x, baseline), 1.0);
                Glyph {
                    key: p.cache_key,
                    x: p.x,
                    y: p.y,
                    color,
                }
            })
            .collect();
        self.result.boxes[box_index]
            .content
            .push(Content::Text(TextRun { rect, glyphs, text }));
        for name in style.text_decoration.split_ascii_whitespace() {
            let y = match name {
                "underline" => baseline + 1.0,
                "overline" => rect.y,
                "line-through" => baseline - style.font_size * 0.3,
                _ => continue,
            };
            self.result.boxes[box_index]
                .content
                .push(Content::Decoration {
                    rect: Rect::new(rect.x, y, rect.width, (style.font_size / 16.0).max(1.0)),
                    color,
                });
        }
    }
}

#[derive(Clone)]
enum PieceKind {
    Text { text: String, shaped: Shaped },
    Atomic(usize, Edges),
    Spacer,
    Break,
}
#[derive(Clone)]
struct Piece {
    element: ElementId,
    kind: PieceKind,
    style: Arc<ComputedStyle>,
    href: Option<Url>,
    ancestors: Vec<usize>,
    width: f32,
    ascent: f32,
    descent: f32,
    space: bool,
    collapsible: bool,
    can_wrap: bool,
}

impl Engine<'_> {
    fn pieces(
        &mut self,
        index: usize,
        width: f32,
        ancestors: &[usize],
        out: &mut Vec<Piece>,
        pending_space: &mut bool,
    ) {
        let node = &self.tree.nodes[index];
        if out.len() >= MAX_GLYPHS {
            self.result.stats.truncated = true;
            return;
        }
        match &node.kind {
            RenderKind::Text(text) => {
                let text = match node.style.text_transform.as_str() {
                    "uppercase" => text.to_uppercase(),
                    "lowercase" => text.to_lowercase(),
                    "capitalize" => text
                        .split_inclusive(char::is_whitespace)
                        .map(|word| {
                            let mut c = word.chars();
                            c.next()
                                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                                .unwrap_or_default()
                        })
                        .collect(),
                    _ => text.clone(),
                };
                let preserve = matches!(
                    node.style.white_space,
                    WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces
                );
                let newlines = preserve || node.style.white_space == WhiteSpace::PreLine;
                let wrap = !matches!(node.style.white_space, WhiteSpace::Pre | WhiteSpace::NoWrap);
                let mut word = String::new();
                for ch in text.chars() {
                    let ws = matches!(ch, ' ' | '\t' | '\n' | '\r' | '\x0c');
                    if ws {
                        if !word.is_empty() {
                            self.word_piece(
                                std::mem::take(&mut word),
                                node,
                                ancestors,
                                false,
                                false,
                                wrap,
                                out,
                            );
                        }
                        if ch == '\n' && newlines {
                            *pending_space = false;
                            out.push(Piece {
                                element: node.element,
                                kind: PieceKind::Break,
                                style: node.style.clone(),
                                href: None,
                                ancestors: ancestors.to_vec(),
                                width: 0.0,
                                ascent: 0.0,
                                descent: 0.0,
                                space: false,
                                collapsible: false,
                                can_wrap: wrap,
                            });
                        } else if preserve {
                            let text = if ch == '\t' { "    " } else { " " };
                            self.word_piece(text.into(), node, ancestors, true, false, wrap, out);
                        } else {
                            *pending_space = true;
                        }
                    } else {
                        if *pending_space {
                            self.word_piece(" ".into(), node, ancestors, true, true, wrap, out);
                            *pending_space = false;
                        }
                        word.push(ch);
                    }
                }
                if !word.is_empty() {
                    self.word_piece(word, node, ancestors, false, false, wrap, out);
                }
            }
            RenderKind::Break => {
                *pending_space = false;
                out.push(Piece {
                    element: node.element,
                    kind: PieceKind::Break,
                    style: node.style.clone(),
                    href: None,
                    ancestors: ancestors.to_vec(),
                    width: 0.0,
                    ascent: 0.0,
                    descent: 0.0,
                    space: false,
                    collapsible: false,
                    can_wrap: true,
                });
            }
            RenderKind::Replaced(_) | RenderKind::InlineBlock => {
                if *pending_space {
                    self.word_piece(" ".into(), node, ancestors, true, true, true, out);
                    *pending_space = false;
                }
                self.compute_margins(index, width, true);
                let b = self.block(index, Rect::new(0.0, 0.0, width, 0.0), 0.0, None, &[], true);
                let (m, _, _) = self.edges(node, width);
                let rect = self.result.boxes[b].border_box;
                out.push(Piece {
                    element: node.element,
                    kind: PieceKind::Atomic(b, m),
                    style: node.style.clone(),
                    href: node.href.clone(),
                    ancestors: ancestors.to_vec(),
                    width: rect.width + m.horizontal(),
                    ascent: rect.height + m.top + m.bottom,
                    descent: 0.0,
                    space: false,
                    collapsible: false,
                    can_wrap: !matches!(
                        node.style.white_space,
                        WhiteSpace::Pre | WhiteSpace::NoWrap
                    ),
                });
            }
            _ => {
                let mut inner = ancestors.to_vec();
                let (margin, padding, border) = self.edges(node, width);
                let is_inline = matches!(node.kind, RenderKind::Inline);
                if is_inline {
                    self.spacer(margin.left, node, ancestors, out);
                    inner.push(index);
                    self.spacer(padding.left + border.left, node, &inner, out);
                }
                for &child in &node.children {
                    self.pieces(child, width, &inner, out, pending_space);
                }
                if is_inline {
                    self.spacer(padding.right + border.right, node, &inner, out);
                    self.spacer(margin.right, node, ancestors, out);
                }
            }
        }
    }
    fn spacer(&mut self, width: f32, node: &RenderNode, ancestors: &[usize], out: &mut Vec<Piece>) {
        if width == 0.0 {
            return;
        }
        let strut = self.shaper.shape("Mg", &node.style);
        out.push(Piece {
            element: node.element,
            kind: PieceKind::Spacer,
            style: node.style.clone(),
            href: None,
            ancestors: ancestors.to_vec(),
            width,
            ascent: strut.ascent,
            descent: strut.descent,
            space: false,
            collapsible: false,
            can_wrap: false,
        });
    }
    #[allow(clippy::too_many_arguments)]
    fn word_piece(
        &mut self,
        text: String,
        node: &RenderNode,
        ancestors: &[usize],
        space: bool,
        collapsible: bool,
        can_wrap: bool,
        out: &mut Vec<Piece>,
    ) {
        let shaped = self.shaper.shape(&text, &node.style);
        let width = shaped.width;
        let ascent = shaped.ascent;
        let descent = shaped.descent;
        out.push(Piece {
            element: node.element,
            kind: PieceKind::Text { text, shaped },
            style: node.style.clone(),
            href: node.href.clone(),
            ancestors: ancestors.to_vec(),
            width,
            ascent,
            descent,
            space,
            collapsible,
            can_wrap,
        });
    }
    fn inline(
        &mut self,
        index: usize,
        box_index: usize,
        containing: Rect,
        ancestors: &[usize],
    ) -> f32 {
        let node = &self.tree.nodes[index];
        let mut pieces = Vec::new();
        let mut pending = false;
        for &child in &node.children {
            self.pieces(child, containing.width, &[], &mut pieces, &mut pending);
        }
        let mut line = Vec::new();
        let mut width = 0.0;
        let mut y = containing.y;
        let mut trailing_break = false;
        for piece in pieces {
            if matches!(piece.kind, PieceKind::Break) {
                y += self.line(
                    box_index,
                    &mut line,
                    containing.x,
                    y,
                    containing.width,
                    &node.style,
                    ancestors,
                    false,
                    true,
                );
                width = 0.0;
                trailing_break = true;
                continue;
            }
            trailing_break = false;
            if piece.collapsible && line.is_empty() {
                continue;
            }
            // Only spaces/atomic boundaries are legal word-wrap opportunities.
            // Styling a substring does not introduce an artificial break.
            if piece.can_wrap
                && !piece.space
                && width + piece.width > containing.width
                && !line.is_empty()
            {
                let opportunity = line.iter().rposition(|p: &Piece| {
                    p.can_wrap && (p.space || matches!(p.kind, PieceKind::Atomic(..)))
                });
                if let Some(at) = opportunity {
                    let mut tail = line.split_off(at + 1);
                    y += self.line(
                        box_index,
                        &mut line,
                        containing.x,
                        y,
                        containing.width,
                        &node.style,
                        ancestors,
                        true,
                        false,
                    );
                    line.append(&mut tail);
                    width = line.iter().map(|p| p.width).sum();
                }
            }
            width += piece.width;
            line.push(piece);
        }
        if !line.is_empty() || trailing_break {
            y += self.line(
                box_index,
                &mut line,
                containing.x,
                y,
                containing.width,
                &node.style,
                ancestors,
                false,
                trailing_break,
            );
        }
        finite(y - containing.y).max(0.0)
    }
    #[allow(clippy::too_many_arguments)]
    fn line(
        &mut self,
        box_index: usize,
        pieces: &mut Vec<Piece>,
        x: f32,
        y: f32,
        available: f32,
        parent: &ComputedStyle,
        ancestors: &[usize],
        justify: bool,
        force: bool,
    ) -> f32 {
        while pieces.last().is_some_and(|p| p.collapsible) {
            pieces.pop();
        }
        if pieces.is_empty() && !force {
            return 0.0;
        }
        self.result.stats.lines += 1;
        let strut = self.shaper.shape("Mg", parent);
        let ascent = pieces.iter().map(|p| p.ascent).fold(strut.ascent, f32::max);
        let descent = pieces
            .iter()
            .map(|p| p.descent)
            .fold(strut.descent, f32::max);
        let height = (ascent + descent).max(0.1);
        let width = pieces.iter().map(|p| p.width).sum::<f32>();
        let extra = (available - width).max(0.0);
        let rtl = parent.get_property_value("direction") == "rtl";
        let align = match parent.text_align {
            TextAlign::Start => {
                if rtl {
                    TextAlign::Right
                } else {
                    TextAlign::Left
                }
            }
            TextAlign::End => {
                if rtl {
                    TextAlign::Left
                } else {
                    TextAlign::Right
                }
            }
            other => other,
        };
        let mut cursor = x + match align {
            TextAlign::Right => extra,
            TextAlign::Center => extra / 2.0,
            _ => 0.0,
        };
        let spaces = pieces.iter().filter(|p| p.space).count();
        let stretch = if align == TextAlign::Justify && justify && spaces > 0 {
            extra / spaces as f32
        } else {
            0.0
        };
        let mut logical = String::new();
        let mut starts = Vec::new();
        for piece in pieces.iter() {
            starts.push(logical.len());
            match &piece.kind {
                PieceKind::Text { text, .. } => logical.push_str(text),
                _ => logical.push('\u{fffc}'),
            }
        }
        let bidi = unicode_bidi::BidiInfo::new(
            &logical,
            Some(if rtl {
                unicode_bidi::Level::rtl()
            } else {
                unicode_bidi::Level::ltr()
            }),
        );
        let levels: Vec<_> = starts
            .iter()
            .map(|&i| {
                bidi.levels
                    .get(i)
                    .copied()
                    .unwrap_or(unicode_bidi::Level::ltr())
            })
            .collect();
        let order = unicode_bidi::BidiInfo::reorder_visual(&levels);
        let mut inline_rects: HashMap<usize, Rect> = HashMap::new();
        let mut foreground = Vec::new();
        let content_start = self.result.boxes[box_index].content.len();
        for i in order {
            let piece = &pieces[i];
            let w = piece.width + if piece.space { stretch } else { 0.0 };
            let rect = Rect::new(
                cursor,
                y + ascent - piece.ascent,
                w,
                piece.ascent + piece.descent,
            );
            if piece.style.visible {
                self.result.targets.push(TargetRegion {
                    element: piece.element,
                    rect,
                    ancestors: ancestors.to_vec(),
                });
            }
            for &ancestor in &piece.ancestors {
                inline_rects
                    .entry(ancestor)
                    .and_modify(|r| {
                        let right = r.right().max(rect.right());
                        let bottom = r.bottom().max(rect.bottom());
                        r.x = r.x.min(rect.x);
                        r.y = r.y.min(rect.y);
                        r.width = right - r.x;
                        r.height = bottom - r.y;
                    })
                    .or_insert(rect);
            }
            match &piece.kind {
                PieceKind::Text { text, shaped } => {
                    let alpha = piece
                        .ancestors
                        .iter()
                        .map(|&i| self.tree.nodes[i].style.opacity)
                        .product::<f32>();
                    if alpha >= 1.0 {
                        self.emit_text(
                            box_index,
                            text.clone(),
                            shaped,
                            (cursor, y + ascent),
                            rect,
                            &piece.style,
                        );
                    } else {
                        let mut style = (*piece.style).clone();
                        style.color.a = (style.color.a as f32 * alpha) as u8;
                        self.emit_text(
                            box_index,
                            text.clone(),
                            shaped,
                            (cursor, y + ascent),
                            rect,
                            &style,
                        );
                    }
                }
                PieceKind::Atomic(child, margin) => {
                    let old = self.result.boxes[*child].border_box;
                    self.translate_box(
                        *child,
                        cursor + margin.left - old.x,
                        rect.y + margin.top - old.y,
                        ancestors,
                    );
                    self.result.boxes[box_index]
                        .content
                        .push(Content::Box(*child));
                }
                PieceKind::Break | PieceKind::Spacer => {}
            }
            if let Some(href) = &piece.href {
                if !matches!(piece.kind, PieceKind::Atomic(..)) && piece.style.visible {
                    self.result.hits.push(HitRegion {
                        rect,
                        href: href.clone(),
                        ancestors: ancestors.to_vec(),
                    });
                }
            }
            cursor += w;
        }
        foreground.extend(self.result.boxes[box_index].content.drain(content_start..));
        // Inline backgrounds sit behind glyphs, in outer-to-inner tree order.
        let mut inline_rects: Vec<_> = inline_rects.into_iter().collect();
        inline_rects.sort_by_key(|(id, _)| std::cmp::Reverse(*id));
        for (id, rect) in inline_rects {
            let n = &self.tree.nodes[id];
            let (_, padding, border) = self.edges(n, available);
            let bg = n.style.background_color;
            if bg.a == 0 && border.horizontal() + border.vertical() == 0.0 {
                continue;
            }
            let rect = Rect::new(
                rect.x,
                rect.y - padding.top - border.top,
                rect.width,
                rect.height + padding.vertical() + border.vertical(),
            );
            let child = self.result.boxes.len();
            self.result.boxes.push(LayoutBox {
                element: n.element,
                anonymous: false,
                style: n.style.clone(),
                border_box: rect,
                padding_box: rect.inset(border),
                content_box: rect.inset(border.add_edges(padding)),
                border,
                content: Vec::new(),
                overflow: Overflow::Visible,
                scroll_size: rect.size(),
                radius: 0.0,
                background_image: None,
            });
            self.result.boxes[box_index]
                .content
                .push(Content::Box(child));
        }
        self.result.boxes[box_index].content.extend(foreground);
        pieces.clear();
        height
    }
    fn translate_box(&mut self, index: usize, dx: f32, dy: f32, outer: &[usize]) {
        let b = &mut self.result.boxes[index];
        b.border_box = b.border_box.translate(dx, dy);
        b.padding_box = b.padding_box.translate(dx, dy);
        b.content_box = b.content_box.translate(dx, dy);
        let children = b
            .content
            .iter()
            .filter_map(|c| {
                if let Content::Box(i) = c {
                    Some(*i)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for content in &mut b.content {
            match content {
                Content::Text(t) => {
                    t.rect = t.rect.translate(dx, dy);
                    for g in &mut t.glyphs {
                        g.x += dx as i32;
                        g.y += dy as i32;
                    }
                }
                Content::Image { rect, .. } | Content::Decoration { rect, .. } => {
                    *rect = rect.translate(dx, dy)
                }
                _ => {}
            }
        }
        for hit in &mut self.result.hits {
            if hit.ancestors.first() == Some(&index) {
                hit.rect = hit.rect.translate(dx, dy);
                let mut a = outer.to_vec();
                a.extend_from_slice(&hit.ancestors);
                hit.ancestors = a;
            }
        }
        for target in &mut self.result.targets {
            if target.ancestors.first() == Some(&index) {
                target.rect = target.rect.translate(dx, dy);
                let mut a = outer.to_vec();
                a.extend_from_slice(&target.ancestors);
                target.ancestors = a;
            }
        }
        for child in children {
            self.translate_box(child, dx, dy, &[]);
        }
    }
}
