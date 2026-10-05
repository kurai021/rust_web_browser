//! CSS Syntax 3 rules, blocks and declarations (plan/06 §6.2/§6.4).
//! Unknown at-rules remain intact for the stylesheet layer. Recovery
//! always advances the cursor; balanced values keep their original tokens.

use crate::token::{tokenize_all, Token};

pub const MAX_STYLESHEET_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_RULES: usize = 50_000;
pub const MAX_NESTING: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    pub name: String,
    pub value: Vec<Token>,
    pub important: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RawRule {
    Qualified {
        prelude: Vec<Token>,
        block: Vec<Token>,
    },
    At {
        name: String,
        prelude: Vec<Token>,
        block: Option<Vec<Token>>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct SyntaxResult {
    pub rules: Vec<RawRule>,
    pub errors: Vec<&'static str>,
}

pub fn parse_rules(tokens: &[Token]) -> SyntaxResult {
    let mut result = SyntaxResult::default();
    let mut index = 0;
    while index < tokens.len() && result.rules.len() < MAX_RULES {
        if matches!(
            tokens[index],
            Token::Whitespace | Token::Cdo | Token::Cdc | Token::Semicolon
        ) {
            index += 1;
            continue;
        }
        if tokens[index] == Token::CloseCurly {
            record_error(&mut result.errors, "unexpected-close-curly");
            index += 1;
            continue;
        }
        let at_name = if let Token::AtKeyword(name) = &tokens[index] {
            index += 1;
            Some(name.to_ascii_lowercase())
        } else {
            None
        };
        let start = index;
        let mut stack = Vec::new();
        while index < tokens.len() {
            match &tokens[index] {
                Token::OpenCurly if stack.is_empty() => break,
                Token::Semicolon if stack.is_empty() && at_name.is_some() => break,
                token => update_stack(&mut stack, token),
            }
            if stack.len() > MAX_NESTING {
                record_error(&mut result.errors, "nesting-limit");
                return result;
            }
            index += 1;
        }
        let prelude = trim(&tokens[start..index]).to_vec();
        if tokens.get(index) == Some(&Token::OpenCurly) {
            let (end, balanced) = block_end(tokens, index);
            if !balanced {
                record_error(&mut result.errors, "unclosed-block");
            }
            let body_end = if balanced { end - 1 } else { end };
            let block = tokens[index + 1..body_end].to_vec();
            index = end;
            match at_name {
                Some(name) => result.rules.push(RawRule::At {
                    name,
                    prelude,
                    block: Some(block),
                }),
                None if !prelude.is_empty() => {
                    result.rules.push(RawRule::Qualified { prelude, block })
                }
                _ => record_error(&mut result.errors, "empty-selector"),
            }
        } else if let Some(name) = at_name {
            result.rules.push(RawRule::At {
                name,
                prelude,
                block: None,
            });
            if index < tokens.len() {
                index += 1;
            }
        } else {
            record_error(&mut result.errors, "qualified-rule-without-block");
        }
    }
    result
}

pub fn parse_declarations(css: &str) -> Vec<Declaration> {
    if css.len() > MAX_STYLESHEET_BYTES {
        return Vec::new();
    }
    declarations_from_tokens(&tokenize_all(css))
}

pub fn declarations_from_tokens(tokens: &[Token]) -> Vec<Declaration> {
    let mut out = Vec::new();
    for part in split_top_level(tokens, &Token::Semicolon) {
        let part = trim(part);
        let Some(Token::Ident(name)) = part.first() else {
            continue;
        };
        let rest = trim(&part[1..]);
        if rest.first() != Some(&Token::Colon) {
            continue;
        }
        let mut value = trim(&rest[1..]).to_vec();
        if value
            .iter()
            .any(|t| matches!(t, Token::BadString | Token::BadUrl | Token::CloseCurly))
            || !balanced_value(&value)
        {
            continue;
        }
        let mut important = false;
        if matches!(value.last(), Some(Token::Ident(v)) if v.eq_ignore_ascii_case("important")) {
            let mut end = value.len().saturating_sub(1);
            while end > 0 && value[end - 1] == Token::Whitespace {
                end -= 1;
            }
            if end > 0 && value[end - 1] == Token::Delim('!') {
                important = true;
                value.truncate(end - 1);
                value = trim(&value).to_vec();
            }
        }
        if value.is_empty() && !name.starts_with("--") {
            continue;
        }
        out.push(Declaration {
            name: if name.starts_with("--") {
                name.clone()
            } else {
                name.to_ascii_lowercase()
            },
            value,
            important,
        });
    }
    out
}

pub(crate) fn trim(tokens: &[Token]) -> &[Token] {
    let start = tokens
        .iter()
        .position(|t| *t != Token::Whitespace)
        .unwrap_or(tokens.len());
    let end = tokens
        .iter()
        .rposition(|t| *t != Token::Whitespace)
        .map_or(start, |i| i + 1);
    &tokens[start..end]
}

pub(crate) fn split_top_level<'a>(tokens: &'a [Token], separator: &Token) -> Vec<&'a [Token]> {
    let mut parts = Vec::new();
    let mut stack = Vec::new();
    let mut start = 0;
    for (index, token) in tokens.iter().enumerate() {
        if stack.is_empty() && token == separator {
            parts.push(&tokens[start..index]);
            start = index + 1;
        } else {
            update_stack(&mut stack, token);
        }
    }
    parts.push(&tokens[start..]);
    parts
}

/// End index AFTER the matching close delimiter; EOF is a recovered end.
pub(crate) fn block_end(tokens: &[Token], start: usize) -> (usize, bool) {
    let mut stack = Vec::new();
    for (index, token) in tokens.iter().enumerate().skip(start) {
        update_stack(&mut stack, token);
        if stack.len() > MAX_NESTING {
            return (tokens.len(), false);
        }
        if stack.is_empty() {
            return (index + 1, true);
        }
    }
    (tokens.len(), false)
}

fn update_stack(stack: &mut Vec<Token>, token: &Token) {
    match token {
        Token::Function(_) | Token::OpenParen => stack.push(Token::CloseParen),
        Token::OpenCurly => stack.push(Token::CloseCurly),
        Token::OpenSquare => stack.push(Token::CloseSquare),
        token if stack.last() == Some(token) => {
            stack.pop();
        }
        _ => {}
    }
}

fn balanced_value(tokens: &[Token]) -> bool {
    let mut stack = Vec::new();
    for token in tokens {
        if matches!(
            token,
            Token::CloseCurly | Token::CloseSquare | Token::CloseParen
        ) && stack.last() != Some(token)
        {
            return false;
        }
        update_stack(&mut stack, token);
        if stack.len() > MAX_NESTING {
            return false;
        }
    }
    stack.is_empty()
}

fn record_error(errors: &mut Vec<&'static str>, error: &'static str) {
    if errors.len() < 100 {
        errors.push(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_preserves_valid_declarations_and_function_semicolons() {
        let decls = parse_declarations("broken; color:red ! IMPORTANT; x: url(\"a;b\"); --Case: calc(1px + 2px); color:blue; width:); padding:3px");
        assert_eq!(decls.len(), 5);
        assert!(decls[0].important);
        assert_eq!(decls[2].name, "--Case");
        assert_eq!(decls[4].name, "padding");
    }

    #[test]
    fn rule_blocks_and_unknown_at_rules_do_not_leak() {
        let parsed = parse_rules(&tokenize_all("@import 'x.css'; @media (width: 1px) { p{color:red} } @unknown { a{bad:yes} } b { color:blue }"));
        assert_eq!(parsed.rules.len(), 4);
        assert!(
            matches!(&parsed.rules[1], RawRule::At { name, block: Some(_), .. } if name == "media")
        );
        assert!(matches!(&parsed.rules[3], RawRule::Qualified { .. }));
    }
}
