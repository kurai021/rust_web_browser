//! Bounded CPU Canvas2D backing store; presentation reuses the retained image path.
use crate::*;
#[derive(Clone, Debug)]
pub struct Canvas {
    pub epoch: u64,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub fill: [u8; 4],
    pub stroke: [u8; 4],
    pub line_width: f64,
    pub path: Vec<Vec<(f64, f64)>>,
    pub state: Vec<Style>,
    pub alpha: f64,
    pub transform: [f64; 6],
    pub font: String,
    pub font_style: Arc<dom_bindings::css::ComputedStyle>,
    pub text_align: String,
    pub text_baseline: String,
    pub dirty: bool,
    pub tainted: bool,
}
use std::sync::Arc;
#[derive(Clone, Debug)]
pub struct Style {
    fill: [u8; 4],
    stroke: [u8; 4],
    line_width: f64,
    alpha: f64,
    transform: [f64; 6],
    font: String,
    font_style: Arc<dom_bindings::css::ComputedStyle>,
    align: String,
    baseline: String,
}
impl Canvas {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        if width > 4096 || height > 4096 {
            return Err(ApiError::new("QuotaExceededError", "canvas dimensions"));
        }
        let n = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .filter(|n| *n <= 16 * 1024 * 1024)
            .ok_or_else(|| ApiError::new("QuotaExceededError", "canvas dimensions"))?;
        Ok(Self {
            epoch: 0,
            width,
            height,
            pixels: vec![0; n],
            fill: [0, 0, 0, 255],
            stroke: [0, 0, 0, 255],
            line_width: 1.0,
            path: Vec::new(),
            state: Vec::new(),
            alpha: 1.0,
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            font: "10px sans-serif".into(),
            font_style: Arc::new(font_style("10px sans-serif").unwrap_or_default()),
            text_align: "start".into(),
            text_baseline: "alphabetic".into(),
            dirty: true,
            tainted: false,
        })
    }
    pub fn pixel(&mut self, x: i32, y: i32, color: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        let a = (color[3] as f64 * self.alpha).round() as u32;
        let old = self.pixels[i + 3] as u32;
        let out = a + old * (255 - a) / 255;
        if out == 0 {
            self.pixels[i..i + 4].fill(0);
            return;
        }
        for (j, c) in color.iter().enumerate().take(3) {
            self.pixels[i + j] =
                ((*c as u32 * a + self.pixels[i + j] as u32 * old * (255 - a) / 255) / out) as u8;
        }
        self.pixels[i + 3] = out as u8;
    }
    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, clear: bool) {
        if [x, y, w, h].iter().any(|v| !v.is_finite()) {
            return;
        }
        let (l, r) = (
            x.min(x + w).floor().max(0.0) as u32,
            x.max(x + w).ceil().clamp(0.0, self.width as f64) as u32,
        );
        let (t, b) = (
            y.min(y + h).floor().max(0.0) as u32,
            y.max(y + h).ceil().clamp(0.0, self.height as f64) as u32,
        );
        for yy in t.min(self.height)..b {
            for xx in l.min(self.width)..r {
                if clear {
                    let i = (yy as usize * self.width as usize + xx as usize) * 4;
                    self.pixels[i..i + 4].fill(0);
                } else {
                    self.pixel(xx as i32, yy as i32, self.fill);
                }
            }
        }
        self.dirty = true;
    }
    pub fn line(&mut self, a: (f64, f64), b: (f64, f64)) -> Result<()> {
        if [a.0, a.1, b.0, b.1].iter().any(|n| !n.is_finite()) {
            return Ok(());
        }
        let steps = (a.0 - b.0)
            .abs()
            .max((a.1 - b.1).abs())
            .ceil()
            .clamp(1.0, 16384.0) as usize;
        let radius = (self.line_width / 2.0).clamp(0.0, 32.0) as i32;
        if steps.saturating_mul(((radius * 2 + 1) * (radius * 2 + 1)) as usize) > 2_000_000 {
            return Err(ApiError::new(
                "QuotaExceededError",
                "canvas stroke work budget",
            ));
        }
        for i in 0..=steps {
            let f = i as f64 / steps as f64;
            let x = (a.0 + (b.0 - a.0) * f).round() as i32;
            let y = (a.1 + (b.1 - a.1) * f).round() as i32;
            for yy in -radius..=radius {
                for xx in -radius..=radius {
                    self.pixel(x.saturating_add(xx), y.saturating_add(yy), self.stroke);
                }
            }
        }
        self.dirty = true;
        Ok(())
    }
    pub fn fill_path(&mut self, clear: bool, even_odd: bool) {
        if self.path.iter().all(|p| p.len() < 3) {
            return;
        }
        let paths = self.path.clone();
        for y in 0..self.height {
            let yy = y as f64 + 0.5;
            let mut xs = Vec::new();
            for path in &paths {
                for (a, b) in path
                    .iter()
                    .zip(path.iter().cycle().skip(1))
                    .take(path.len())
                {
                    if (a.1 > yy) != (b.1 > yy) {
                        xs.push((
                            a.0 + (yy - a.1) * (b.0 - a.0) / (b.1 - a.1),
                            if b.1 > a.1 { 1 } else { -1 },
                        ));
                    }
                }
            }
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0;
            let mut left = 0.0f64;
            for (x, delta) in xs {
                let inside = if even_odd {
                    winding % 2 != 0
                } else {
                    winding != 0
                };
                if inside {
                    for x in
                        left.ceil().max(0.0) as u32..x.ceil().clamp(0.0, self.width as f64) as u32
                    {
                        if clear {
                            let i = (y as usize * self.width as usize + x as usize) * 4;
                            self.pixels[i..i + 4].fill(0);
                        } else {
                            self.pixel(x as i32, y as i32, self.fill);
                        }
                    }
                }
                winding += delta;
                left = x;
            }
        }
        self.dirty = true;
    }
    #[allow(clippy::too_many_arguments)]
    pub fn image(
        &mut self,
        rgba: &[u8],
        iw: u32,
        ih: u32,
        sx: f64,
        sy: f64,
        sw: f64,
        sh: f64,
        dx: f64,
        dy: f64,
        dw: f64,
        dh: f64,
    ) {
        if iw == 0
            || ih == 0
            || (iw as usize)
                .checked_mul(ih as usize)
                .and_then(|n| n.checked_mul(4))
                != Some(rgba.len())
            || [sx, sy, sw, sh, dx, dy, dw, dh]
                .iter()
                .any(|n| !n.is_finite())
            || dw <= 0.0
            || dh <= 0.0
        {
            return;
        }
        for y in
            dy.floor().max(0.0) as u32..((dy + dh).ceil().clamp(0.0, self.height as f64) as u32)
        {
            for x in
                dx.floor().max(0.0) as u32..((dx + dw).ceil().clamp(0.0, self.width as f64) as u32)
            {
                let xx = (sx + (x as f64 - dx) * sw / dw).floor();
                let yy = (sy + (y as f64 - dy) * sh / dh).floor();
                if xx < 0.0 || yy < 0.0 || xx >= iw as f64 || yy >= ih as f64 {
                    continue;
                }
                let (xx, yy) = (xx as usize, yy as usize);
                let i = (yy * iw as usize + xx) * 4;
                self.pixel(
                    x as i32,
                    y as i32,
                    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]],
                );
            }
        }
        self.dirty = true;
    }
    pub fn point(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.transform;
        (a * x + c * y + e, b * x + d * y + f)
    }
    pub fn multiply(&mut self, m: [f64; 6]) {
        let [a, b, c, d, e, f] = self.transform;
        let [g, h, i, j, k, l] = m;
        let result = [
            a * g + c * h,
            b * g + d * h,
            a * i + c * j,
            b * i + d * j,
            a * k + c * l + e,
            b * k + d * l + f,
        ];
        if result
            .iter()
            .all(|n| n.is_finite() && n.abs() <= 1_000_000.0)
        {
            self.transform = result;
        }
    }
    pub fn save(&mut self) -> Result<()> {
        if self.state.len() >= 128 {
            return Err(ApiError::new("QuotaExceededError", "canvas state stack"));
        }
        self.state.push(Style {
            fill: self.fill,
            stroke: self.stroke,
            line_width: self.line_width,
            alpha: self.alpha,
            transform: self.transform,
            font: self.font.clone(),
            font_style: self.font_style.clone(),
            align: self.text_align.clone(),
            baseline: self.text_baseline.clone(),
        });
        Ok(())
    }
    pub fn restore(&mut self) {
        if let Some(s) = self.state.pop() {
            self.fill = s.fill;
            self.stroke = s.stroke;
            self.line_width = s.line_width;
            self.alpha = s.alpha;
            self.transform = s.transform;
            self.font = s.font;
            self.font_style = s.font_style;
            self.text_align = s.align;
            self.text_baseline = s.baseline;
        }
    }
    pub fn stroke_path(&mut self) -> Result<()> {
        let scale = self.transform[0]
            .hypot(self.transform[1])
            .max(self.transform[2].hypot(self.transform[3]));
        let width = self.line_width * scale;
        if width > 64.0 {
            return Err(ApiError::new("QuotaExceededError", "canvas stroke width"));
        }
        let radius = (width / 2.0).floor() as usize;
        let work = self
            .path
            .iter()
            .flat_map(|path| path.windows(2))
            .map(|p| {
                (p[0].0 - p[1].0)
                    .abs()
                    .max((p[0].1 - p[1].1).abs())
                    .ceil()
                    .clamp(1.0, 16384.0) as usize
            })
            .fold(0usize, usize::saturating_add);
        if work.saturating_mul((radius * 2 + 1).saturating_pow(2)) > 2_000_000 {
            return Err(ApiError::new(
                "QuotaExceededError",
                "canvas stroke work budget",
            ));
        }
        let old = self.line_width;
        self.line_width = width;
        let path = self.path.clone();
        let result = (|| {
            for path in path {
                for pair in path.windows(2) {
                    self.line(pair[0], pair[1])?;
                }
            }
            Ok(())
        })();
        self.line_width = old;
        result
    }
    #[allow(clippy::too_many_arguments)]
    pub fn transformed_image(
        &mut self,
        pixels: &[u8],
        iw: u32,
        ih: u32,
        sx: f64,
        sy: f64,
        sw: f64,
        sh: f64,
        dx: f64,
        dy: f64,
        dw: f64,
        dh: f64,
    ) {
        if self.transform == [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] {
            self.image(pixels, iw, ih, sx, sy, sw, sh, dx, dy, dw, dh);
            return;
        }
        if dw <= 0.0 || dh <= 0.0 {
            return;
        }
        let [a, b, c, d, e, f] = self.transform;
        let determinant = a * d - b * c;
        if !determinant.is_finite() || determinant == 0.0 {
            return;
        }
        let corners = [
            self.point(dx, dy),
            self.point(dx + dw, dy),
            self.point(dx, dy + dh),
            self.point(dx + dw, dy + dh),
        ];
        let (left, right) = corners
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, r), p| {
                (l.min(p.0), r.max(p.0))
            });
        let (top, bottom) = corners
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, r), p| {
                (l.min(p.1), r.max(p.1))
            });
        for y in top.floor().max(0.0) as u32..bottom.ceil().clamp(0.0, self.height as f64) as u32 {
            for x in left.floor().max(0.0) as u32..right.ceil().clamp(0.0, self.width as f64) as u32
            {
                let (px, py) = (x as f64 + 0.5 - e, y as f64 + 0.5 - f);
                let (px, py) = (
                    (d * px - c * py) / determinant,
                    (-b * px + a * py) / determinant,
                );
                if px < dx || py < dy || px >= dx + dw || py >= dy + dh {
                    continue;
                }
                let (xx, yy) = (
                    (sx + (px - dx) * sw / dw).floor(),
                    (sy + (py - dy) * sh / dh).floor(),
                );
                if xx < 0.0 || yy < 0.0 || xx >= iw as f64 || yy >= ih as f64 {
                    continue;
                }
                let i = (yy as usize * iw as usize + xx as usize) * 4;
                if let Some(pixel) = pixels.get(i..i + 4) {
                    self.pixel(x as i32, y as i32, [pixel[0], pixel[1], pixel[2], pixel[3]]);
                }
            }
        }
        self.dirty = true;
    }
}
pub fn font_style(font: &str) -> Option<dom_bindings::css::ComputedStyle> {
    use dom_bindings::{css, html};
    if font.len() > 512 {
        return None;
    }
    let declaration = css::syntax::parse_declarations(&format!("font:{font}"));
    let declaration = declaration.first()?;
    if !css::style::supported_property("font", &declaration.value) {
        return None;
    }
    let mut doc = html::Document::new();
    let node = doc.create_element("span", html::Namespace::Html);
    doc.append_child(0, node);
    doc.set_attribute(node, "style", &format!("font:{font}"))
        .ok()?;
    css::Cascade::default()
        .compute(&doc, &[])
        .get(node)
        .map(|s| (**s).clone())
}
