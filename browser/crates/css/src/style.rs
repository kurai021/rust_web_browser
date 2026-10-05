//! Computed styles and property grammars (plan/06 §6.2/§6.3).
//! The CSSOM-facing map and typed fields share one computed-value source.

use crate::conditions::Environment;
use crate::stylesheet::{resource_url, url_value};
use crate::syntax::{trim, Declaration};
use crate::token::{serialize, tokenize_all, Token};
use crate::values::{
    components, font_families, keyword, parse_color, wide_keyword, Color, Length, SizeValue,
};
use std::collections::BTreeMap;
use url::Url;

pub type Properties = BTreeMap<String, Vec<Token>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    None,
    Inline,
    Block,
    InlineBlock,
    ListItem,
    Table,
    Flex,
    Grid,
    Contents,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Right,
    Center,
    Justify,
    Start,
    End,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhiteSpace {
    Normal,
    Pre,
    PreWrap,
    PreLine,
    NoWrap,
    BreakSpaces,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineHeight {
    Normal,
    Number(f32),
    Pixels(f32),
}
impl LineHeight {
    pub fn pixels(self, font_size: f32) -> f32 {
        match self {
            Self::Normal => font_size * 1.2,
            Self::Number(n) => n * font_size,
            Self::Pixels(n) => n,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundImage {
    Url(Url),
    LinearGradient(Vec<Color>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    pub color: Color,
    pub background_color: Color,
    pub background_image: Option<BackgroundImage>,
    pub font_size: f32,
    pub font_family: Vec<String>,
    pub font_weight: u16,
    pub italic: bool,
    pub line_height: LineHeight,
    pub display: Display,
    pub visible: bool,
    pub opacity: f32,
    pub text_align: TextAlign,
    pub white_space: WhiteSpace,
    pub text_transform: String,
    pub text_decoration: String,
    pub margin: [SizeValue; 4],
    pub padding: [Length; 4],
    pub border_width: [Length; 4],
    pub border_color: [Color; 4],
    pub border_style: [String; 4],
    pub width: SizeValue,
    pub height: SizeValue,
    pub min_width: SizeValue,
    pub max_width: SizeValue,
    pub min_height: SizeValue,
    pub max_height: SizeValue,
    pub border_box: bool,
    pub content: String,
    pub custom_properties: Properties,
    pub(crate) properties: Properties,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self::from_properties(
            initial_properties(),
            None,
            16.0,
            Environment::default(),
            None,
            Properties::new(),
        )
    }
}

impl ComputedStyle {
    pub fn get_property_value(&self, name: &str) -> String {
        if name.starts_with("--") {
            return self
                .custom_properties
                .get(name)
                .map(|t| serialize(t))
                .unwrap_or_default();
        }
        self.properties
            .get(&name.to_ascii_lowercase())
            .map(|v| serialize(v))
            .unwrap_or_default()
    }

    pub(crate) fn from_properties(
        mut properties: Properties,
        parent: Option<&Self>,
        root_font_size: f32,
        env: Environment,
        image_base: Option<&Url>,
        custom_properties: Properties,
    ) -> Self {
        let parent_size = parent.map_or(16.0, |s| s.font_size);
        let font_size = properties
            .get("font-size")
            .and_then(|value| font_size(value, parent_size, root_font_size, env))
            .unwrap_or(parent_size)
            .clamp(0.0, 10_000.0);
        let font_ctx = env.length_context(
            font_size,
            if parent.is_none() {
                font_size
            } else {
                root_font_size
            },
            0.0,
        );
        // Canonicalize relative lengths, retaining percentages for layout.
        for (name, value) in &mut properties {
            if name != "font-size" && name != "line-height" {
                for token in value {
                    if let Token::Dimension(n, unit) = token {
                        if let Some(px) = Length::parse(&[Token::Dimension(*n, unit.clone())])
                            .and_then(|l| l.resolve(font_ctx))
                        {
                            *token = Token::Dimension(px as f64, "px".into());
                        }
                    }
                }
            }
        }
        let get = |name: &str| properties.get(name).map(Vec::as_slice).unwrap_or(&[]);
        let kw = |name: &str| keyword(get(name)).unwrap_or_default();
        let parent_color = parent.map_or(Color::BLACK, |s| s.color);
        let color = parse_color(get("color"))
            .map(|v| v.resolve(parent_color))
            .unwrap_or(parent_color);
        let background_color = parse_color(get("background-color"))
            .map(|v| v.resolve(color))
            .unwrap_or(Color::TRANSPARENT);
        let font_family = font_families(get("font-family")).unwrap_or_else(|| vec!["serif".into()]);
        let font_weight =
            font_weight(get("font-weight"), parent.map_or(400, |s| s.font_weight)).unwrap_or(400);
        let italic = matches!(kw("font-style").as_str(), "italic" | "oblique");
        let line_height = line_height(get("line-height"), font_size, root_font_size, env)
            .unwrap_or(LineHeight::Normal);
        let display = match kw("display").as_str() {
            "none" => Display::None,
            "block" => Display::Block,
            "inline-block" => Display::InlineBlock,
            "list-item" => Display::ListItem,
            "table" | "table-row" | "table-cell" | "table-row-group" | "table-header-group"
            | "table-footer-group" | "table-caption" | "table-column" | "table-column-group" => {
                Display::Table
            }
            "flex" | "inline-flex" => Display::Flex,
            "grid" | "inline-grid" => Display::Grid,
            "contents" => Display::Contents,
            _ => Display::Inline,
        };
        let margin = std::array::from_fn(|i| {
            size_value(get(&format!("margin-{}", SIDES[i])))
                .unwrap_or(SizeValue::Length(Length::px(0.0)))
        });
        let padding = std::array::from_fn(|i| {
            Length::parse(get(&format!("padding-{}", SIDES[i]))).unwrap_or(Length::px(0.0))
        });
        let border_style: [String; 4] =
            std::array::from_fn(|i| kw(&format!("border-{}-style", SIDES[i])));
        let border_width = std::array::from_fn(|i| {
            if matches!(border_style[i].as_str(), "none" | "hidden") {
                Length::px(0.0)
            } else {
                border_length(get(&format!("border-{}-width", SIDES[i]))).unwrap_or(Length::px(3.0))
            }
        });
        let border_color = std::array::from_fn(|i| {
            parse_color(get(&format!("border-{}-color", SIDES[i])))
                .map(|v| v.resolve(color))
                .unwrap_or(color)
        });
        let width = size_value(get("width")).unwrap_or(SizeValue::Auto);
        let height = size_value(get("height")).unwrap_or(SizeValue::Auto);
        let min_width = size_value(get("min-width")).unwrap_or(SizeValue::Auto);
        let max_width = size_value(get("max-width")).unwrap_or(SizeValue::None);
        let min_height = size_value(get("min-height")).unwrap_or(SizeValue::Auto);
        let max_height = size_value(get("max-height")).unwrap_or(SizeValue::None);
        let visible = kw("visibility") == "visible";
        let opacity = match trim(get("opacity")) {
            [Token::Number(n)] => n.clamp(0.0, 1.0) as f32,
            [Token::Percentage(n)] => (*n / 100.0).clamp(0.0, 1.0) as f32,
            _ => 1.0,
        };
        let text_align = match kw("text-align").as_str() {
            "right" => TextAlign::Right,
            "center" => TextAlign::Center,
            "justify" => TextAlign::Justify,
            "end" => TextAlign::End,
            "start" => TextAlign::Start,
            _ => TextAlign::Left,
        };
        let white_space = match kw("white-space").as_str() {
            "pre" => WhiteSpace::Pre,
            "pre-wrap" => WhiteSpace::PreWrap,
            "pre-line" => WhiteSpace::PreLine,
            "nowrap" => WhiteSpace::NoWrap,
            "break-spaces" => WhiteSpace::BreakSpaces,
            _ => WhiteSpace::Normal,
        };
        let text_transform = kw("text-transform");
        let text_decoration = serialize(get("text-decoration-line"));
        let border_box = kw("box-sizing") == "border-box";
        let content = get("content")
            .iter()
            .filter_map(|t| {
                if let Token::QuotedString(s) = t {
                    Some(s.as_str())
                } else {
                    None
                }
            })
            .collect::<String>();
        let background_image = background_image(get("background-image"), image_base, color);
        properties.insert(
            "font-size".into(),
            vec![Token::Dimension(font_size as f64, "px".into())],
        );
        properties.insert(
            "font-weight".into(),
            vec![Token::Number(font_weight as f64)],
        );
        properties.insert("color".into(), tokenize_all(&color.css()));
        properties.insert(
            "background-color".into(),
            tokenize_all(&background_color.css()),
        );
        properties.insert(
            "line-height".into(),
            match line_height {
                LineHeight::Normal => vec![Token::Ident("normal".into())],
                LineHeight::Number(n) => vec![Token::Number(n as f64)],
                LineHeight::Pixels(n) => vec![Token::Dimension(n as f64, "px".into())],
            },
        );
        Self {
            color,
            background_color,
            background_image,
            font_size,
            font_family,
            font_weight,
            italic,
            line_height,
            display,
            visible,
            opacity,
            text_align,
            white_space,
            text_transform,
            text_decoration,
            margin,
            padding,
            border_width,
            border_color,
            border_style,
            width,
            height,
            min_width,
            max_width,
            min_height,
            max_height,
            border_box,
            content,
            custom_properties,
            properties,
        }
    }
}

pub const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];
const INHERITED: &[&str] = &[
    "color",
    "font-family",
    "font-size",
    "font-style",
    "font-weight",
    "line-height",
    "text-align",
    "white-space",
    "text-transform",
    "visibility",
    "direction",
];
pub(crate) fn inherited(name: &str) -> bool {
    INHERITED.contains(&name)
}

pub(crate) fn initial_properties() -> Properties {
    initial_table().clone()
}

fn initial_table() -> &'static Properties {
    static INITIAL: std::sync::OnceLock<Properties> = std::sync::OnceLock::new();
    INITIAL.get_or_init(build_initial_properties)
}

fn build_initial_properties() -> Properties {
    let mut map = Properties::new();
    for (name, value) in [
        ("color", "black"),
        ("background-color", "transparent"),
        ("background-image", "none"),
        ("font-family", "serif"),
        ("font-size", "16px"),
        ("font-weight", "normal"),
        ("font-style", "normal"),
        ("line-height", "normal"),
        ("display", "inline"),
        ("visibility", "visible"),
        ("opacity", "1"),
        ("text-align", "start"),
        ("white-space", "normal"),
        ("text-transform", "none"),
        ("text-decoration-line", "none"),
        ("text-decoration-color", "currentColor"),
        ("width", "auto"),
        ("height", "auto"),
        ("min-width", "auto"),
        ("min-height", "auto"),
        ("max-width", "none"),
        ("max-height", "none"),
        ("box-sizing", "content-box"),
        ("aspect-ratio", "auto"),
        ("overflow", "visible"),
        ("position", "static"),
        ("top", "auto"),
        ("right", "auto"),
        ("bottom", "auto"),
        ("left", "auto"),
        ("float", "none"),
        ("clear", "none"),
        ("z-index", "auto"),
        ("border-radius", "0px"),
        ("content", "normal"),
        ("direction", "ltr"),
    ] {
        map.insert(name.into(), tokenize_all(value));
    }
    for side in SIDES {
        for (prefix, value) in [("margin", "0px"), ("padding", "0px")] {
            map.insert(format!("{prefix}-{side}"), tokenize_all(value));
        }
        for (kind, value) in [
            ("width", "medium"),
            ("style", "none"),
            ("color", "currentColor"),
        ] {
            map.insert(format!("border-{side}-{kind}"), tokenize_all(value));
        }
    }
    map
}

pub fn supported_property(name: &str, tokens: &[Token]) -> bool {
    if contains_var(tokens) {
        return !affected_properties(name).is_empty();
    }
    expand_property(name, tokens).is_some()
}
pub(crate) fn contains_var(tokens: &[Token]) -> bool {
    tokens
        .iter()
        .any(|t| matches!(t,Token::Function(s) if s.eq_ignore_ascii_case("var")))
}

pub(crate) fn affected_properties(name: &str) -> Vec<String> {
    if name.starts_with("--") {
        return vec![name.to_owned()];
    }
    match name {
        "margin" | "padding" => SIDES.iter().map(|side| format!("{name}-{side}")).collect(),
        "border-width" | "border-color" | "border-style" => SIDES
            .iter()
            .map(|side| format!("border-{side}-{}", &name[7..]))
            .collect(),
        "border" => SIDES
            .iter()
            .flat_map(|side| {
                ["width", "style", "color"].map(|kind| format!("border-{side}-{kind}"))
            })
            .collect(),
        "border-top" | "border-right" | "border-bottom" | "border-left" => {
            ["width", "style", "color"]
                .iter()
                .map(|kind| format!("{name}-{kind}"))
                .collect()
        }
        "font" => [
            "font-size",
            "font-family",
            "font-style",
            "font-weight",
            "line-height",
        ]
        .iter()
        .map(|s| (*s).into())
        .collect(),
        "background" => vec!["background-color".into(), "background-image".into()],
        "text-decoration" => vec![
            "text-decoration-line".into(),
            "text-decoration-color".into(),
        ],
        _ if initial_table().contains_key(name) => vec![name.to_owned()],
        _ => Vec::new(),
    }
}

/// Expand shorthand declarations atomically. Invalid declarations do not
/// enter the cascade. Deferred var() shorthands use this after substitution.
pub(crate) fn expand_property(name: &str, tokens: &[Token]) -> Option<Vec<(String, Vec<Token>)>> {
    let keys = affected_properties(name);
    if keys.is_empty() {
        return None;
    }
    if name.starts_with("--") || wide_keyword(tokens).is_some() {
        return Some(keys.into_iter().map(|key| (key, tokens.to_vec())).collect());
    }
    let tokens = trim(tokens);
    if keys.len() == 1 {
        return valid_longhand(name, tokens).then(|| vec![(name.into(), tokens.to_vec())]);
    }
    let parts = components(tokens);
    if matches!(
        name,
        "margin" | "padding" | "border-width" | "border-style" | "border-color"
    ) {
        if !(1..=4).contains(&parts.len()) {
            return None;
        }
        let indices = match parts.len() {
            1 => [0, 0, 0, 0],
            2 => [0, 1, 0, 1],
            3 => [0, 1, 2, 1],
            _ => [0, 1, 2, 3],
        };
        let mut out = Vec::new();
        for (key, index) in keys.into_iter().zip(indices) {
            if !valid_longhand(&key, parts[index]) {
                return None;
            }
            out.push((key, parts[index].to_vec()));
        }
        return Some(out);
    }
    if name == "border" || name.starts_with("border-") {
        let mut width = tokenize_all("medium");
        let mut style = tokenize_all("none");
        let mut color = tokenize_all("currentColor");
        let mut seen = [false; 3];
        for part in parts {
            if border_length(part).is_some() && !seen[0] {
                width = part.to_vec();
                seen[0] = true;
            } else if border_style(part) && !seen[1] {
                style = part.to_vec();
                seen[1] = true;
            } else if parse_color(part).is_some() && !seen[2] {
                color = part.to_vec();
                seen[2] = true;
            } else {
                return None;
            }
        }
        return Some(
            keys.into_iter()
                .map(|key| {
                    let value = if key.ends_with("width") {
                        width.clone()
                    } else if key.ends_with("style") {
                        style.clone()
                    } else {
                        color.clone()
                    };
                    (key, value)
                })
                .collect(),
        );
    }
    if name == "font" {
        let mut weight = tokenize_all("normal");
        let mut style = tokenize_all("normal");
        let mut size = None;
        let mut line = tokenize_all("normal");
        let mut index = 0;
        while index < parts.len() {
            if font_size(parts[index], 16.0, 16.0, Environment::default()).is_some() {
                size = Some(parts[index].to_vec());
                index += 1;
                break;
            }
            if matches!(
                keyword(parts[index]).as_deref(),
                Some("normal" | "italic" | "oblique")
            ) {
                style = parts[index].to_vec();
            } else if font_weight(parts[index], 400).is_some() {
                weight = parts[index].to_vec();
            } else {
                return None;
            }
            index += 1;
        }
        let size = size?;
        if parts.get(index) == Some(&&[Token::Delim('/')][..]) {
            index += 1;
            let part = *parts.get(index)?;
            line_height(part, 16.0, 16.0, Environment::default())?;
            line = part.to_vec();
            index += 1;
        }
        let mut family = Vec::new();
        for part in parts.iter().skip(index) {
            if !family.is_empty() {
                family.push(Token::Whitespace);
            }
            family.extend_from_slice(part);
        }
        font_families(&family)?;
        return Some(vec![
            ("font-size".into(), size),
            ("font-style".into(), style),
            ("font-weight".into(), weight),
            ("line-height".into(), line),
            ("font-family".into(), family),
        ]);
    }
    if name == "background" {
        let mut color = tokenize_all("transparent");
        let mut image = tokenize_all("none");
        for part in parts {
            if parse_color(part).is_some() {
                color = part.to_vec();
            } else if valid_image(part) {
                image = part.to_vec();
            } else if !matches!(
                keyword(part).as_deref(),
                Some(
                    "repeat"
                        | "no-repeat"
                        | "repeat-x"
                        | "repeat-y"
                        | "scroll"
                        | "fixed"
                        | "center"
                        | "left"
                        | "right"
                        | "top"
                        | "bottom"
                        | "cover"
                        | "contain"
                        | "padding-box"
                        | "border-box"
                        | "content-box"
                )
            ) && Length::parse(part).is_none()
                && part != [Token::Delim('/')]
            {
                return None;
            }
        }
        return Some(vec![
            ("background-color".into(), color),
            ("background-image".into(), image),
        ]);
    }
    if name == "text-decoration" {
        let mut lines = Vec::new();
        let mut color = tokenize_all("currentColor");
        for part in parts {
            if parse_color(part).is_some() {
                color = part.to_vec();
            } else if matches!(
                keyword(part).as_deref(),
                Some("none" | "underline" | "overline" | "line-through")
            ) {
                if !lines.is_empty() {
                    lines.push(Token::Whitespace);
                }
                lines.extend_from_slice(part);
            } else {
                return None;
            }
        }
        if lines.is_empty() {
            lines = tokenize_all("none");
        }
        return Some(vec![
            ("text-decoration-line".into(), lines),
            ("text-decoration-color".into(), color),
        ]);
    }
    None
}

fn valid_longhand(name: &str, value: &[Token]) -> bool {
    let kw = keyword(value).unwrap_or_default();
    match name {
        "color" | "background-color" | "text-decoration-color" => parse_color(value).is_some(),
        "background-image" => valid_image(value),
        "font-family" => font_families(value).is_some(),
        "font-size" => font_size(value, 16.0, 16.0, Environment::default()).is_some(),
        "font-weight" => font_weight(value, 400).is_some(),
        "font-style" => matches!(kw.as_str(), "normal" | "italic" | "oblique"),
        "line-height" => line_height(value, 16.0, 16.0, Environment::default()).is_some(),
        "display" => matches!(
            kw.as_str(),
            "none"
                | "inline"
                | "block"
                | "inline-block"
                | "list-item"
                | "contents"
                | "flex"
                | "grid"
                | "inline-flex"
                | "inline-grid"
                | "table"
                | "table-row"
                | "table-cell"
                | "table-row-group"
                | "table-header-group"
                | "table-footer-group"
                | "table-caption"
                | "table-column"
                | "table-column-group"
        ),
        "visibility" => matches!(kw.as_str(), "visible" | "hidden" | "collapse"),
        "opacity" => matches!(value,[Token::Number(n)]|[Token::Percentage(n)] if n.is_finite()),
        "text-align" => matches!(
            kw.as_str(),
            "left" | "right" | "center" | "justify" | "start" | "end"
        ),
        "white-space" => matches!(
            kw.as_str(),
            "normal" | "pre" | "pre-wrap" | "pre-line" | "nowrap" | "break-spaces"
        ),
        "text-transform" => matches!(
            kw.as_str(),
            "none" | "uppercase" | "lowercase" | "capitalize"
        ),
        "text-decoration-line" => {
            !value.is_empty()
                && components(value).iter().all(|p| {
                    matches!(
                        keyword(p).as_deref(),
                        Some("none" | "underline" | "overline" | "line-through")
                    )
                })
        }
        "box-sizing" => matches!(kw.as_str(), "content-box" | "border-box"),
        "aspect-ratio" => {
            if kw == "auto" {
                return true;
            }
            let parts: Vec<_> = value.iter().filter(|t| **t != Token::Whitespace).collect();
            let parts = if matches!(parts.first(), Some(Token::Ident(s)) if s.eq_ignore_ascii_case("auto"))
            {
                &parts[1..]
            } else {
                &parts[..]
            };
            matches!(parts, [Token::Number(n)] if n.is_finite() && *n > 0.0)
                || matches!(parts, [Token::Number(n), Token::Delim('/'), Token::Number(d)] if n.is_finite() && d.is_finite() && *n > 0.0 && *d > 0.0)
        }
        "overflow" => matches!(
            kw.as_str(),
            "visible" | "hidden" | "clip" | "scroll" | "auto"
        ),
        "position" => matches!(
            kw.as_str(),
            "static" | "relative" | "absolute" | "fixed" | "sticky"
        ),
        "float" => matches!(kw.as_str(), "none" | "left" | "right"),
        "clear" => matches!(kw.as_str(), "none" | "left" | "right" | "both"),
        "z-index" => {
            kw == "auto" || matches!(value,[Token::Number(n)] if n.is_finite() && n.fract()==0.0)
        }
        "direction" => matches!(kw.as_str(), "ltr" | "rtl"),
        "content" => {
            matches!(kw.as_str(), "normal" | "none")
                || (!value.is_empty()
                    && value
                        .iter()
                        .all(|t| matches!(t, Token::QuotedString(_) | Token::Whitespace)))
        }
        "border-radius" => components(value).iter().all(|p| Length::parse(p).is_some()),
        "width" | "height" | "min-width" | "min-height" | "max-width" | "max-height" => {
            size_value(value).is_some()
        }
        "top" | "right" | "bottom" | "left" => size_value(value).is_some(),
        _ if name.starts_with("margin-") => kw == "auto" || Length::parse(value).is_some(),
        _ if name.starts_with("padding-") => Length::parse(value)
            .and_then(|l| l.resolve(Default::default()))
            .is_some_and(|n| n >= 0.0),
        _ if name.starts_with("border-") && name.ends_with("-color") => {
            parse_color(value).is_some()
        }
        _ if name.starts_with("border-") && name.ends_with("-width") => {
            border_length(value).is_some()
        }
        _ if name.starts_with("border-") && name.ends_with("-style") => border_style(value),
        _ => false,
    }
}

fn font_size(tokens: &[Token], parent: f32, root: f32, env: Environment) -> Option<f32> {
    if let Some(kw) = keyword(tokens) {
        return match kw.as_str() {
            "xx-small" => Some(9.0),
            "x-small" => Some(10.0),
            "small" => Some(13.0),
            "medium" => Some(16.0),
            "large" => Some(18.0),
            "x-large" => Some(24.0),
            "xx-large" => Some(32.0),
            "xxx-large" => Some(48.0),
            "smaller" => Some(parent * 0.8),
            "larger" => Some(parent * 1.2),
            _ => None,
        };
    }
    Length::parse(tokens)?
        .resolve(env.length_context(parent, root, parent))
        .filter(|n| *n >= 0.0)
}
fn font_weight(tokens: &[Token], parent: u16) -> Option<u16> {
    match trim(tokens) {
        [Token::Number(n)] if n.is_finite() && (1.0..=1000.0).contains(n) => Some(*n as u16),
        _ => match keyword(tokens)?.as_str() {
            "normal" => Some(400),
            "bold" => Some(700),
            "bolder" => Some(if parent < 350 {
                400
            } else if parent < 550 {
                700
            } else {
                900
            }),
            "lighter" => Some(if parent < 550 {
                100
            } else if parent < 750 {
                400
            } else {
                700
            }),
            _ => None,
        },
    }
}
fn line_height(
    tokens: &[Token],
    font_size: f32,
    root: f32,
    env: Environment,
) -> Option<LineHeight> {
    match trim(tokens) {
        [Token::Ident(kw)] if kw.eq_ignore_ascii_case("normal") => Some(LineHeight::Normal),
        [Token::Number(n)] if n.is_finite() && *n >= 0.0 => Some(LineHeight::Number(*n as f32)),
        _ => Length::parse(tokens)?
            .resolve(env.length_context(font_size, root, font_size))
            .filter(|n| *n >= 0.0)
            .map(LineHeight::Pixels),
    }
}
fn size_value(tokens: &[Token]) -> Option<SizeValue> {
    match keyword(tokens).as_deref() {
        Some("auto") => Some(SizeValue::Auto),
        Some("none") => Some(SizeValue::None),
        _ => Length::parse(tokens).map(SizeValue::Length),
    }
}
fn border_length(tokens: &[Token]) -> Option<Length> {
    match keyword(tokens).as_deref() {
        Some("thin") => Some(Length::px(1.0)),
        Some("medium") => Some(Length::px(3.0)),
        Some("thick") => Some(Length::px(5.0)),
        _ => Length::parse(tokens)
            .filter(|l| l.resolve(Default::default()).is_some_and(|n| n >= 0.0)),
    }
}
fn border_style(tokens: &[Token]) -> bool {
    matches!(
        keyword(tokens).as_deref(),
        Some(
            "none"
                | "hidden"
                | "solid"
                | "dashed"
                | "dotted"
                | "double"
                | "groove"
                | "ridge"
                | "inset"
                | "outset"
        )
    )
}
fn valid_image(tokens: &[Token]) -> bool {
    keyword(tokens).as_deref() == Some("none")
        || url_value(tokens).is_some_and(|(_, n)| n == tokens.len())
        || gradient(tokens, Color::BLACK).is_some()
}
fn gradient(tokens: &[Token], current: Color) -> Option<Vec<Color>> {
    if !matches!(tokens.first(),Some(Token::Function(n)) if n.eq_ignore_ascii_case("linear-gradient"))
        || tokens.last() != Some(&Token::CloseParen)
    {
        return None;
    }
    let parts = crate::syntax::split_top_level(&tokens[1..tokens.len() - 1], &Token::Comma);
    let mut colors = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        let comps = components(part);
        if let Some(color) = comps.first().and_then(|p| parse_color(p)) {
            colors.push(color.resolve(current));
        } else if i != 0 {
            return None;
        }
    }
    (colors.len() >= 2).then_some(colors)
}
fn background_image(
    tokens: &[Token],
    base: Option<&Url>,
    current: Color,
) -> Option<BackgroundImage> {
    if let Some((raw, _)) = url_value(tokens) {
        return resource_url(base, &raw).map(BackgroundImage::Url);
    }
    gradient(tokens, current).map(BackgroundImage::LinearGradient)
}

/// A declaration retaining its original shorthand for deferred var() expansion.
pub(crate) fn declaration_keys(decl: &Declaration) -> Vec<String> {
    if contains_var(&decl.value) {
        affected_properties(&decl.name)
    } else {
        expand_property(&decl.name, &decl.value)
            .map(|expanded| expanded.into_iter().map(|(k, _)| k).collect())
            .unwrap_or_default()
    }
}
