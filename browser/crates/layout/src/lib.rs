//! In-house Stage A flow layout (plan/07). No network or window ownership.
//! DOM identities survive anonymous boxes, generated content and re-layout.

mod flow;
pub mod geometry;
pub mod text;
mod tree;

pub use flow::layout;
pub use geometry::{Edges, Rect, Size};
pub use tree::{build_render_tree, ElementId, RenderKind, RenderNode, RenderTree, Replaced};

use cosmic_text::{CacheKey, FontSystem};
use css::ComputedStyle;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use url::Url;

pub const MAX_BOX_DEPTH: usize = 512;
pub const MAX_GLYPHS: usize = 1_000_000;
pub const MAX_EXTENT: f32 = 10_000_000.0;

/// Straight-alpha sRGB image shared by decoded assets and glyph raster caches.
#[derive(Debug)]
pub struct RasterImage {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}
impl RasterImage {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Arc<Self>> {
        let len = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        if width == 0 || height == 0 || len != rgba.len() {
            return None;
        }
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Some(Arc::new(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            width,
            height,
            rgba: rgba.into(),
        }))
    }
    pub fn size(&self) -> Size {
        Size::new(self.width as f32, self.height as f32)
    }
}

pub type Images = HashMap<Url, Arc<RasterImage>>;

pub struct LayoutOpts<'a> {
    pub font_system: &'a mut FontSystem,
    pub images: &'a Images,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    Visible,
    Hidden,
    Scroll,
    Auto,
}

#[derive(Debug, Clone)]
pub struct Glyph {
    pub key: CacheKey,
    pub x: i32,
    pub y: i32,
    pub color: css::values::Color,
}

#[derive(Debug, Clone)]
pub struct TextRun {
    pub rect: Rect,
    pub glyphs: Vec<Glyph>,
    pub text: String,
}

#[derive(Debug, Clone)]
pub enum Content {
    Box(usize),
    Text(TextRun),
    Image {
        rect: Rect,
        image: Arc<RasterImage>,
    },
    Decoration {
        rect: Rect,
        color: css::values::Color,
    },
}

#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub element: ElementId,
    pub anonymous: bool,
    pub style: Arc<ComputedStyle>,
    pub border_box: Rect,
    pub padding_box: Rect,
    pub content_box: Rect,
    pub border: Edges,
    pub content: Vec<Content>,
    pub overflow: Overflow,
    pub scroll_size: Size,
    pub radius: f32,
    pub background_image: Option<Arc<RasterImage>>,
}

#[derive(Debug, Clone)]
pub struct HitRegion {
    pub rect: Rect,
    pub href: Url,
    /// Overflow ancestors, outermost first. Scroll transforms are paint-only.
    pub ancestors: Vec<usize>,
}
#[derive(Debug, Clone)]
pub struct TargetRegion {
    pub element: ElementId,
    pub rect: Rect,
    pub ancestors: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct LayoutStats {
    pub boxes: usize,
    pub lines: usize,
    pub glyphs: usize,
    pub depth_fallbacks: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default)]
pub struct LayoutResult {
    pub boxes: Vec<LayoutBox>,
    pub roots: Vec<usize>,
    pub hits: Vec<HitRegion>,
    pub targets: Vec<TargetRegion>,
    pub content_size: Size,
    pub viewport: Size,
    pub stats: LayoutStats,
}

pub type ScrollOffsets = HashMap<ElementId, (f32, f32)>;

impl LayoutResult {
    pub fn hit_element(&self, x: f32, y: f32, offsets: &ScrollOffsets) -> Option<ElementId> {
        fn local(
            result: &LayoutResult,
            x: f32,
            y: f32,
            ancestors: &[usize],
            offsets: &ScrollOffsets,
        ) -> Option<(f32, f32)> {
            let (mut x, mut y) = (x, y);
            for &i in ancestors {
                let b = &result.boxes[i];
                if b.overflow != Overflow::Visible && !b.padding_box.contains(x, y) {
                    return None;
                }
                if matches!(b.overflow, Overflow::Auto | Overflow::Scroll) {
                    let (dx, dy) = offsets.get(&b.element).copied().unwrap_or_default();
                    x += dx;
                    y += dy;
                }
            }
            Some((x, y))
        }
        for target in self.targets.iter().rev() {
            if let Some((px, py)) = local(self, x, y, &target.ancestors, offsets) {
                if target.rect.contains(px, py) {
                    return Some(target.element);
                }
            }
        }
        self.boxes
            .iter()
            .rev()
            .find(|b| !b.anonymous && b.style.visible && b.border_box.contains(x, y))
            .map(|b| b.element)
    }
    /// Links use the same overflow clips and nested offsets as painting.
    pub fn hit_link(&self, x: f32, y: f32, offsets: &ScrollOffsets) -> Option<Url> {
        self.hits.iter().rev().find_map(|hit| {
            let (mut px, mut py) = (x, y);
            for &index in &hit.ancestors {
                let b = &self.boxes[index];
                if b.overflow != Overflow::Visible && !b.padding_box.contains(px, py) {
                    return None;
                }
                if matches!(b.overflow, Overflow::Scroll | Overflow::Auto) {
                    let (dx, dy) = offsets.get(&b.element).copied().unwrap_or_default();
                    px += dx;
                    py += dy;
                }
            }
            hit.rect.contains(px, py).then(|| hit.href.clone())
        })
    }

    /// Scroll the deepest overflow container under the pointer. No layout.
    pub fn scroll_at(&self, x: f32, y: f32, dy: f32, offsets: &mut ScrollOffsets) -> bool {
        self.scroll_by_at(x, y, (0.0, dy), offsets)
    }
    pub fn scroll_by_at(
        &self,
        x: f32,
        y: f32,
        delta: (f32, f32),
        offsets: &mut ScrollOffsets,
    ) -> bool {
        fn find(
            result: &LayoutResult,
            index: usize,
            x: f32,
            y: f32,
            delta: (f32, f32),
            offsets: &mut ScrollOffsets,
        ) -> bool {
            let b = &result.boxes[index];
            if b.overflow != Overflow::Visible && !b.padding_box.contains(x, y) {
                return false;
            }
            let (sx, sy) = if matches!(b.overflow, Overflow::Scroll | Overflow::Auto) {
                offsets.get(&b.element).copied().unwrap_or_default()
            } else {
                (0.0, 0.0)
            };
            for child in b.content.iter().rev() {
                if let Content::Box(child) = child {
                    if find(result, *child, x + sx, y + sy, delta, offsets) {
                        return true;
                    }
                }
            }
            if matches!(b.overflow, Overflow::Scroll | Overflow::Auto)
                && b.padding_box.contains(x, y)
            {
                let max = (b.scroll_size.height - b.padding_box.height).max(0.0);
                let next = (sy + delta.1).clamp(0.0, max);
                let nx =
                    (sx + delta.0).clamp(0.0, (b.scroll_size.width - b.padding_box.width).max(0.0));
                if (next - sy).abs() > 0.01 || (nx - sx).abs() > 0.01 {
                    offsets.insert(b.element, (nx, next));
                    return true;
                }
            }
            false
        }
        self.roots
            .iter()
            .rev()
            .any(|&root| find(self, root, x, y, delta, offsets))
    }
}
