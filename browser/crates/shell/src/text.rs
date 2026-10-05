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
        let x1 = x.saturating_add(w).min(self.w as i32);
        let y1 = y.saturating_add(h).min(self.h as i32);
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
        let x1 = x.saturating_add(w as i32).min(self.w as i32);
        let y1 = y.saturating_add(h as i32).min(self.h as i32);
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

    /// CSS attributes are applied to the existing shaping path. Inline
    /// spans share their surrounding run, preserving kerning and hit offsets.
    pub fn set_styled_text(
        &mut self,
        font_system: &mut FontSystem,
        text: &str,
        style: &css::ComputedStyle,
        spans: &[crate::article::InlineStyleSpan],
    ) {
        let size = style.font_size.clamp(0.1, 10_000.0);
        self.buffer.set_metrics(
            font_system,
            Metrics::new(size, style.line_height.pixels(size).clamp(0.1, 20_000.0)),
        );
        self.wrap = match style.white_space {
            css::style::WhiteSpace::Pre | css::style::WhiteSpace::NoWrap => Wrap::None,
            _ => Wrap::WordOrGlyph,
        };
        self.buffer.set_wrap(font_system, self.wrap);
        let mut cuts = std::collections::BTreeSet::from([0, text.len()]);
        for span in spans {
            if span.end <= text.len()
                && text.is_char_boundary(span.start)
                && text.is_char_boundary(span.end)
            {
                cuts.insert(span.start);
                cuts.insert(span.end);
            }
        }
        let cuts: Vec<usize> = cuts.into_iter().collect();
        let mut runs = Vec::new();
        for window in cuts.windows(2) {
            let (start, end) = (window[0], window[1]);
            let selected = spans
                .iter()
                .rev()
                .find(|s| s.start <= start && s.end >= end)
                .map(|s| &*s.style)
                .unwrap_or(style);
            runs.push((start, end, style_attrs(selected, font_system)));
        }
        let default_attrs = style_attrs(style, font_system);
        self.buffer.set_rich_text(
            font_system,
            runs.iter()
                .map(|(start, end, attrs)| (&text[*start..*end], attrs.as_attrs())),
            default_attrs.as_attrs(),
            Shaping::Advanced,
        );
        self.buffer.shape_until_scroll(font_system, false);
    }

    pub fn set_alignment(&mut self, font_system: &mut FontSystem, align: css::style::TextAlign) {
        let align = match align {
            css::style::TextAlign::Right | css::style::TextAlign::End => cosmic_text::Align::Right,
            css::style::TextAlign::Center => cosmic_text::Align::Center,
            css::style::TextAlign::Justify => cosmic_text::Align::Justified,
            _ => cosmic_text::Align::Left,
        };
        for line in &mut self.buffer.lines {
            line.set_align(Some(align));
        }
        self.buffer.shape_until_scroll(font_system, false);
    }

    /// Geometry from shaped glyphs, including wrapped/RTL text. Unlike the
    /// Phase 2 substring overlays this does not reshape a slice in isolation.
    pub fn range_rects(
        &mut self,
        font_system: &mut FontSystem,
        start: usize,
        end: usize,
    ) -> Vec<(f32, f32, f32, f32, f32)> {
        self.buffer.shape_until_scroll(font_system, false);
        let offsets = self.line_offsets();
        let mut rects = Vec::new();
        for run in self.buffer.layout_runs() {
            let base = offsets.get(run.line_i).copied().unwrap_or(0);
            let mut left = f32::INFINITY;
            let mut right = f32::NEG_INFINITY;
            for glyph in run.glyphs {
                if glyph.end + base > start && glyph.start + base < end {
                    left = left.min(glyph.x);
                    right = right.max(glyph.x + glyph.w);
                }
            }
            if left.is_finite() && right >= left {
                rects.push((
                    left,
                    run.line_top,
                    run.line_y,
                    right - left,
                    run.line_height,
                ));
            }
        }
        rects
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

    /// Ink color for the next draws.
    pub fn set_color(&mut self, color: Color) {
        self.color = color;
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

    /// Top offset of the first laid-out line (overlay alignment).
    pub fn first_line_top(&mut self, font_system: &mut FontSystem) -> Option<f32> {
        self.buffer.shape_until_scroll(font_system, false);
        self.buffer.layout_runs().next().map(|run| run.line_top)
    }

    /// Byte offset of line starts in the current text (for hit-testing).
    fn line_offsets(&self) -> Vec<usize> {
        let mut offsets = Vec::with_capacity(self.buffer.lines.len() + 1);
        let mut offset = 0usize;
        for line in &self.buffer.lines {
            offsets.push(offset);
            offset += line.text().len() + 1;
        }
        offsets.push(offset);
        offsets
    }

    /// Map a buffer-local hit to a byte offset in the text.
    pub fn hit_offset(&mut self, font_system: &mut FontSystem, x: f32, y: f32) -> Option<usize> {
        self.buffer.shape_until_scroll(font_system, false);
        let cursor = self.buffer.hit(x, y)?;
        let offsets = self.line_offsets();
        let line_start = *offsets.get(cursor.line)?;
        let line_len = self.buffer.lines.get(cursor.line)?.text().len();
        Some(line_start + cursor.index.min(line_len))
    }

    /// Visual fragments of the byte range `[start, end)`: one per wrapped
    /// line as `(x, line_top, baseline, width, slice)`.
    ///
    /// Call after the buffer was fully shaped (e.g. right after
    /// `full_height`); this also forces a full shape pass to be safe.
    /// Slice shaping may differ sub-pixel from in-context shaping at run
    /// boundaries; good enough for link overlays (documented, Phase 2).
    pub fn link_fragments(
        &mut self,
        font_system: &mut FontSystem,
        text: &str,
        start: usize,
        end: usize,
    ) -> Vec<(f32, f32, f32, f32, String)> {
        let saved_scroll = self.buffer.scroll();
        let (saved_w, saved_h) = self.buffer.size();
        self.buffer
            .set_scroll(cosmic_text::Scroll::new(0, 0.0, 0.0));
        // Tall box lays out every line in one pass.
        if let Some(w) = saved_w {
            self.buffer
                .set_size(font_system, Some(w), Some(1_000_000.0));
        }
        self.buffer.shape_until_scroll(font_system, false);
        let mut fragments = Vec::new();
        let offsets = self.line_offsets();
        for run in self.buffer.layout_runs() {
            let line_start = offsets.get(run.line_i).copied().unwrap_or(0);
            let line_text = self
                .buffer
                .lines
                .get(run.line_i)
                .map(|line| line.text())
                .unwrap_or("");
            let (run_start, run_end) = glyph_byte_range(&run);
            let piece_start = start.max(line_start + run_start);
            let piece_end = end.min(line_start + run_end);
            if piece_start >= piece_end {
                continue;
            }
            let slice = text.get(piece_start..piece_end).unwrap_or("").to_owned();
            if slice.is_empty() {
                continue;
            }
            let run_x = run
                .glyphs
                .iter()
                .map(|glyph| glyph.x)
                .fold(f32::INFINITY, f32::min);
            let run_x = if run_x.is_finite() { run_x } else { 0.0 };
            // Prefix measured from the RUN start (visual line), not the
            // logical line start: wrapped runs share one BufferLine.
            let prefix_end = piece_start - line_start;
            let from = run_start.min(prefix_end).min(line_text.len());
            let to = prefix_end.min(line_text.len());
            let prefix = &line_text[from..to];
            let dx = measure_slice(font_system, prefix, self.buffer.metrics());
            let width = measure_slice(font_system, &slice, self.buffer.metrics());
            fragments.push((run_x + dx, run.line_top, run.line_y, width, slice));
        }
        self.buffer.set_size(font_system, saved_w, saved_h);
        self.buffer.set_scroll(saved_scroll);
        fragments
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

fn style_attrs(style: &css::ComputedStyle, system: &FontSystem) -> cosmic_text::AttrsOwned {
    use cosmic_text::{Family, Style, Weight};
    let mut family = Family::Serif;
    let mut web_name = None;
    for name in &style.font_family {
        let alias = crate::fonts::web_family(name);
        if system
            .db()
            .faces()
            .any(|f| f.families.iter().any(|(face, _)| face == &alias))
        {
            web_name = Some(alias);
            break;
        }
        let generic = match name.to_ascii_lowercase().as_str() {
            "serif" => Some(Family::Serif),
            "sans-serif" | "system-ui" => Some(Family::SansSerif),
            "monospace" => Some(Family::Monospace),
            "cursive" => Some(Family::Cursive),
            "fantasy" => Some(Family::Fantasy),
            _ => None,
        };
        if let Some(generic) = generic {
            family = generic;
            break;
        }
        if system.db().faces().any(|f| {
            f.families
                .iter()
                .any(|(face, _)| face.eq_ignore_ascii_case(name))
        }) {
            family = Family::Name(name);
            break;
        }
    }
    if let Some(name) = &web_name {
        family = Family::Name(name);
    }
    let color = style.color;
    let alpha = if style.visible && style.font_size > 0.0 {
        (color.a as f32 * style.opacity) as u8
    } else {
        0
    };
    let size = style.font_size.clamp(0.1, 10_000.0);
    cosmic_text::AttrsOwned::new(
        Attrs::new()
            .family(family)
            .weight(Weight(style.font_weight))
            .style(if style.italic {
                Style::Italic
            } else {
                Style::Normal
            })
            .color(Color::rgba(color.r, color.g, color.b, alpha))
            .metrics(Metrics::new(
                size,
                style.line_height.pixels(size).clamp(0.1, 20_000.0),
            )),
    )
}

/// Byte range in the line covered by a run's glyphs.
fn glyph_byte_range(run: &cosmic_text::LayoutRun<'_>) -> (usize, usize) {
    let mut start = usize::MAX;
    let mut end = 0usize;
    for glyph in run.glyphs {
        start = start.min(glyph.start);
        end = end.max(glyph.end);
    }
    if end <= start {
        (0, 0)
    } else {
        (start, end)
    }
}

/// Advance width of `text` on one unwrapped line with the given metrics.
fn measure_slice(font_system: &mut FontSystem, text: &str, metrics: cosmic_text::Metrics) -> f32 {
    let mut scratch = Buffer::new(font_system, metrics);
    scratch.set_wrap(font_system, Wrap::None);
    scratch.set_size(font_system, Some(100_000.0), Some(metrics.line_height));
    scratch.set_text(font_system, text, Attrs::new(), Shaping::Advanced);
    scratch.shape_until_scroll(font_system, false);
    scratch
        .layout_runs()
        .next()
        .map(|run| run.line_w)
        .unwrap_or(0.0)
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
