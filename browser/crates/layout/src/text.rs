//! Shared font selection and bounded shaping. Bidi stays in Rust.

use cosmic_text::{
    Attrs, AttrsOwned, Buffer, Family, FontSystem, LayoutGlyph, Metrics, Shaping, Style, Weight,
    Wrap,
};
use css::ComputedStyle;
use std::collections::HashMap;

pub fn web_family(name: &str) -> String {
    format!("__browser_web_{}", name.to_ascii_lowercase())
}

pub fn font_attrs(style: &ComputedStyle, system: &FontSystem) -> AttrsOwned {
    let mut family = Family::Serif;
    let mut web_name = None;
    for name in &style.font_family {
        let alias = web_family(name);
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
    AttrsOwned::new(
        Attrs::new()
            .family(family)
            .weight(Weight(style.font_weight))
            .style(if style.italic {
                Style::Italic
            } else {
                Style::Normal
            }),
    )
}

#[derive(Debug, Clone)]
pub(crate) struct Shaped {
    pub glyphs: Vec<LayoutGlyph>,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
}

#[derive(Hash, PartialEq, Eq)]
struct Key {
    text: String,
    size: u32,
    height: u32,
    weight: u16,
    italic: bool,
    family: Vec<String>,
    rtl: bool,
}

pub(crate) struct Shaper<'a> {
    pub system: &'a mut FontSystem,
    cache: HashMap<Key, Shaped>,
}
impl<'a> Shaper<'a> {
    pub fn new(system: &'a mut FontSystem) -> Self {
        Self {
            system,
            cache: HashMap::new(),
        }
    }
    pub fn shape(&mut self, text: &str, style: &ComputedStyle) -> Shaped {
        let size = style.font_size.clamp(0.1, 10_000.0);
        let height = style.line_height.pixels(size).clamp(0.1, 20_000.0);
        let key = Key {
            text: text.into(),
            size: size.to_bits(),
            height: height.to_bits(),
            weight: style.font_weight,
            italic: style.italic,
            family: style.font_family.clone(),
            rtl: style.get_property_value("direction") == "rtl",
        };
        if let Some(shape) = self.cache.get(&key) {
            return shape.clone();
        }
        let mut buffer = Buffer::new(self.system, Metrics::new(size, height));
        buffer.set_wrap(self.system, Wrap::None);
        buffer.set_size(self.system, Some(crate::MAX_EXTENT), Some(height * 2.0));
        let attrs = font_attrs(style, self.system);
        buffer.set_text(self.system, text, attrs.as_attrs(), Shaping::Advanced);
        buffer.shape_until_scroll(self.system, false);
        let mut shape = Shaped {
            glyphs: Vec::new(),
            width: 0.0,
            ascent: height * 0.8,
            descent: height * 0.2,
        };
        if let Some(run) = buffer.layout_runs().next() {
            shape.width = run.line_w;
            shape.ascent = run.line_y - run.line_top;
            shape.descent = (run.line_height - shape.ascent).max(0.0);
            let left = run.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
            shape.glyphs = run.glyphs.to_vec();
            if left.is_finite() {
                for glyph in &mut shape.glyphs {
                    glyph.x -= left;
                }
            }
        }
        if self.cache.len() < 4096 && text.len() <= 4096 {
            self.cache.insert(key, shape.clone());
        }
        shape
    }
}
