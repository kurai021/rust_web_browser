//! Bounded Stage B/C grammars. Used values are resolved by the layout crate.
use crate::style::Properties;
use crate::syntax::{split_top_level, trim};
use crate::values::{components, keyword, Length, SizeValue};
use crate::Token;

pub const MAX_TRACKS: usize = 128;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Position {
    #[default]
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}
impl Position {
    pub fn out_of_flow(self) -> bool {
        matches!(self, Self::Absolute | Self::Fixed)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Float {
    #[default]
    None,
    Left,
    Right,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Clear {
    #[default]
    None,
    Left,
    Right,
    Both,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Flow,
    Flex,
    Grid,
    Table,
    RowGroup,
    Row,
    Cell,
    Caption,
    Column,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Normal,
    Auto,
    Start,
    End,
    Center,
    Stretch,
    Baseline,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}
pub fn alignment(v: &[Token]) -> Option<Align> {
    Some(match keyword(v)?.as_str() {
        "normal" => Align::Normal,
        "auto" => Align::Auto,
        "start" | "flex-start" | "self-start" | "left" => Align::Start,
        "end" | "flex-end" | "self-end" | "right" => Align::End,
        "center" => Align::Center,
        "stretch" => Align::Stretch,
        "baseline" => Align::Baseline,
        "space-between" => Align::SpaceBetween,
        "space-around" => Align::SpaceAround,
        "space-evenly" => Align::SpaceEvenly,
        _ => return None,
    })
}
#[derive(Debug, Clone, PartialEq)]
pub enum Track {
    Auto,
    Length(Length),
    Fr(f32),
    MinMax(Box<Track>, Box<Track>),
    FitContent(Length),
}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tracks {
    pub values: Vec<Track>,
    /// Insertion point and pattern for repeat(auto-fill/fit, ...).
    pub automatic: Option<(usize, Vec<Track>, bool)>,
}
fn nonnegative_length(v: &[Token]) -> Option<Length> {
    Length::parse(v).filter(|l| l.resolve(Default::default()).is_some_and(|n| n >= 0.0))
}
fn track(v: &[Token]) -> Option<Track> {
    match trim(v) {
        [Token::Ident(s)] if matches!(s.as_str(), "auto" | "min-content" | "max-content") => {
            Some(Track::Auto)
        }
        [Token::Dimension(n, unit)]
            if unit.eq_ignore_ascii_case("fr") && n.is_finite() && *n >= 0.0 =>
        {
            Some(Track::Fr(*n as f32))
        }
        [Token::Function(s), inner @ .., Token::CloseParen] if s.eq_ignore_ascii_case("minmax") => {
            let parts = split_top_level(inner, &Token::Comma);
            if parts.len() != 2 {
                return None;
            }
            let min = track(parts[0])?;
            let max = track(parts[1])?;
            if matches!(min, Track::Fr(_) | Track::MinMax(..) | Track::FitContent(_))
                || matches!(max, Track::MinMax(..) | Track::FitContent(_))
            {
                return None;
            }
            Some(Track::MinMax(Box::new(min), Box::new(max)))
        }
        [Token::Function(s), inner @ .., Token::CloseParen]
            if s.eq_ignore_ascii_case("fit-content") =>
        {
            nonnegative_length(inner).map(Track::FitContent)
        }
        _ => nonnegative_length(v).map(Track::Length),
    }
}
pub fn tracks(v: &[Token]) -> Option<Tracks> {
    if keyword(v).as_deref() == Some("none") {
        return Some(Tracks::default());
    }
    let mut result = Tracks::default();
    for part in components(v) {
        if let [Token::Function(s), inner @ .., Token::CloseParen] = part {
            if s.eq_ignore_ascii_case("repeat") {
                let parts = split_top_level(inner, &Token::Comma);
                if parts.len() != 2 {
                    return None;
                }
                let pattern: Vec<_> = components(parts[1])
                    .into_iter()
                    .map(track)
                    .collect::<Option<_>>()?;
                if pattern.is_empty() || pattern.len() > MAX_TRACKS {
                    return None;
                }
                match trim(parts[0]) {
                    [Token::Number(n)]
                        if n.is_finite()
                            && *n >= 1.0
                            && n.fract() == 0.0
                            && *n <= MAX_TRACKS as f64 =>
                    {
                        if result.values.len() + pattern.len() * *n as usize > MAX_TRACKS {
                            return None;
                        }
                        for _ in 0..*n as usize {
                            result.values.extend(pattern.clone());
                        }
                    }
                    [Token::Ident(s)]
                        if matches!(s.as_str(), "auto-fit" | "auto-fill")
                            && result.automatic.is_none() =>
                    {
                        // Auto repetition needs a positive fixed sizing component.
                        if !pattern
                            .iter()
                            .all(|t| matches!(t, Track::Length(_) | Track::MinMax(_, _)))
                        {
                            return None;
                        }
                        result.automatic = Some((result.values.len(), pattern, s == "auto-fit"));
                    }
                    _ => return None,
                }
                continue;
            }
        }
        result.values.push(track(part)?);
        if result.values.len() > MAX_TRACKS {
            return None;
        }
    }
    (!result.values.is_empty() || result.automatic.is_some()).then_some(result)
}
#[derive(Debug, Clone, PartialEq, Default)]
pub enum GridLine {
    #[default]
    Auto,
    Line(i32),
    Span(usize),
    Name(String),
}
pub fn grid_line(v: &[Token]) -> Option<GridLine> {
    match trim(v) {
        [Token::Ident(s)] if s == "auto" => Some(GridLine::Auto),
        [Token::Ident(s)] if !matches!(s.as_str(), "span" | "none") => {
            Some(GridLine::Name(s.clone()))
        }
        [Token::Number(n)]
            if n.is_finite() && n.fract() == 0.0 && *n != 0.0 && n.abs() <= 4096.0 =>
        {
            Some(GridLine::Line(*n as i32))
        }
        _ => {
            let parts = components(v);
            if parts.len() != 2 {
                return None;
            }
            let number = if keyword(parts[0]).as_deref() == Some("span") {
                parts[1]
            } else if keyword(parts[1]).as_deref() == Some("span") {
                parts[0]
            } else {
                return None;
            };
            match number {
                [Token::Number(n)]
                    if n.is_finite()
                        && *n >= 1.0
                        && *n <= MAX_TRACKS as f64
                        && n.fract() == 0.0 =>
                {
                    Some(GridLine::Span(*n as usize))
                }
                _ => None,
            }
        }
    }
}
pub fn areas(v: &[Token]) -> Option<Vec<Vec<String>>> {
    if keyword(v).as_deref() == Some("none") {
        return Some(Vec::new());
    }
    let mut rows = Vec::new();
    for part in components(v) {
        let [Token::QuotedString(s)] = part else {
            return None;
        };
        let row = s
            .split_ascii_whitespace()
            .map(|name| {
                if name.chars().all(|c| c == '.') {
                    ".".into()
                } else {
                    name.into()
                }
            })
            .collect::<Vec<String>>();
        if row.is_empty()
            || row.len() > MAX_TRACKS
            || rows
                .first()
                .is_some_and(|r: &Vec<String>| r.len() != row.len())
        {
            return None;
        }
        rows.push(row);
        if rows.len() > MAX_TRACKS {
            return None;
        }
    }
    if rows.is_empty() {
        return None;
    }
    let mut rectangles =
        std::collections::HashMap::<&str, (usize, usize, usize, usize, usize)>::new();
    for (r, row) in rows.iter().enumerate() {
        for (c, name) in row.iter().enumerate() {
            if name == "." {
                continue;
            }
            if !name
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_'))
            {
                return None;
            }
            rectangles
                .entry(name)
                .and_modify(|b| {
                    b.0 = b.0.min(r);
                    b.1 = b.1.min(c);
                    b.2 = b.2.max(r);
                    b.3 = b.3.max(c);
                    b.4 += 1;
                })
                .or_insert((r, c, r, c, 1));
        }
    }
    if rectangles
        .values()
        .any(|b| (b.2 - b.0 + 1) * (b.3 - b.1 + 1) != b.4)
    {
        return None;
    }
    Some(rows)
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutStyle {
    pub position: Position,
    pub inset: [SizeValue; 4],
    pub float: Float,
    pub clear: Clear,
    pub z_index: Option<i32>,
    pub mode: Mode,
    pub inline: bool,
    pub order: i32,
    pub column: bool,
    pub reverse: bool,
    pub wrap: bool,
    pub wrap_reverse: bool,
    pub grow: f32,
    pub shrink: f32,
    pub basis: SizeValue,
    pub justify_content: Align,
    pub align_content: Align,
    pub align_items: Align,
    pub align_self: Align,
    pub justify_items: Align,
    pub justify_self: Align,
    pub gap: [Length; 2],
    pub columns: Tracks,
    pub rows: Tracks,
    pub auto_columns: Tracks,
    pub auto_rows: Tracks,
    pub areas: Vec<Vec<String>>,
    pub area: Option<String>,
    pub placement: [GridLine; 4],
    pub auto_column_flow: bool,
    pub dense: bool,
    pub border_collapse: bool,
    pub border_spacing: [Length; 2],
    pub table_fixed: bool,
}
impl LayoutStyle {
    pub(crate) fn from_properties(p: &Properties) -> Self {
        let get = |s: &str| p.get(s).map(Vec::as_slice).unwrap_or(&[]);
        let kw = |s: &str| keyword(get(s)).unwrap_or_default();
        let size = |s: &str| match kw(s).as_str() {
            "auto" | "content" => SizeValue::Auto,
            _ => Length::parse(get(s))
                .map(SizeValue::Length)
                .unwrap_or(SizeValue::Auto),
        };
        let number = |s: &str, default| match get(s) {
            [Token::Number(n)] => *n as f32,
            _ => default,
        };
        let align = |s: &str| alignment(get(s)).unwrap_or_default();
        let display = kw("display");
        let direction = kw("flex-direction");
        let wrap = kw("flex-wrap");
        let spacing = components(get("border-spacing"));
        Self {
            position: match kw("position").as_str() {
                "relative" => Position::Relative,
                "absolute" => Position::Absolute,
                "fixed" => Position::Fixed,
                "sticky" => Position::Sticky,
                _ => Position::Static,
            },
            inset: [size("top"), size("right"), size("bottom"), size("left")],
            float: match kw("float").as_str() {
                "left" => Float::Left,
                "right" => Float::Right,
                _ => Float::None,
            },
            clear: match kw("clear").as_str() {
                "left" => Clear::Left,
                "right" => Clear::Right,
                "both" => Clear::Both,
                _ => Clear::None,
            },
            z_index: match get("z-index") {
                [Token::Number(n)] => Some(*n as i32),
                _ => None,
            },
            mode: match display.as_str() {
                "flex" | "inline-flex" => Mode::Flex,
                "grid" | "inline-grid" => Mode::Grid,
                "table" | "inline-table" => Mode::Table,
                "table-row" => Mode::Row,
                "table-cell" => Mode::Cell,
                "table-caption" => Mode::Caption,
                "table-column" | "table-column-group" => Mode::Column,
                "table-row-group" | "table-header-group" | "table-footer-group" => Mode::RowGroup,
                _ => Mode::Flow,
            },
            inline: display.starts_with("inline-"),
            order: number("order", 0.0) as i32,
            column: direction.starts_with("column"),
            reverse: direction.ends_with("reverse"),
            wrap: wrap != "nowrap",
            wrap_reverse: wrap == "wrap-reverse",
            grow: number("flex-grow", 0.0),
            shrink: number("flex-shrink", 1.0),
            basis: size("flex-basis"),
            justify_content: align("justify-content"),
            align_content: align("align-content"),
            align_items: align("align-items"),
            align_self: align("align-self"),
            justify_items: align("justify-items"),
            justify_self: align("justify-self"),
            gap: [
                Length::parse(get("row-gap")).unwrap_or(Length::px(0.0)),
                Length::parse(get("column-gap")).unwrap_or(Length::px(0.0)),
            ],
            columns: tracks(get("grid-template-columns")).unwrap_or_default(),
            rows: tracks(get("grid-template-rows")).unwrap_or_default(),
            auto_columns: tracks(get("grid-auto-columns")).unwrap_or_default(),
            auto_rows: tracks(get("grid-auto-rows")).unwrap_or_default(),
            areas: areas(get("grid-template-areas")).unwrap_or_default(),
            area: if kw("grid-area-name") == "auto" {
                None
            } else {
                keyword(get("grid-area-name"))
            },
            placement: std::array::from_fn(|i| {
                grid_line(get([
                    "grid-row-start",
                    "grid-column-start",
                    "grid-row-end",
                    "grid-column-end",
                ][i]))
                .unwrap_or_default()
            }),
            auto_column_flow: components(get("grid-auto-flow"))
                .iter()
                .any(|v| keyword(v).as_deref() == Some("column")),
            dense: components(get("grid-auto-flow"))
                .iter()
                .any(|v| keyword(v).as_deref() == Some("dense")),
            border_collapse: kw("border-collapse") == "collapse",
            border_spacing: std::array::from_fn(|i| {
                spacing
                    .get(i)
                    .or_else(|| spacing.first())
                    .and_then(|v| Length::parse(v))
                    .unwrap_or(Length::px(0.0))
            }),
            table_fixed: kw("table-layout") == "fixed",
        }
    }
}
pub(crate) fn valid(name: &str, v: &[Token]) -> Option<bool> {
    let kw = keyword(v).unwrap_or_default();
    Some(match name {
        "flex-direction" => matches!(
            kw.as_str(),
            "row" | "row-reverse" | "column" | "column-reverse"
        ),
        "flex-wrap" => matches!(kw.as_str(), "nowrap" | "wrap" | "wrap-reverse"),
        "flex-grow" | "flex-shrink" => {
            matches!(v,[Token::Number(n)] if n.is_finite() && *n>=0.0 && *n<=1e6)
        }
        "flex-basis" => {
            matches!(kw.as_str(), "auto" | "content") || nonnegative_length(v).is_some()
        }
        "order" => {
            matches!(v,[Token::Number(n)] if n.is_finite() && n.fract()==0.0 && n.abs()<=i32::MAX as f64)
        }
        "row-gap" | "column-gap" => kw == "normal" || nonnegative_length(v).is_some(),
        "align-items" | "align-self" | "justify-items" | "justify-self" => alignment(v)
            .is_some_and(|a| {
                !matches!(
                    a,
                    Align::SpaceBetween | Align::SpaceAround | Align::SpaceEvenly
                )
            }),
        "align-content" | "justify-content" => alignment(v).is_some(),
        "grid-template-columns" | "grid-template-rows" => tracks(v).is_some(),
        "grid-auto-columns" | "grid-auto-rows" => {
            tracks(v).is_some_and(|t| t.automatic.is_none() && !t.values.is_empty())
        }
        "grid-template-areas" => areas(v).is_some(),
        "grid-row-start" | "grid-row-end" | "grid-column-start" | "grid-column-end" => {
            grid_line(v).is_some()
        }
        "grid-area-name" => keyword(v).is_some(),
        "grid-auto-flow" => {
            let parts = components(v);
            !parts.is_empty()
                && parts.len() <= 2
                && parts
                    .iter()
                    .all(|v| matches!(keyword(v).as_deref(), Some("row" | "column" | "dense")))
                && !(parts.iter().any(|v| keyword(v).as_deref() == Some("row"))
                    && parts
                        .iter()
                        .any(|v| keyword(v).as_deref() == Some("column")))
        }
        "border-collapse" => matches!(kw.as_str(), "collapse" | "separate"),
        "border-spacing" => {
            let c = components(v);
            (1..=2).contains(&c.len()) && c.iter().all(|v| nonnegative_length(v).is_some())
        }
        "table-layout" => matches!(kw.as_str(), "auto" | "fixed"),
        "vertical-align" => {
            matches!(kw.as_str(), "baseline" | "top" | "middle" | "bottom")
                || Length::parse(v).is_some()
        }
        _ => return None,
    })
}
