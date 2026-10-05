//! Media Queries 4 and basic @supports conditions (plan/06 §6.2).

use crate::syntax::{block_end, split_top_level, trim};
use crate::token::{tokenize_all, Token};
use crate::values::{keyword, Length, LengthContext};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Environment {
    pub width: f32,
    pub height: f32,
    pub dark: bool,
}
impl Default for Environment {
    fn default() -> Self {
        Self {
            width: 800.0,
            height: 600.0,
            dark: false,
        }
    }
}
impl Environment {
    pub fn length_context(
        self,
        font_size: f32,
        root_font_size: f32,
        percentage_basis: f32,
    ) -> LengthContext {
        LengthContext {
            font_size,
            root_font_size,
            percentage_basis,
            viewport_width: self.width,
            viewport_height: self.height,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaQueryList {
    queries: Vec<Condition>,
}
impl Default for MediaQueryList {
    fn default() -> Self {
        Self {
            queries: vec![Condition::True],
        }
    }
}
impl MediaQueryList {
    pub fn parse(css: &str) -> Self {
        Self::from_tokens(&tokenize_all(css))
    }
    pub(crate) fn from_tokens(tokens: &[Token]) -> Self {
        if trim(tokens).is_empty() {
            return Self::default();
        }
        let queries = split_top_level(tokens, &Token::Comma)
            .iter()
            .map(|part| condition(part, Mode::Media, 0))
            .collect();
        Self { queries }
    }
    pub fn matches(&self, env: Environment) -> bool {
        self.queries.iter().any(|c| c.matches(env, &|_, _| false))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Supports(pub(crate) Condition);
impl Supports {
    pub(crate) fn from_tokens(tokens: &[Token]) -> Self {
        Self(condition(tokens, Mode::Supports, 0))
    }
    pub fn matches(&self, test: impl Fn(&str, &[Token]) -> bool) -> bool {
        self.0.matches(Environment::default(), &test)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Condition {
    True,
    False,
    Not(Box<Condition>),
    And(Vec<Condition>),
    Or(Vec<Condition>),
    Feature {
        name: String,
        value: Vec<Token>,
        comparison: String,
    },
    Declaration {
        name: String,
        value: Vec<Token>,
    },
}
#[derive(Clone, Copy)]
enum Mode {
    Media,
    Supports,
}

fn condition(tokens: &[Token], mode: Mode, depth: usize) -> Condition {
    if depth > 32 {
        return Condition::False;
    }
    let tokens = trim(tokens);
    if tokens.is_empty() {
        return Condition::False;
    }
    if matches!(tokens.first(), Some(Token::Ident(s)) if s.eq_ignore_ascii_case("only")) {
        return condition(&tokens[1..], mode, depth + 1);
    }
    if matches!(tokens.first(), Some(Token::Ident(s)) if s.eq_ignore_ascii_case("not")) {
        return Condition::Not(Box::new(condition(&tokens[1..], mode, depth + 1)));
    }
    for (word, is_or) in [("or", true), ("and", false)] {
        let mut parts = Vec::new();
        let mut index = 0;
        let mut start = 0;
        while index < tokens.len() {
            if matches!(tokens[index], Token::OpenParen | Token::Function(_)) {
                index = block_end(tokens, index).0;
                continue;
            }
            if matches!(&tokens[index], Token::Ident(s) if s.eq_ignore_ascii_case(word)) {
                parts.push(condition(&tokens[start..index], mode, depth + 1));
                start = index + 1;
            }
            index += 1;
        }
        if !parts.is_empty() {
            parts.push(condition(&tokens[start..], mode, depth + 1));
            return if is_or {
                Condition::Or(parts)
            } else {
                Condition::And(parts)
            };
        }
    }
    if tokens.first() == Some(&Token::OpenParen) && block_end(tokens, 0) == (tokens.len(), true) {
        let body = trim(&tokens[1..tokens.len() - 1]);
        if let Some(Token::Ident(name)) = body.first() {
            let rest = trim(&body[1..]);
            if rest.first() == Some(&Token::Colon) {
                let value = trim(&rest[1..]).to_vec();
                return match mode {
                    Mode::Supports => Condition::Declaration {
                        name: name.to_ascii_lowercase(),
                        value,
                    },
                    Mode::Media => Condition::Feature {
                        name: name.to_ascii_lowercase(),
                        value,
                        comparison: ":".into(),
                    },
                };
            }
            if matches!(mode, Mode::Media) {
                if rest.is_empty() {
                    return Condition::Feature {
                        name: name.to_ascii_lowercase(),
                        value: Vec::new(),
                        comparison: ":".into(),
                    };
                }
                if let Some(Token::Delim(op @ ('<' | '>' | '='))) = rest.first() {
                    let equal = rest.get(1) == Some(&Token::Delim('='));
                    let comparison = format!("{op}{}", if equal { "=" } else { "" });
                    let value = trim(&rest[1 + usize::from(equal)..]).to_vec();
                    return Condition::Feature {
                        name: name.to_ascii_lowercase(),
                        value,
                        comparison,
                    };
                }
            }
        }
        return condition(body, mode, depth + 1);
    }
    if matches!(mode, Mode::Media) {
        if let [Token::Ident(kind)] = tokens {
            // A future print operation may reuse screen layout, but must
            // not apply print-only rules to the current screen viewport.
            return if matches!(kind.to_ascii_lowercase().as_str(), "all" | "screen") {
                Condition::True
            } else {
                Condition::False
            };
        }
    }
    Condition::False
}

impl Condition {
    fn matches(&self, env: Environment, supports: &impl Fn(&str, &[Token]) -> bool) -> bool {
        match self {
            Self::True => true,
            Self::False => false,
            Self::Not(c) => !c.matches(env, supports),
            Self::And(cs) => cs.iter().all(|c| c.matches(env, supports)),
            Self::Or(cs) => cs.iter().any(|c| c.matches(env, supports)),
            Self::Declaration { name, value } => supports(name, value),
            Self::Feature {
                name,
                value,
                comparison,
            } => match name.as_str() {
                "orientation" => keyword(value).is_some_and(|v| {
                    if v == "portrait" {
                        env.height >= env.width
                    } else if v == "landscape" {
                        env.width > env.height
                    } else {
                        false
                    }
                }),
                "prefers-color-scheme" => keyword(value).is_some_and(|v| match v.as_str() {
                    "dark" => env.dark,
                    "light" => !env.dark,
                    _ => false,
                }),
                _ => {
                    let (name, bound) = if let Some(s) = name.strip_prefix("min-") {
                        (s, ">=")
                    } else if let Some(s) = name.strip_prefix("max-") {
                        (s, "<=")
                    } else {
                        (name.as_str(), comparison.as_str())
                    };
                    let actual = match name {
                        "width" => env.width,
                        "height" => env.height,
                        _ => return false,
                    };
                    if value.is_empty() {
                        return actual != 0.0;
                    }
                    if value.iter().any(|t| matches!(t, Token::Percentage(_))) {
                        return false;
                    }
                    let Some(limit) = Length::parse(value)
                        .and_then(|l| l.resolve(env.length_context(16.0, 16.0, 0.0)))
                    else {
                        return false;
                    };
                    match bound {
                        ":" | "=" => actual == limit,
                        ">=" => actual >= limit,
                        "<=" => actual <= limit,
                        ">" => actual > limit,
                        "<" => actual < limit,
                        _ => false,
                    }
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn viewport_orientation_negation_and_lists() {
        let env = Environment {
            width: 600.0,
            height: 800.0,
            dark: true,
        };
        for q in [
            "screen and (max-width: 600px)",
            "(orientation: portrait)",
            "(prefers-color-scheme: dark)",
            "(width >= 30em)",
            "speech, (min-width: 200px)",
            "not speech",
        ] {
            assert!(MediaQueryList::parse(q).matches(env), "{q}");
        }
        for q in [
            "(width: junk)",
            "(width: 60%)",
            "not screen",
            "(min-height: 1000px)",
            "(unknown:yes)",
        ] {
            assert!(!MediaQueryList::parse(q).matches(env), "{q}");
        }
    }
}
