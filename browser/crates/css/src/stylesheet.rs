//! Semantic CSS rules, import URLs and font-face descriptors (plan/06).
//! No network dependency: the shell fills `ImportRule::sheet` after fetch.

use crate::conditions::{MediaQueryList, Supports};
use crate::selectors::{parse_tokens, Selector};
use crate::syntax::{
    declarations_from_tokens, parse_rules, split_top_level, trim, Declaration, RawRule,
    MAX_NESTING, MAX_RULES, MAX_STYLESHEET_BYTES,
};
use crate::token::{serialize, tokenize_all, Token};
use crate::values::{font_families, keyword};
use url::Url;

#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    pub base_url: Option<Url>,
    pub errors: Vec<String>,
}
#[derive(Debug, Clone)]
pub enum Rule {
    Style(StyleRule),
    Media(MediaQueryList, Vec<Rule>),
    Supports(Supports, Vec<Rule>),
    Layer {
        name: Option<String>,
        rules: Vec<Rule>,
    },
    LayerOrder(Vec<String>),
    Import(ImportRule),
    FontFace(FontFace),
    Keyframes {
        name: String,
        frames: Vec<Keyframe>,
    },
}
#[derive(Debug, Clone)]
pub struct StyleRule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
}
#[derive(Debug, Clone)]
pub struct ImportRule {
    pub url: Url,
    pub media: MediaQueryList,
    pub layer: Option<String>,
    pub sheet: Option<Box<Stylesheet>>,
}
#[derive(Debug, Clone)]
pub struct Keyframe {
    pub positions: Vec<f32>,
    pub declarations: Vec<Declaration>,
}
#[derive(Debug, Clone, PartialEq)]
pub enum FontSource {
    Local(String),
    Url { url: Url, format: Option<String> },
}
#[derive(Debug, Clone)]
pub struct FontFace {
    pub family: String,
    pub sources: Vec<FontSource>,
    pub weight: u16,
    pub italic: bool,
    pub display: String,
}

pub fn parse_stylesheet(css: &str, base_url: Option<&Url>) -> Stylesheet {
    let mut sheet = Stylesheet {
        base_url: base_url.cloned(),
        ..Stylesheet::default()
    };
    if css.len() > MAX_STYLESHEET_BYTES {
        sheet.errors.push("stylesheet-size-limit".into());
        return sheet;
    }
    let raw = parse_rules(&tokenize_all(css));
    sheet
        .errors
        .extend(raw.errors.into_iter().map(str::to_owned));
    let mut budget = MAX_RULES;
    sheet.rules = semantic_rules(raw.rules, base_url, 0, &mut budget, &mut sheet.errors);
    sheet
}

fn semantic_rules(
    raw: Vec<RawRule>,
    base: Option<&Url>,
    depth: usize,
    budget: &mut usize,
    errors: &mut Vec<String>,
) -> Vec<Rule> {
    if depth > MAX_NESTING {
        return Vec::new();
    }
    let mut rules = Vec::new();
    let mut imports_allowed = depth == 0;
    for raw in raw {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        match raw {
            RawRule::Qualified { prelude, block } => {
                imports_allowed = false;
                match parse_tokens(&prelude) {
                    Ok(selectors) => rules.push(Rule::Style(StyleRule {
                        selectors,
                        declarations: declarations_from_tokens(&block),
                    })),
                    Err(_) => {
                        if errors.len() < 100 {
                            errors.push(format!("invalid-selector: {}", serialize(&prelude)));
                        }
                    }
                }
            }
            RawRule::At {
                name,
                prelude,
                block,
            } => match (name.as_str(), block) {
                ("charset", None) if depth == 0 => {}
                ("import", None) if imports_allowed => {
                    if let Some(import) = parse_import(&prelude, base) {
                        rules.push(Rule::Import(import));
                    } else if errors.len() < 100 {
                        errors.push("invalid-import".into());
                    }
                }
                ("layer", None) => {
                    let names = split_top_level(&prelude, &Token::Comma)
                        .iter()
                        .map(|p| serialize(trim(p)))
                        .filter(|s| !s.is_empty())
                        .collect();
                    rules.push(Rule::LayerOrder(names));
                }
                ("media", Some(body)) => {
                    imports_allowed = false;
                    let children =
                        semantic_rules(parse_rules(&body).rules, base, depth + 1, budget, errors);
                    rules.push(Rule::Media(MediaQueryList::from_tokens(&prelude), children));
                }
                ("supports", Some(body)) => {
                    imports_allowed = false;
                    let children =
                        semantic_rules(parse_rules(&body).rules, base, depth + 1, budget, errors);
                    rules.push(Rule::Supports(Supports::from_tokens(&prelude), children));
                }
                ("layer", Some(body)) => {
                    imports_allowed = false;
                    let name = if trim(&prelude).is_empty() {
                        None
                    } else {
                        Some(serialize(trim(&prelude)))
                    };
                    let children =
                        semantic_rules(parse_rules(&body).rules, base, depth + 1, budget, errors);
                    rules.push(Rule::Layer {
                        name,
                        rules: children,
                    });
                }
                ("font-face", Some(body)) => {
                    imports_allowed = false;
                    if let Some(face) = parse_font_face(&body, base) {
                        rules.push(Rule::FontFace(face));
                    }
                }
                ("keyframes" | "-webkit-keyframes", Some(body)) => {
                    imports_allowed = false;
                    if let [Token::Ident(name) | Token::QuotedString(name)] = trim(&prelude) {
                        let mut frames = Vec::new();
                        for frame in parse_rules(&body).rules {
                            if let RawRule::Qualified { prelude, block } = frame {
                                let mut positions = Vec::new();
                                let mut valid = true;
                                for p in split_top_level(&prelude, &Token::Comma) {
                                    match trim(p) {
                                        [Token::Ident(s)] if s == "from" => positions.push(0.0),
                                        [Token::Ident(s)] if s == "to" => positions.push(1.0),
                                        [Token::Percentage(p)] if (0.0..=100.0).contains(p) => {
                                            positions.push((*p / 100.0) as f32)
                                        }
                                        _ => valid = false,
                                    }
                                }
                                if valid {
                                    frames.push(Keyframe {
                                        positions,
                                        declarations: declarations_from_tokens(&block)
                                            .into_iter()
                                            .filter(|d| !d.important)
                                            .collect(),
                                    });
                                }
                            }
                        }
                        rules.push(Rule::Keyframes {
                            name: name.clone(),
                            frames,
                        });
                    }
                }
                _ => {
                    imports_allowed = false;
                }
            },
        }
    }
    rules
}

fn parse_import(tokens: &[Token], base: Option<&Url>) -> Option<ImportRule> {
    let tokens = trim(tokens);
    let (raw, consumed) = url_value(tokens)?;
    let url = resource_url(base, &raw)?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let mut rest = trim(&tokens[consumed..]);
    let layer = match rest.first() {
        Some(Token::Ident(name)) if name.eq_ignore_ascii_case("layer") => {
            rest = trim(&rest[1..]);
            Some(String::new())
        }
        Some(Token::Function(name)) if name.eq_ignore_ascii_case("layer") => {
            let (end, balanced) = crate::syntax::block_end(rest, 0);
            if !balanced {
                return None;
            }
            let layer = serialize(trim(&rest[1..end - 1]));
            rest = trim(&rest[end..]);
            Some(layer)
        }
        _ => None,
    };
    Some(ImportRule {
        url,
        media: MediaQueryList::from_tokens(rest),
        layer,
        sheet: None,
    })
}

pub(crate) fn url_value(tokens: &[Token]) -> Option<(String, usize)> {
    match tokens.first()? {
        Token::Url(s) | Token::QuotedString(s) => Some((s.clone(), 1)),
        Token::Function(name) if name.eq_ignore_ascii_case("url") => {
            let (end, balanced) = crate::syntax::block_end(tokens, 0);
            if !balanced {
                return None;
            }
            if let [Token::QuotedString(s)] = trim(&tokens[1..end - 1]) {
                Some((s.clone(), end))
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn resource_url(base: Option<&Url>, raw: &str) -> Option<Url> {
    let url = base
        .map_or_else(|| Url::parse(raw), |base| base.join(raw))
        .ok()?;
    match url.scheme() {
        "http" | "https" => Some(url),
        "data" if raw.len() <= 2 * 1024 * 1024 => Some(url),
        _ => None,
    }
}

fn parse_font_face(tokens: &[Token], base: Option<&Url>) -> Option<FontFace> {
    let declarations = declarations_from_tokens(tokens);
    let value = |name: &str| {
        declarations
            .iter()
            .rev()
            .find(|d| d.name == name)
            .map(|d| d.value.as_slice())
    };
    let family = font_families(value("font-family")?)?.into_iter().next()?;
    let mut sources = Vec::new();
    for part in split_top_level(value("src")?, &Token::Comma) {
        let part = trim(part);
        if matches!(part.first(),Some(Token::Function(name)) if name.eq_ignore_ascii_case("local"))
        {
            let (end, balanced) = crate::syntax::block_end(part, 0);
            if !balanced {
                continue;
            }
            if let Some(names) = font_families(&part[1..end - 1]) {
                for name in names {
                    sources.push(FontSource::Local(name));
                }
            }
        } else if let Some((raw, end)) = url_value(part) {
            let Some(url) = resource_url(base, &raw) else {
                continue;
            };
            let remaining = trim(&part[end..]);
            let format = if matches!(remaining.first(),Some(Token::Function(s)) if s.eq_ignore_ascii_case("format"))
            {
                remaining.get(1).and_then(|t| match t {
                    Token::Ident(s) | Token::QuotedString(s) => Some(s.clone()),
                    _ => None,
                })
            } else {
                None
            };
            sources.push(FontSource::Url { url, format });
        }
    }
    if sources.is_empty() {
        return None;
    }
    let weight = match value("font-weight").map(trim) {
        Some([Token::Number(n)]) if (1.0..=1000.0).contains(n) => *n as u16,
        Some(v) if keyword(v).as_deref() == Some("bold") => 700,
        _ => 400,
    };
    Some(FontFace {
        family,
        sources,
        weight,
        italic: value("font-style").and_then(keyword).as_deref() == Some("italic"),
        display: value("font-display")
            .and_then(keyword)
            .unwrap_or_else(|| "auto".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imports_font_descriptors_and_rule_order() {
        let base = "https://example.com/css/main.css".parse().unwrap();
        let sheet = parse_stylesheet("@layer base, theme; @import url('../shared.css') screen; @font-face {font-family:'Demo';src:local('Local'),url('demo.woff2') format('woff2');font-display:swap} p{color:red} @import 'late.css'; @bad {div{color:blue}}",Some(&base));
        assert_eq!(sheet.rules.len(), 4);
        assert!(
            matches!(&sheet.rules[1],Rule::Import(i) if i.url.as_str()=="https://example.com/shared.css")
        );
        assert!(
            matches!(&sheet.rules[2],Rule::FontFace(f) if f.sources.len()==2 && f.family=="Demo" && f.display=="swap")
        );
        assert!(resource_url(Some(&base), "file:///etc/passwd").is_none());
    }
}
