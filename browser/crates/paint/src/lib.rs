//! Backend-independent display lists and GPU/software compositing (plan/07).
//! Layout and glyph preparation run only on content/font/viewport changes.

pub mod gpu;
pub mod images;
mod software;

use cosmic_text::{CacheKey, FontSystem, SwashCache, SwashContent};
use css::values::Color;
use layout::{
    Content, Edges, ElementId, LayoutResult, Overflow, RasterImage, Rect, ScrollOffsets, Size,
    TextRun,
};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum DisplayItem {
    FillRect {
        rect: Rect,
        color: Color,
        radius: f32,
    },
    Border {
        rect: Rect,
        widths: Edges,
        colors: [Color; 4],
        radius: f32,
    },
    TextRun(TextRun),
    Image {
        rect: Rect,
        image: Arc<RasterImage>,
    },
    Clip(Rect),
    EndClip,
    ScrollFrame {
        element: ElementId,
        rect: Rect,
        size: Size,
    },
    EndScrollFrame,
    ScrollBars {
        element: ElementId,
        rect: Rect,
        size: Size,
    },
    StackingContext {
        opacity: f32,
    },
    EndStackingContext,
}

#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    pub items: Vec<DisplayItem>,
}

pub fn build_display_list(layout: &LayoutResult) -> DisplayList {
    enum Walk {
        Box(usize),
        Item(DisplayItem),
    }
    fn walk(result: &LayoutResult, index: usize, list: &mut DisplayList) -> Vec<Walk> {
        let b = &result.boxes[index];
        if !b.anonymous {
            list.items.push(DisplayItem::StackingContext {
                opacity: b.style.opacity,
            });
            if b.style.visible {
                list.items.push(DisplayItem::FillRect {
                    rect: b.border_box,
                    color: b.style.background_color,
                    radius: b.radius,
                });
                if let Some(image) = &b.background_image {
                    list.items.push(DisplayItem::Clip(b.padding_box));
                    list.items.push(DisplayItem::Image {
                        rect: b.padding_box,
                        image: image.clone(),
                    });
                    list.items.push(DisplayItem::EndClip);
                }
                if let Some(css::style::BackgroundImage::LinearGradient(colors)) =
                    &b.style.background_image
                {
                    if let (Some(first), Some(last)) = (colors.first(), colors.last()) {
                        for i in 0..64 {
                            let t = i as f32 / 63.0;
                            let channel =
                                |a: u8, z: u8| (a as f32 * (1.0 - t) + z as f32 * t) as u8;
                            let color = Color {
                                r: channel(first.r, last.r),
                                g: channel(first.g, last.g),
                                b: channel(first.b, last.b),
                                a: channel(first.a, last.a),
                            };
                            list.items.push(DisplayItem::FillRect {
                                rect: Rect::new(
                                    b.border_box.x,
                                    b.border_box.y + b.border_box.height * i as f32 / 64.0,
                                    b.border_box.width,
                                    b.border_box.height / 64.0 + 0.01,
                                ),
                                color,
                                radius: 0.0,
                            });
                        }
                    }
                }
                list.items.push(DisplayItem::Border {
                    rect: b.border_box,
                    widths: b.border,
                    colors: b.style.border_color,
                    radius: b.radius,
                });
            }
        }
        match b.overflow {
            Overflow::Hidden => list.items.push(DisplayItem::Clip(b.padding_box)),
            Overflow::Scroll | Overflow::Auto => list.items.push(DisplayItem::ScrollFrame {
                element: b.element,
                rect: b.padding_box,
                size: b.scroll_size,
            }),
            Overflow::Visible => {}
        }
        let mut tail = Vec::new();
        for content in &b.content {
            match content {
                Content::Box(i) => tail.push(Walk::Box(*i)),
                Content::Text(run) => tail.push(Walk::Item(DisplayItem::TextRun(run.clone()))),
                Content::Image { rect, image } => {
                    if b.style.visible {
                        tail.push(Walk::Item(DisplayItem::Image {
                            rect: *rect,
                            image: image.clone(),
                        }));
                    }
                }
                Content::Decoration { rect, color } => {
                    tail.push(Walk::Item(DisplayItem::FillRect {
                        rect: *rect,
                        color: *color,
                        radius: 0.0,
                    }))
                }
            }
        }
        match b.overflow {
            Overflow::Hidden => tail.push(Walk::Item(DisplayItem::EndClip)),
            Overflow::Scroll | Overflow::Auto => tail.push(Walk::Item(DisplayItem::EndScrollFrame)),
            Overflow::Visible => {}
        }
        if matches!(b.overflow, Overflow::Scroll | Overflow::Auto) {
            tail.push(Walk::Item(DisplayItem::ScrollBars {
                element: b.element,
                rect: b.padding_box,
                size: b.scroll_size,
            }));
        }
        if !b.anonymous {
            tail.push(Walk::Item(DisplayItem::EndStackingContext));
        }
        tail
    }
    let mut list = DisplayList::default();
    let mut work: Vec<_> = layout.roots.iter().rev().map(|&i| Walk::Box(i)).collect();
    while let Some(next) = work.pop() {
        match next {
            Walk::Box(index) => work.extend(walk(layout, index, &mut list).into_iter().rev()),
            Walk::Item(item) => list.items.push(item),
        }
    }
    list
}

#[derive(Debug, Clone)]
pub struct Quad {
    pub rect: Rect,
    pub color: Color,
    pub image: Option<Arc<RasterImage>>,
    pub rounded: Option<(Rect, f32)>,
}
impl Quad {
    pub fn solid(rect: Rect, color: Color) -> Self {
        Self {
            rect,
            color,
            image: None,
            rounded: None,
        }
    }
    pub fn image(rect: Rect, image: Arc<RasterImage>) -> Self {
        Self {
            rect,
            color: Color::WHITE,
            image: Some(image),
            rounded: None,
        }
    }
}

#[derive(Debug, Clone)]
enum SceneItem {
    Quad(Quad),
    Clip(Rect),
    Scroll {
        element: ElementId,
        rect: Rect,
        size: Size,
    },
    Opacity(f32),
    ScrollBars {
        element: ElementId,
        rect: Rect,
        size: Size,
    },
    Pop,
}

#[derive(Debug, Clone, Default)]
pub struct Scene {
    items: Vec<SceneItem>,
}
impl Scene {
    pub fn from_quads(quads: Vec<Quad>) -> Self {
        Self {
            items: quads.into_iter().map(SceneItem::Quad).collect(),
        }
    }
    pub fn quad_count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i, SceneItem::Quad(_)))
            .count()
    }
    /// Resolve clips and layer offsets. Scrolling never touches the render tree.
    pub fn visible_quads(
        &self,
        viewport: Rect,
        origin: (f32, f32),
        scale: f32,
        offsets: &ScrollOffsets,
    ) -> Vec<(Quad, Rect)> {
        #[derive(Clone, Copy)]
        struct State {
            x: f32,
            y: f32,
            clip: Rect,
            alpha: f32,
        }
        let scale = scale.clamp(0.25, 4.0);
        let mut state = State {
            x: origin.0,
            y: origin.1,
            clip: viewport,
            alpha: 1.0,
        };
        let transform = |r: Rect, s: State| {
            Rect::new(
                r.x * scale + s.x,
                r.y * scale + s.y,
                r.width * scale,
                r.height * scale,
            )
        };
        let mut stack = Vec::new();
        let mut quads = Vec::new();
        for item in &self.items {
            match item {
                SceneItem::Quad(q) => {
                    if q.color.a == 0 || state.alpha <= 0.0 {
                        continue;
                    }
                    let rect = transform(q.rect, state);
                    if !rect.intersects(state.clip) {
                        continue;
                    }
                    let mut q = q.clone();
                    q.rect = rect;
                    q.color.a = (q.color.a as f32 * state.alpha) as u8;
                    q.rounded = q
                        .rounded
                        .map(|(r, radius)| (transform(r, state), radius * scale));
                    quads.push((q, state.clip));
                }
                SceneItem::Clip(rect) => {
                    stack.push(state);
                    state.clip = state.clip.intersection(transform(*rect, state));
                }
                SceneItem::Scroll {
                    element,
                    rect,
                    size,
                } => {
                    stack.push(state);
                    state.clip = state.clip.intersection(transform(*rect, state));
                    let (x, y) = offsets.get(element).copied().unwrap_or_default();
                    state.x -= x.clamp(0.0, (size.width - rect.width).max(0.0)) * scale;
                    state.y -= y.clamp(0.0, (size.height - rect.height).max(0.0)) * scale;
                }
                SceneItem::Opacity(alpha) => {
                    stack.push(state);
                    state.alpha *= alpha.clamp(0.0, 1.0);
                }
                SceneItem::ScrollBars {
                    element,
                    rect,
                    size,
                } => {
                    let (dx, dy) = offsets.get(element).copied().unwrap_or_default();
                    let mut bars = Vec::new();
                    if size.height > rect.height && rect.height > 0.0 {
                        let thumb = (rect.height * rect.height / size.height)
                            .max(12.0)
                            .min(rect.height);
                        let y = rect.y
                            + (rect.height - thumb)
                                * (dy / (size.height - rect.height)).clamp(0.0, 1.0);
                        bars.push((Rect::new(rect.right() - 6.0, rect.y, 6.0, rect.height), 220));
                        bars.push((Rect::new(rect.right() - 5.0, y, 4.0, thumb), 120));
                    }
                    if size.width > rect.width && rect.width > 0.0 {
                        let thumb = (rect.width * rect.width / size.width)
                            .max(12.0)
                            .min(rect.width);
                        let x = rect.x
                            + (rect.width - thumb)
                                * (dx / (size.width - rect.width)).clamp(0.0, 1.0);
                        bars.push((Rect::new(rect.x, rect.bottom() - 6.0, rect.width, 6.0), 220));
                        bars.push((Rect::new(x, rect.bottom() - 5.0, thumb, 4.0), 120));
                    }
                    for (rect, color) in bars {
                        let mut q =
                            Quad::solid(transform(rect, state), Color::rgb(color, color, color));
                        q.color.a = (255.0 * state.alpha) as u8;
                        quads.push((q, state.clip));
                    }
                }
                SceneItem::Pop => {
                    if let Some(previous) = stack.pop() {
                        state = previous;
                    }
                }
            }
        }
        quads
    }
    #[allow(clippy::too_many_arguments)]
    pub fn paint_software(
        &self,
        pixels: &mut [u32],
        width: u32,
        height: u32,
        viewport: Rect,
        origin: (f32, f32),
        scale: f32,
        offsets: &ScrollOffsets,
    ) {
        software::paint(
            pixels,
            width,
            height,
            &self.visible_quads(viewport, origin, scale, offsets),
        );
    }
}

pub struct Rasterizer {
    cache: SwashCache,
    glyphs: HashMap<CacheKey, Option<Arc<RasterImage>>>,
    bytes: usize,
    pub skipped_glyphs: usize,
}
impl Default for Rasterizer {
    fn default() -> Self {
        Self {
            cache: SwashCache::new(),
            glyphs: HashMap::new(),
            bytes: 0,
            skipped_glyphs: 0,
        }
    }
}
impl Rasterizer {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    /// Rasterizes new glyphs once, sharing their images across all text runs.
    pub fn prepare(&mut self, list: &DisplayList, fonts: &mut FontSystem) -> Scene {
        let mut items = Vec::new();
        for item in &list.items {
            match item {
                DisplayItem::FillRect {
                    rect,
                    color,
                    radius,
                } => {
                    let mut q = Quad::solid(*rect, *color);
                    if *radius > 0.0 {
                        q.rounded = Some((*rect, *radius));
                    }
                    items.push(SceneItem::Quad(q));
                }
                DisplayItem::Border {
                    rect,
                    widths: b,
                    colors,
                    radius,
                } => {
                    for (r, color) in [
                        (Rect::new(rect.x, rect.y, rect.width, b.top), colors[0]),
                        (
                            Rect::new(
                                rect.right() - b.right,
                                rect.y + b.top,
                                b.right,
                                (rect.height - b.top - b.bottom).max(0.0),
                            ),
                            colors[1],
                        ),
                        (
                            Rect::new(rect.x, rect.bottom() - b.bottom, rect.width, b.bottom),
                            colors[2],
                        ),
                        (
                            Rect::new(
                                rect.x,
                                rect.y + b.top,
                                b.left,
                                (rect.height - b.top - b.bottom).max(0.0),
                            ),
                            colors[3],
                        ),
                    ] {
                        if r.width <= 0.0 || r.height <= 0.0 {
                            continue;
                        }
                        let mut q = Quad::solid(r, color);
                        if *radius > 0.0 {
                            q.rounded = Some((*rect, *radius));
                        }
                        items.push(SceneItem::Quad(q));
                    }
                }
                DisplayItem::TextRun(run) => {
                    for glyph in &run.glyphs {
                        if glyph.color.a == 0 {
                            continue;
                        }
                        let mut key = glyph.key;
                        let size = f32::from_bits(key.font_size_bits);
                        if !size.is_finite() || size <= 0.0 {
                            continue;
                        }
                        let factor = (size / 512.0).max(1.0);
                        if factor > 1.0 {
                            key.font_size_bits = 512.0f32.to_bits();
                        }
                        if !self.glyphs.contains_key(&key) {
                            let side = (size.clamp(1.0, 512.0) * 2.0 + 8.0).ceil() as usize;
                            if self.bytes.saturating_add(side * side * 4) > 64 * 1024 * 1024 {
                                self.skipped_glyphs += 1;
                                continue;
                            }
                            let image =
                                self.cache.get_image(fonts, key).as_ref().and_then(|image| {
                                    let mut rgba = Vec::new();
                                    match image.content {
                                        SwashContent::Mask => {
                                            for &a in &image.data {
                                                rgba.extend_from_slice(&[255, 255, 255, a]);
                                            }
                                        }
                                        SwashContent::SubpixelMask => {
                                            for c in image.data.chunks_exact(4) {
                                                rgba.extend_from_slice(&[
                                                    255,
                                                    255,
                                                    255,
                                                    c[0].max(c[1]).max(c[2]),
                                                ]);
                                            }
                                        }
                                        SwashContent::Color => rgba.extend_from_slice(&image.data),
                                    }
                                    RasterImage::new(
                                        image.placement.width,
                                        image.placement.height,
                                        rgba,
                                    )
                                });
                            self.bytes += image.as_ref().map_or(0, |i| i.rgba.len());
                            self.glyphs.insert(key, image);
                        }
                        if let (Some(Some(image)), Some(swash)) = (
                            self.glyphs.get(&key),
                            self.cache.get_image(fonts, key).as_ref(),
                        ) {
                            let p = swash.placement;
                            let rect = Rect::new(
                                glyph.x as f32 + p.left as f32 * factor,
                                glyph.y as f32 - p.top as f32 * factor,
                                p.width as f32 * factor,
                                p.height as f32 * factor,
                            );
                            let mut q = Quad::image(rect, image.clone());
                            q.color = if swash.content == SwashContent::Color {
                                Color {
                                    a: glyph.color.a,
                                    ..Color::WHITE
                                }
                            } else {
                                glyph.color
                            };
                            items.push(SceneItem::Quad(q));
                        }
                    }
                }
                DisplayItem::Image { rect, image } => {
                    items.push(SceneItem::Quad(Quad::image(*rect, image.clone())))
                }
                DisplayItem::Clip(rect) => items.push(SceneItem::Clip(*rect)),
                DisplayItem::ScrollFrame {
                    element,
                    rect,
                    size,
                } => items.push(SceneItem::Scroll {
                    element: *element,
                    rect: *rect,
                    size: *size,
                }),
                DisplayItem::StackingContext { opacity } => {
                    items.push(SceneItem::Opacity(*opacity))
                }
                DisplayItem::ScrollBars {
                    element,
                    rect,
                    size,
                } => items.push(SceneItem::ScrollBars {
                    element: *element,
                    rect: *rect,
                    size: *size,
                }),
                DisplayItem::EndClip
                | DisplayItem::EndScrollFrame
                | DisplayItem::EndStackingContext => items.push(SceneItem::Pop),
            }
        }
        Scene { items }
    }
}

pub enum Target<'a> {
    Software {
        pixels: &'a mut [u32],
        width: u32,
        height: u32,
    },
    Gpu(&'a mut gpu::GpuWindow),
}

/// The fixed compositing API accepts either backend. The shell retains its
/// prepared Scene between frames so scroll does not prepare/layout again.
pub struct Backend<'a> {
    pub target: Target<'a>,
    pub fonts: &'a mut FontSystem,
    pub rasterizer: &'a mut Rasterizer,
}
pub fn composite(
    backend: &mut Backend<'_>,
    list: &DisplayList,
    viewport: Rect,
) -> Result<(), String> {
    let scene = backend.rasterizer.prepare(list, backend.fonts);
    match &mut backend.target {
        Target::Software {
            pixels,
            width,
            height,
        } => scene.paint_software(
            pixels,
            *width,
            *height,
            viewport,
            (0.0, 0.0),
            1.0,
            &Default::default(),
        ),
        Target::Gpu(gpu) => {
            return gpu.present(&scene.visible_quads(
                viewport,
                (0.0, 0.0),
                1.0,
                &Default::default(),
            ))
        }
    }
    Ok(())
}
