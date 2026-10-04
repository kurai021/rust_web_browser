//! Software text rendering (plan/07 §7.4, plan/10 §10.4).
//!
//! `cosmic-text` layout + shaping, painted as alpha-blended fills onto a
//! `0x00RRGGBB` pixel buffer (the `softbuffer` format). This is the CPU
//! fallback path; the GPU path lands with `paint` in Phase 4.

use cosmic_text::{Attrs, Buffer, Color, FontSystem, Metrics, Shaping, SwashCache, Wrap};

/// Pack 8-bit channels into the `softbuffer` pixel format.
#[must_use]
pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// Writable pixel surface.
pub struct Canvas<'a> {
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
    /// Row-major `0x00RRGGBB` pixels.
    pub pixels: &'a mut [u32],
}

impl Canvas<'_> {
    /// Fill a rect, clipped to the canvas. Coordinates may be negative.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(self.w as i32);
        let y1 = (y + h).min(self.h as i32);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        for row in y0..y1 {
            let base = (row as u32 * self.w) as usize;
            for col in x0..x1 {
                self.pixels[base + col as usize] = color;
            }
        }
    }

    /// Alpha-blend a solid rect over the canvas. Colors pack as `(r, g, b, a)`.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn blend_rect(&mut self, x: i32, y: i32, w: u32, h: u32, rgba: (u8, u8, u8, u8)) {
        let (r, g, b, a) = rgba;
        if a == 0 {
            return;
        }
        let w = w.min(4096);
        let h = h.min(4096);
        if a == 255 {
            self.fill_rect(x, y, w as i32, h as i32, rgb(r, g, b));
            return;
        }
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w as i32).min(self.w as i32);
        let y1 = (y + h as i32).min(self.h as i32);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let alpha = u32::from(a);
        let inv = 255 - alpha;
        for row in y0..y1 {
            let base = (row as u32 * self.w) as usize;
            for col in x0..x1 {
                let dst = self.pixels[base + col as usize];
                let dr = (dst >> 16) & 0xFF;
                let dg = (dst >> 8) & 0xFF;
                let db = dst & 0xFF;
                let nr = (u32::from(r) * alpha + dr * inv) / 255;
                let ng = (u32::from(g) * alpha + dg * inv) / 255;
                let nb = (u32::from(b) * alpha + db * inv) / 255;
                self.pixels[base + col as usize] = (nr << 16) | (ng << 8) | nb;
            }
        }
    }
}

/// Shaped text prepared once, drawn every frame.
pub struct TextBuffer {
    buffer: Buffer,
    color: Color,
    wrap: Wrap,
}

impl TextBuffer {
    /// New buffer with fixed metrics. Requires the app `FontSystem`.
    pub fn with_metrics(font_system: &mut FontSystem, font_size: f32, wrap: Wrap) -> Self {
        let metrics = Metrics::new(font_size, font_size * 1.3);
        let mut buffer = Buffer::new(font_system, metrics);
        buffer.set_wrap(font_system, wrap);
        Self {
            buffer,
            color: Color::rgb(0x1A, 0x1A, 0x1A),
            wrap,
        }
    }

    /// Replace the text (re-shapes; call only on content change).
    pub fn set_text(&mut self, font_system: &mut FontSystem, text: &str) {
        self.buffer
            .set_text(font_system, text, Attrs::new(), Shaping::Advanced);
    }

    /// Resize the layout box (call on window resize).
    pub fn set_size(&mut self, font_system: &mut FontSystem, w: f32, h: f32) {
        self.buffer.set_size(font_system, Some(w), Some(h));
        self.buffer.set_wrap(font_system, self.wrap);
    }

    /// Vertical scroll offset in pixels.
    pub fn set_scroll_px(&mut self, _font_system: &mut FontSystem, vertical: f32) {
        self.buffer
            .set_scroll(cosmic_text::Scroll::new(0, vertical.max(0.0), 0.0));
    }

    /// Total laid-out height in pixels. Temporarily grows the layout box so
    /// one shape pass covers every line, then restores the real box. Call
    /// only when content or box size changes — never per frame.
    ///
    /// NOTE: never force this by setting a huge scroll offset — cosmic-text
    /// walks lines one by one and never terminates on extreme values.
    pub fn full_height(&mut self, font_system: &mut FontSystem) -> f32 {
        const TALL_BOX: f32 = 1_000_000.0;
        let saved_scroll = self.buffer.scroll();
        let (saved_w, saved_h) = self.buffer.size();
        self.buffer
            .set_scroll(cosmic_text::Scroll::new(0, 0.0, 0.0));
        self.buffer.set_size(font_system, saved_w, Some(TALL_BOX));
        self.buffer.shape_until_scroll(font_system, false);
        let height = self
            .buffer
            .layout_runs()
            .map(|run| run.line_y + run.line_height)
            .fold(0.0f32, f32::max);
        self.buffer.set_size(font_system, saved_w, saved_h);
        self.buffer.set_scroll(saved_scroll);
        height
    }

    /// Width of the first laid-out line (caret measurement for single-line boxes).
    pub fn first_line_width(&mut self, font_system: &mut FontSystem) -> f32 {
        self.buffer.shape_until_scroll(font_system, false);
        self.buffer
            .layout_runs()
            .next()
            .map(|run| run.line_w)
            .unwrap_or(0.0)
    }

    /// Paint glyphs at `(offset_x, offset_y)` clipped to `canvas`.
    pub fn draw(
        &self,
        font_system: &mut FontSystem,
        cache: &mut SwashCache,
        canvas: &mut Canvas<'_>,
        offset_x: i32,
        offset_y: i32,
    ) {
        let (w, h) = (canvas.w, canvas.h);
        let pixels = &mut canvas.pixels;
        self.buffer
            .draw(font_system, cache, self.color, |x, y, rw, rh, color| {
                let (r, g, b, a) = color.as_rgba_tuple();
                let mut tmp = Canvas { w, h, pixels };
                tmp.blend_rect(x + offset_x, y + offset_y, rw, rh, (r, g, b, a));
            });
    }
}

/// Shared glyph cache for all text buffers.
pub struct TextRenderer {
    /// Swash raster cache.
    pub cache: SwashCache,
}

impl TextRenderer {
    /// Empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: SwashCache::new(),
        }
    }
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_rgb_channels() {
        assert_eq!(rgb(0xFF, 0x00, 0x80), 0x00FF0080);
        assert_eq!(rgb(0, 0, 0), 0);
    }

    #[test]
    fn fill_rect_clips_to_canvas() {
        let mut pixels = vec![0u32; 4 * 4];
        let mut canvas = Canvas {
            w: 4,
            h: 4,
            pixels: &mut pixels,
        };
        canvas.fill_rect(-2, -2, 4, 4, rgb(255, 0, 0));
        assert_eq!(canvas.pixels[0], rgb(255, 0, 0));
        assert_eq!(canvas.pixels[5], rgb(255, 0, 0));
        assert_eq!(canvas.pixels[15], 0);
    }

    #[test]
    fn blend_rect_respects_alpha() {
        let mut pixels = vec![rgb(255, 255, 255); 2];
        let mut canvas = Canvas {
            w: 2,
            h: 1,
            pixels: &mut pixels,
        };
        canvas.blend_rect(0, 0, 1, 1, (0, 0, 0, 128));
        let px = canvas.pixels[0];
        let (r, g, b) = ((px >> 16) & 0xFF, (px >> 8) & 0xFF, px & 0xFF);
        assert!(r < 200 && r > 50 && r == g && g == b, "got {r},{g},{b}");
        assert_eq!(canvas.pixels[1], rgb(255, 255, 255));
        canvas.blend_rect(0, 0, 2, 1, (1, 2, 3, 0));
        assert_eq!(canvas.pixels[0], (r << 16) | (g << 8) | b);
    }
}
