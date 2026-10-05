//! Shared graphical/headless flow viewport. Navigation chrome stays in shell.

use crate::page::Page;
use cosmic_text::FontSystem;
use css::ComputedStyles;
use layout::{LayoutOpts, LayoutResult, Rect, Size};
use paint::{build_display_list, Rasterizer, Scene};

pub fn layout_page(
    page: &Page,
    styles: &ComputedStyles,
    viewport: Size,
    fonts: &mut FontSystem,
    rasterizer: &mut Rasterizer,
) -> (LayoutResult, Scene) {
    let mut tree = layout::build_render_tree(&page.document, styles);
    let images = page.image_cache.snapshot();
    let result = layout::layout(
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
