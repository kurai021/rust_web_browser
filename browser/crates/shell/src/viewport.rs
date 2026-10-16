//! Shared graphical/headless flow viewport. Navigation chrome stays in shell.

use crate::page::Page;
use cosmic_text::FontSystem;
use css::ComputedStyles;
use layout::{LayoutOpts, LayoutResult, Rect, Size};
use paint::{build_display_list, Rasterizer, Scene};
pub fn observer_viewport(width: f64, height: f64) -> web_api::runtime::GeometryRect {
    web_api::runtime::GeometryRect {
        x: 0.0,
        y: 0.0,
        width,
        height,
    }
}

pub fn layout_page(
    page: &Page,
    styles: &ComputedStyles,
    viewport: Size,
    fonts: &mut FontSystem,
    rasterizer: &mut Rasterizer,
) -> (LayoutResult, Scene) {
    layout_page_cached(
        page,
        styles,
        viewport,
        fonts,
        rasterizer,
        &mut layout::LayoutCache::default(),
    )
}
pub fn layout_page_cached(
    page: &Page,
    styles: &ComputedStyles,
    viewport: Size,
    fonts: &mut FontSystem,
    rasterizer: &mut Rasterizer,
    cache: &mut layout::LayoutCache,
) -> (LayoutResult, Scene) {
    let mut tree = layout::build_render_tree(&page.document, styles);
    let images = page.image_cache.snapshot();
    let result = cache.layout(
        viewport,
        &mut tree,
        LayoutOpts {
            font_system: fonts,
            images: &images,
        },
    );
    let list = build_display_list(&result);
    let scene = rasterizer.prepare(&list, fonts);
    (result, scene)
}

pub struct HeadlessFrame {
    pub layout: LayoutResult,
    pub pixels: Vec<u32>,
    pub hash: u64,
    pub layout_ms: f64,
    pub paint_ms: f64,
}
pub fn headless_frame(
    page: &Page,
    styles: &ComputedStyles,
    viewport: Size,
    fonts: &mut FontSystem,
) -> HeadlessFrame {
    let width = viewport.width.clamp(1.0, 4096.0) as u32;
    let height = viewport.height.clamp(1.0, 4096.0) as u32;
    let start = std::time::Instant::now();
    let (layout, scene) = layout_page(
        page,
        styles,
        Size::new(width as f32, height as f32),
        fonts,
        &mut Rasterizer::default(),
    );
    let layout_ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut pixels = vec![0xffffff; width as usize * height as usize];
    let start = std::time::Instant::now();
    scene.paint_software(
        &mut pixels,
        width,
        height,
        Rect::new(0.0, 0.0, width as f32, height as f32),
        (0.0, 0.0),
        1.0,
        &Default::default(),
    );
    let paint_ms = start.elapsed().as_secs_f64() * 1000.0;
    // Deterministic FNV-1a pixel hash, independent of Rust's randomized hashes.
    let hash = pixels.iter().fold(0xcbf29ce484222325, |h, pixel| {
        pixel
            .to_le_bytes()
            .into_iter()
            .fold(h, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3))
    });
    HeadlessFrame {
        layout,
        pixels,
        hash,
        layout_ms,
        paint_ms,
    }
}
/// Linear geometry conversion on the JS/layout side, sharing paint scroll clips.
pub fn web_geometry(
    flow: &LayoutResult,
    offsets: &layout::ScrollOffsets,
) -> Vec<(html::NodeId, web_api::runtime::GeometryBox)> {
    use web_api::runtime::{GeometryBox, GeometryRect};
    #[derive(Clone, Copy, Default)]
    struct Context {
        x: f64,
        y: f64,
        clip: Option<GeometryRect>,
        fixed: bool,
    }
    fn rect(r: Rect, c: Context) -> GeometryRect {
        GeometryRect {
            x: r.x as f64 + c.x,
            y: r.y as f64 + c.y,
            width: r.width as f64,
            height: r.height as f64,
        }
    }
    fn intersection(a: GeometryRect, b: GeometryRect) -> GeometryRect {
        let x = a.x.max(b.x);
        let y = a.y.max(b.y);
        GeometryRect {
            x,
            y,
            width: ((a.x + a.width).min(b.x + b.width) - x).max(0.0),
            height: ((a.y + a.height).min(b.y + b.height) - y).max(0.0),
        }
    }
    fn union(a: GeometryRect, b: GeometryRect) -> GeometryRect {
        let x = a.x.min(b.x);
        let y = a.y.min(b.y);
        GeometryRect {
            x,
            y,
            width: (a.x + a.width).max(b.x + b.width) - x,
            height: (a.y + a.height).max(b.y + b.height) - y,
        }
    }
    let mut contexts = Vec::<Context>::with_capacity(flow.boxes.len());
    let mut geometry = std::collections::HashMap::<html::NodeId, GeometryBox>::new();
    for b in &flow.boxes {
        let mut context = b
            .parent
            .and_then(|p| contexts.get(p))
            .copied()
            .unwrap_or_default();
        if b.fixed && !context.fixed {
            context = Context {
                fixed: true,
                ..Default::default()
            };
        }
        if !b.anonymous && b.element.pseudo.is_none() {
            geometry.insert(
                b.element.node,
                GeometryBox {
                    border: rect(b.border_box, context),
                    content: rect(b.content_box, context),
                    fixed: b.fixed,
                    clip: context.clip,
                },
            );
        }
        if b.overflow != layout::Overflow::Visible {
            let clip = rect(b.padding_box, context);
            context.clip = Some(context.clip.map_or(clip, |old| intersection(old, clip)));
        }
        if matches!(
            b.overflow,
            layout::Overflow::Auto | layout::Overflow::Scroll
        ) {
            let (x, y) = offsets.get(&b.element).copied().unwrap_or_default();
            context.x -= x as f64;
            context.y -= y as f64;
        }
        contexts.push(context);
    }
    let boxed = geometry
        .keys()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    for target in &flow.targets {
        if target.element.pseudo.is_some() || boxed.contains(&target.element.node) {
            continue;
        }
        let mut context = target
            .ancestors
            .last()
            .and_then(|&i| contexts.get(i))
            .copied()
            .unwrap_or_default();
        if target.fixed && !context.fixed {
            context = Context {
                fixed: true,
                ..Default::default()
            };
        }
        let bounds = rect(target.rect, context);
        geometry
            .entry(target.element.node)
            .and_modify(|g| {
                g.border = union(g.border, bounds);
                g.content = g.border;
            })
            .or_insert(GeometryBox {
                border: bounds,
                content: bounds,
                fixed: target.fixed,
                clip: context.clip,
            });
    }
    geometry.into_iter().collect()
}
