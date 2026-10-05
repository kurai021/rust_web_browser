//! CSS values used by the Phase 3 cascade (plan/06 §6.2).
//! Length expressions keep relative units until a caller supplies their
//! containing-block/font context. Math uses dimension-checked arithmetic.

use crate::syntax::{block_end, split_top_level, trim};
use crate::token::Token;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}
impl Color {
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const TRANSPARENT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
    pub fn css(self) -> String {
        if self.a == 255 {
            format!("rgb({}, {}, {})", self.r, self.g, self.b)
        } else {
            format!(
                "rgba({}, {}, {}, {})",
                self.r,
                self.g,
                self.b,
                self.a as f32 / 255.0
            )
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorValue {
    Rgba(Color),
    CurrentColor,
}
impl ColorValue {
    pub fn resolve(self, current: Color) -> Color {
        match self {
            Self::Rgba(c) => c,
            Self::CurrentColor => current,
        }
    }
}

pub fn parse_color(tokens: &[Token]) -> Option<ColorValue> {
    let tokens = trim(tokens);
    match tokens {
        [Token::Ident(name)] if name.eq_ignore_ascii_case("currentcolor") => {
            Some(ColorValue::CurrentColor)
        }
        [Token::Ident(name)] if name.eq_ignore_ascii_case("transparent") => {
            Some(ColorValue::Rgba(Color::TRANSPARENT))
        }
        [Token::Ident(name)] => named_color(&name.to_ascii_lowercase()).map(ColorValue::Rgba),
        [Token::Hash { value, .. }] => hex_color(value).map(ColorValue::Rgba),
        [Token::Function(name), ..] if tokens.last() == Some(&Token::CloseParen) => {
            let name = name.to_ascii_lowercase();
            let parts: Vec<&Token> = tokens[1..tokens.len() - 1]
                .iter()
                .filter(|t| !matches!(t, Token::Whitespace | Token::Comma | Token::Delim('/')))
                .collect();
            if parts.len() != 3 && parts.len() != 4 {
                return None;
            }
            let alpha = if parts.len() == 4 {
                channel(parts[3], 1.0)?
            } else {
                1.0
            };
            let mut color = match name.as_str() {
                "rgb" | "rgba" => Color::rgb(
                    to_byte(channel(parts[0], 255.0)? / 255.0),
                    to_byte(channel(parts[1], 255.0)? / 255.0),
                    to_byte(channel(parts[2], 255.0)? / 255.0),
                ),
                "hsl" | "hsla" => {
                    let hue = match parts[0] {
                        Token::Number(n) => *n,
                        Token::Dimension(n, unit) => match unit.to_ascii_lowercase().as_str() {
                            "deg" => *n,
                            "turn" => *n * 360.0,
                            "rad" => *n * 180.0 / std::f64::consts::PI,
                            "grad" => *n * 0.9,
                            _ => return None,
                        },
                        _ => return None,
                    };
                    let (Token::Percentage(sat), Token::Percentage(light)) = (parts[1], parts[2])
                    else {
                        return None;
                    };
                    hsl(hue, *sat / 100.0, *light / 100.0)
                }
                _ => return None,
            };
            color.a = to_byte(alpha);
            Some(ColorValue::Rgba(color))
        }
        _ => None,
    }
}

fn channel(token: &Token, scale: f64) -> Option<f64> {
    match token {
        Token::Number(n) if n.is_finite() => Some(n.clamp(0.0, scale)),
        Token::Percentage(p) if p.is_finite() => Some(p.clamp(0.0, 100.0) * scale / 100.0),
        _ => None,
    }
}
fn to_byte(n: f64) -> u8 {
    (n.clamp(0.0, 1.0) * 255.0).round() as u8
}
fn hsl(h: f64, s: f64, l: f64) -> Color {
    let h = h.rem_euclid(360.0) / 60.0;
    let s = s.clamp(0.0, 1.0);
    let l = l.clamp(0.0, 1.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u8 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Color::rgb(to_byte(r + m), to_byte(g + m), to_byte(b + m))
}

fn hex_color(text: &str) -> Option<Color> {
    if !text.is_ascii() || !matches!(text.len(), 3 | 4 | 6 | 8) {
        return None;
    }
    let full = if text.len() < 5 {
        text.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        text.to_owned()
    };
    let n = u32::from_str_radix(&full, 16).ok()?;
    if full.len() == 6 {
        Some(Color::rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
    } else {
        Some(Color {
            r: (n >> 24) as u8,
            g: (n >> 16) as u8,
            b: (n >> 8) as u8,
            a: n as u8,
        })
    }
}

fn named_color(name: &str) -> Option<Color> {
    // CSS named colors (case folding happens at the caller).
    let n = match name {
        "aliceblue" => 0xf0f8ff,
        "antiquewhite" => 0xfaebd7,
        "aqua" | "cyan" => 0x00ffff,
        "aquamarine" => 0x7fffd4,
        "azure" => 0xf0ffff,
        "beige" => 0xf5f5dc,
        "bisque" => 0xffe4c4,
        "black" => 0x000000,
        "blanchedalmond" => 0xffebcd,
        "blue" => 0x0000ff,
        "blueviolet" => 0x8a2be2,
        "brown" => 0xa52a2a,
        "burlywood" => 0xdeb887,
        "cadetblue" => 0x5f9ea0,
        "chartreuse" => 0x7fff00,
        "chocolate" => 0xd2691e,
        "coral" => 0xff7f50,
        "cornflowerblue" => 0x6495ed,
        "cornsilk" => 0xfff8dc,
        "crimson" => 0xdc143c,
        "darkblue" => 0x00008b,
        "darkcyan" => 0x008b8b,
        "darkgoldenrod" => 0xb8860b,
        "darkgray" | "darkgrey" => 0xa9a9a9,
        "darkgreen" => 0x006400,
        "darkkhaki" => 0xbdb76b,
        "darkmagenta" => 0x8b008b,
        "darkolivegreen" => 0x556b2f,
        "darkorange" => 0xff8c00,
        "darkorchid" => 0x9932cc,
        "darkred" => 0x8b0000,
        "darksalmon" => 0xe9967a,
        "darkseagreen" => 0x8fbc8f,
        "darkslateblue" => 0x483d8b,
        "darkslategray" | "darkslategrey" => 0x2f4f4f,
        "darkturquoise" => 0x00ced1,
        "darkviolet" => 0x9400d3,
        "deeppink" => 0xff1493,
        "deepskyblue" => 0x00bfff,
        "dimgray" | "dimgrey" => 0x696969,
        "dodgerblue" => 0x1e90ff,
        "firebrick" => 0xb22222,
        "floralwhite" => 0xfffaf0,
        "forestgreen" => 0x228b22,
        "fuchsia" | "magenta" => 0xff00ff,
        "gainsboro" => 0xdcdcdc,
        "ghostwhite" => 0xf8f8ff,
        "gold" => 0xffd700,
        "goldenrod" => 0xdaa520,
        "gray" | "grey" => 0x808080,
        "green" => 0x008000,
        "greenyellow" => 0xadff2f,
        "honeydew" => 0xf0fff0,
        "hotpink" => 0xff69b4,
        "indianred" => 0xcd5c5c,
        "indigo" => 0x4b0082,
        "ivory" => 0xfffff0,
        "khaki" => 0xf0e68c,
        "lavender" => 0xe6e6fa,
        "lavenderblush" => 0xfff0f5,
        "lawngreen" => 0x7cfc00,
        "lemonchiffon" => 0xfffacd,
        "lightblue" => 0xadd8e6,
        "lightcoral" => 0xf08080,
        "lightcyan" => 0xe0ffff,
        "lightgoldenrodyellow" => 0xfafad2,
        "lightgray" | "lightgrey" => 0xd3d3d3,
        "lightgreen" => 0x90ee90,
        "lightpink" => 0xffb6c1,
        "lightsalmon" => 0xffa07a,
        "lightseagreen" => 0x20b2aa,
        "lightskyblue" => 0x87cefa,
        "lightslategray" | "lightslategrey" => 0x778899,
        "lightsteelblue" => 0xb0c4de,
        "lightyellow" => 0xffffe0,
        "lime" => 0x00ff00,
        "limegreen" => 0x32cd32,
        "linen" => 0xfaf0e6,
        "maroon" => 0x800000,
        "mediumaquamarine" => 0x66cdaa,
        "mediumblue" => 0x0000cd,
        "mediumorchid" => 0xba55d3,
        "mediumpurple" => 0x9370db,
        "mediumseagreen" => 0x3cb371,
        "mediumslateblue" => 0x7b68ee,
        "mediumspringgreen" => 0x00fa9a,
        "mediumturquoise" => 0x48d1cc,
        "mediumvioletred" => 0xc71585,
        "midnightblue" => 0x191970,
        "mintcream" => 0xf5fffa,
        "mistyrose" => 0xffe4e1,
        "moccasin" => 0xffe4b5,
        "navajowhite" => 0xffdead,
        "navy" => 0x000080,
        "oldlace" => 0xfdf5e6,
        "olive" => 0x808000,
        "olivedrab" => 0x6b8e23,
        "orange" => 0xffa500,
        "orangered" => 0xff4500,
        "orchid" => 0xda70d6,
        "palegoldenrod" => 0xeee8aa,
        "palegreen" => 0x98fb98,
        "paleturquoise" => 0xafeeee,
        "palevioletred" => 0xdb7093,
        "papayawhip" => 0xffefd5,
        "peachpuff" => 0xffdab9,
        "peru" => 0xcd853f,
        "pink" => 0xffc0cb,
        "plum" => 0xdda0dd,
        "powderblue" => 0xb0e0e6,
        "purple" => 0x800080,
        "rebeccapurple" => 0x663399,
        "red" => 0xff0000,
        "rosybrown" => 0xbc8f8f,
        "royalblue" => 0x4169e1,
        "saddlebrown" => 0x8b4513,
        "salmon" => 0xfa8072,
        "sandybrown" => 0xf4a460,
        "seagreen" => 0x2e8b57,
        "seashell" => 0xfff5ee,
        "sienna" => 0xa0522d,
        "silver" => 0xc0c0c0,
        "skyblue" => 0x87ceeb,
        "slateblue" => 0x6a5acd,
        "slategray" | "slategrey" => 0x708090,
        "snow" => 0xfffafa,
        "springgreen" => 0x00ff7f,
        "steelblue" => 0x4682b4,
        "tan" => 0xd2b48c,
        "teal" => 0x008080,
        "thistle" => 0xd8bfd8,
        "tomato" => 0xff6347,
        "turquoise" => 0x40e0d0,
        "violet" => 0xee82ee,
        "wheat" => 0xf5deb3,
        "white" => 0xffffff,
        "whitesmoke" => 0xf5f5f5,
        "yellow" => 0xffff00,
        "yellowgreen" => 0x9acd32,
        _ => return None,
    };
    Some(Color::rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

#[derive(Debug, Clone, Copy)]
pub struct LengthContext {
    pub font_size: f32,
    pub root_font_size: f32,
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub percentage_basis: f32,
}
impl Default for LengthContext {
    fn default() -> Self {
        Self {
            font_size: 16.0,
            root_font_size: 16.0,
            viewport_width: 800.0,
            viewport_height: 600.0,
            percentage_basis: 100.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Length(pub Vec<Token>);
impl Length {
    pub fn px(n: f32) -> Self {
        Self(vec![Token::Dimension(n as f64, "px".into())])
    }
    pub fn parse(tokens: &[Token]) -> Option<Self> {
        let tokens = trim(tokens);
        let value = Self(tokens.to_vec());
        value.resolve(LengthContext::default()).map(|_| value)
    }
    pub fn resolve(&self, ctx: LengthContext) -> Option<f32> {
        let v = expression(&self.0, ctx, 0)?;
        if !v.length && v.n != 0.0 {
            return None;
        }
        if !v.n.is_finite() {
            return None;
        }
        Some(v.n.clamp(-10_000_000.0, 10_000_000.0) as f32)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SizeValue {
    Auto,
    None,
    Length(Length),
}
impl SizeValue {
    pub fn resolve(&self, ctx: LengthContext) -> Option<f32> {
        if let Self::Length(l) = self {
            l.resolve(ctx)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy)]
struct MathValue {
    n: f64,
    length: bool,
}
fn expression(tokens: &[Token], ctx: LengthContext, depth: usize) -> Option<MathValue> {
    if depth > 32 || tokens.len() > 1024 {
        return None;
    }
    let tokens: Vec<&Token> = tokens.iter().filter(|t| **t != Token::Whitespace).collect();
    let mut cursor = 0;
    let value = math_sum(&tokens, &mut cursor, ctx, depth)?;
    if cursor == tokens.len() {
        Some(value)
    } else {
        None
    }
}
fn math_sum(
    tokens: &[&Token],
    cursor: &mut usize,
    ctx: LengthContext,
    depth: usize,
) -> Option<MathValue> {
    let mut lhs = math_product(tokens, cursor, ctx, depth)?;
    while matches!(tokens.get(*cursor), Some(Token::Delim('+' | '-'))) {
        let subtract = tokens[*cursor] == &Token::Delim('-');
        *cursor += 1;
        let rhs = math_product(tokens, cursor, ctx, depth)?;
        if lhs.length != rhs.length {
            return None;
        }
        lhs.n += if subtract { -rhs.n } else { rhs.n };
    }
    Some(lhs)
}
fn math_product(
    tokens: &[&Token],
    cursor: &mut usize,
    ctx: LengthContext,
    depth: usize,
) -> Option<MathValue> {
    let mut lhs = math_atom(tokens, cursor, ctx, depth)?;
    while matches!(tokens.get(*cursor), Some(Token::Delim('*' | '/'))) {
        let divide = tokens[*cursor] == &Token::Delim('/');
        *cursor += 1;
        let rhs = math_atom(tokens, cursor, ctx, depth)?;
        if divide {
            if rhs.length || rhs.n == 0.0 {
                return None;
            }
            lhs.n /= rhs.n;
        } else {
            if lhs.length && rhs.length {
                return None;
            }
            lhs.n *= rhs.n;
            lhs.length |= rhs.length;
        }
    }
    Some(lhs)
}
fn math_atom(
    tokens: &[&Token],
    cursor: &mut usize,
    ctx: LengthContext,
    depth: usize,
) -> Option<MathValue> {
    let token = *tokens.get(*cursor)?;
    *cursor += 1;
    let (n, length) = match token {
        Token::Number(n) => (*n, false),
        Token::Percentage(p) => (*p * ctx.percentage_basis as f64 / 100.0, true),
        Token::Dimension(n, unit) => (*n * unit_scale(unit, ctx)?, true),
        Token::OpenParen => {
            if depth > 32 {
                return None;
            }
            let value = math_sum(tokens, cursor, ctx, depth + 1)?;
            if tokens.get(*cursor) != Some(&&Token::CloseParen) {
                return None;
            }
            *cursor += 1;
            return Some(value);
        }
        Token::Function(name) => {
            let start = *cursor;
            let mut level = 1;
            while *cursor < tokens.len() {
                match tokens[*cursor] {
                    Token::Function(_) | Token::OpenParen => level += 1,
                    Token::CloseParen => level -= 1,
                    _ => {}
                }
                if level == 0 {
                    break;
                }
                *cursor += 1;
            }
            if level != 0 {
                return None;
            }
            let body: Vec<Token> = tokens[start..*cursor]
                .iter()
                .map(|t| (*t).clone())
                .collect();
            *cursor += 1;
            let name = name.to_ascii_lowercase();
            let mut values = Vec::new();
            for part in split_top_level(&body, &Token::Comma) {
                values.push(expression(part, ctx, depth + 1)?);
            }
            if values.is_empty() || values.iter().any(|v| v.length != values[0].length) {
                return None;
            }
            let n = match name.as_str() {
                "calc" if values.len() == 1 => values[0].n,
                "min" => values.iter().map(|v| v.n).fold(f64::INFINITY, f64::min),
                "max" => values.iter().map(|v| v.n).fold(f64::NEG_INFINITY, f64::max),
                "clamp" if values.len() == 3 => values[0].n.max(values[1].n.min(values[2].n)),
                _ => return None,
            };
            return Some(MathValue {
                n,
                length: values[0].length,
            });
        }
        _ => return None,
    };
    Some(MathValue { n, length })
}
fn unit_scale(unit: &str, ctx: LengthContext) -> Option<f64> {
    Some(match unit.to_ascii_lowercase().as_str() {
        "px" => 1.0,
        "em" => ctx.font_size as f64,
        "rem" => ctx.root_font_size as f64,
        "vw" => ctx.viewport_width as f64 / 100.0,
        "vh" => ctx.viewport_height as f64 / 100.0,
        "vmin" => ctx.viewport_width.min(ctx.viewport_height) as f64 / 100.0,
        "vmax" => ctx.viewport_width.max(ctx.viewport_height) as f64 / 100.0,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        "in" => 96.0,
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        _ => return None,
    })
}

pub(crate) fn keyword(tokens: &[Token]) -> Option<String> {
    if let [Token::Ident(s)] = trim(tokens) {
        Some(s.to_ascii_lowercase())
    } else {
        None
    }
}
pub(crate) fn wide_keyword(tokens: &[Token]) -> Option<String> {
    keyword(tokens).filter(|s| {
        matches!(
            s.as_str(),
            "initial" | "inherit" | "unset" | "revert" | "revert-layer"
        )
    })
}

/// Whitespace-separated components, keeping functions/blocks whole.
pub(crate) fn components(tokens: &[Token]) -> Vec<&[Token]> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if tokens[index] == Token::Whitespace {
            index += 1;
            continue;
        }
        let start = index;
        if matches!(
            tokens[index],
            Token::Function(_) | Token::OpenParen | Token::OpenSquare
        ) {
            index = block_end(tokens, index).0;
        } else {
            index += 1;
        }
        out.push(&tokens[start..index]);
    }
    out
}

pub fn font_families(tokens: &[Token]) -> Option<Vec<String>> {
    let mut families = Vec::new();
    for part in split_top_level(tokens, &Token::Comma) {
        let part = trim(part);
        let family = match part {
            [Token::QuotedString(s)] => s.clone(),
            _ => {
                let mut words = Vec::new();
                for token in part {
                    match token {
                        Token::Ident(s) => words.push(s.as_str()),
                        Token::Whitespace => {}
                        _ => return None,
                    }
                }
                words.join(" ")
            }
        };
        if family.is_empty() {
            return None;
        }
        families.push(family);
    }
    Some(families)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::tokenize_all;
    #[test]
    fn color_models_match_and_alpha_clamps() {
        let red = ColorValue::Rgba(Color::rgb(255, 0, 0));
        for value in [
            "red",
            "#f00",
            "#ff0000",
            "rgb(100%, 0%, 0%)",
            "hsl(0 100% 50%)",
        ] {
            assert_eq!(parse_color(&tokenize_all(value)), Some(red), "{value}");
        }
        assert_eq!(
            parse_color(&tokenize_all("#1234")),
            Some(ColorValue::Rgba(Color {
                r: 17,
                g: 34,
                b: 51,
                a: 68
            }))
        );
        assert_eq!(
            parse_color(&tokenize_all("rebeccapurple")),
            Some(ColorValue::Rgba(Color::rgb(102, 51, 153)))
        );
        assert!(parse_color(&tokenize_all("rgb(garbage)")).is_none());
    }
    #[test]
    fn lengths_and_dimension_checked_math() {
        let ctx = LengthContext {
            font_size: 20.0,
            root_font_size: 16.0,
            percentage_basis: 300.0,
            ..LengthContext::default()
        };
        let value = Length::parse(&tokenize_all("calc(50% - 2em + 1rem)")).unwrap();
        assert_eq!(value.resolve(ctx), Some(126.0));
        assert_eq!(
            Length::parse(&tokenize_all("clamp(20px, 50vw, 100px)"))
                .unwrap()
                .resolve(ctx),
            Some(100.0)
        );
        for value in ["calc(10px * 10px)", "calc(10px / 0)", "2", "garbage"] {
            assert!(Length::parse(&tokenize_all(value)).is_none(), "{value}");
        }
    }
}
