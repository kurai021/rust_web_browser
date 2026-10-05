//! CSS Syntax 3 tokenizer (plan/06 §6.2).
//!
//! Comment-stripping token stream with spec escape handling. The common
//! input path is borrowed; newline/NUL preprocessing allocates only when
//! necessary (CSS Syntax 3 §3.3).

use std::borrow::Cow;

/// One syntax token (whitespace significant: preserved as `Whitespace`).
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// Whitespace run.
    Whitespace,
    /// `foo`, `--bar`, escapes resolved.
    Ident(String),
    /// `foo(` with the `(` consumed.
    Function(String),
    /// `@media`, `@import`.
    AtKeyword(String),
    /// `#abc`: `is_id` follows the spec ident rules.
    Hash { value: String, is_id: bool },
    /// `"…"` / `'…'`.
    QuotedString(String),
    /// Unterminated string (newline hit).
    BadString,
    /// `url(…)` / `url("…")` resolved value.
    Url(String),
    /// Bad `url(` (errors are recorded, parsing continues).
    BadUrl,
    /// Single punctuation char outside any other token.
    Delim(char),
    /// `12`, `+3.5`, `.5`.
    Number(f64),
    /// `50%`.
    Percentage(f64),
    /// `12px`, `2em` (unit lowercased).
    Dimension(f64, String),
    /// `:`.
    Colon,
    /// `;`.
    Semicolon,
    /// `,`.
    Comma,
    /// `[`.
    OpenSquare,
    /// `]`.
    CloseSquare,
    /// `(`.
    OpenParen,
    /// `)`.
    CloseParen,
    /// `{`.
    OpenCurly,
    /// `}`.
    CloseCurly,
    /// `-->`.
    Cdc,
    /// `<!--`.
    Cdo,
    /// End of input.
    Eof,
}

/// Tokenizer over decoded CSS text.
pub struct Tokenizer<'a> {
    input: Cow<'a, str>,
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    /// New tokenizer (input must already be decoded text).
    #[must_use]
    pub fn new(input: &'a str) -> Self {
        let input = if input.contains(['\r', '\x0C', '\0']) {
            Cow::Owned(
                input
                    .replace("\r\n", "\n")
                    .replace(['\r', '\x0C'], "\n")
                    .replace('\0', "\u{FFFD}"),
            )
        } else {
            Cow::Borrowed(input)
        };
        Self { input, pos: 0 }
    }

    fn rest(&self) -> &str {
        self.input.get(self.pos..).unwrap_or("")
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.rest().chars().nth(offset)
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.rest().chars().next()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    /// Pull the next token (comments are skipped silently).
    pub fn next_token(&mut self) -> Token {
        loop {
            let ch = match self.peek() {
                None => return Token::Eof,
                Some(ch) => ch,
            };
            // Whitespace run.
            if matches!(ch, ' ' | '\t' | '\n' | '\x0C' | '\r') {
                self.bump();
                while self
                    .peek()
                    .is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r'))
                {
                    self.bump();
                }
                return Token::Whitespace;
            }
            // Comments vanish (nested `/*` has no meaning in CSS).
            if self.rest().starts_with("/*") {
                self.pos += 2;
                while let Some(ch) = self.bump() {
                    if ch == '*' && self.peek() == Some('/') {
                        self.bump();
                        break;
                    }
                }
                continue;
            }
            if ch == '"' || ch == '\'' {
                return self.string_token();
            }
            if ch == '#' {
                self.bump();
                if !self.peek().is_some_and(name_char) && !self.valid_escape_at(0) {
                    return Token::Delim('#');
                }
                let is_id = self.starts_ident_sequence();
                let value = self.consume_name();
                return Token::Hash { value, is_id };
            }
            if ch == '+' || ch == '.' {
                if let Some(token) = self.numeric_token() {
                    return token;
                }
                self.bump();
                return Token::Delim(ch);
            }
            if ch.is_ascii_digit() {
                if let Some(token) = self.numeric_token() {
                    return token;
                }
                // Unreachable in practice (digits always start a number),
                // kept as a defensive fallback.
                self.bump();
                return Token::Delim(ch);
            }
            if ch == '-' {
                return self.minus_token();
            }
            if ch == '@' {
                self.bump();
                if self.starts_ident_sequence() {
                    return Token::AtKeyword(self.consume_name());
                }
                return Token::Delim('@');
            }
            if ch == '\\' && self.valid_escape_at(0) {
                return self.ident_like_token();
            }
            if starts_ident(ch) {
                return self.ident_like_token();
            }
            // Single-char tokens.
            self.bump();
            return match ch {
                ':' => Token::Colon,
                ';' => Token::Semicolon,
                ',' => Token::Comma,
                '[' => Token::OpenSquare,
                ']' => Token::CloseSquare,
                '(' => Token::OpenParen,
                ')' => Token::CloseParen,
                '{' => Token::OpenCurly,
                '}' => Token::CloseCurly,
                '<' if self.rest().starts_with("!--") => {
                    self.pos += 3;
                    Token::Cdo
                }
                _ => Token::Delim(ch),
            };
        }
    }

    /// `-` start: number, `-->` (CDC), `--ident`, or delim.
    fn minus_token(&mut self) -> Token {
        debug_assert_eq!(self.peek(), Some('-'));
        if let Some(token) = self.numeric_token() {
            return token;
        }
        if self.rest().get(1..3) == Some("->") {
            self.pos += 3;
            return Token::Cdc;
        }
        if self.starts_ident_sequence() {
            return self.ident_like_token();
        }
        self.bump();
        Token::Delim('-')
    }

    /// CSS-valid escape at a character offset (EOF after `\` is valid).
    fn valid_escape_at(&self, offset: usize) -> bool {
        let mut chars = self.rest().chars().skip(offset);
        chars.next() == Some('\\') && !matches!(chars.next(), Some('\n'))
    }

    fn starts_ident_sequence(&self) -> bool {
        match self.peek() {
            Some('-') => {
                self.peek_at(1)
                    .is_some_and(|ch| starts_ident(ch) || ch == '-')
                    || self.valid_escape_at(1)
            }
            Some('\\') => self.valid_escape_at(0),
            Some(ch) => starts_ident(ch),
            None => false,
        }
    }

    fn ident_like_token(&mut self) -> Token {
        let name = self.consume_name();
        if self.peek() != Some('(') {
            return Token::Ident(name);
        }
        self.bump();
        if !name.eq_ignore_ascii_case("url") {
            return Token::Function(name);
        }
        // Quoted URLs are function + string tokens in CSS Syntax, rather
        // than URL tokens. Unquoted URLs use the dedicated URL state.
        let after_ws = self.rest().trim_start_matches(css_whitespace);
        if after_ws.starts_with(['\'', '"']) {
            Token::Function(name)
        } else {
            self.url_token()
        }
    }

    fn url_token(&mut self) -> Token {
        while self.peek().is_some_and(css_whitespace) {
            self.bump();
        }
        let mut value = String::new();
        loop {
            match self.peek() {
                None => return Token::Url(value),
                Some(')') => {
                    self.bump();
                    return Token::Url(value);
                }
                Some(ch) if css_whitespace(ch) => {
                    while self.peek().is_some_and(css_whitespace) {
                        self.bump();
                    }
                    match self.peek() {
                        None => return Token::Url(value),
                        Some(')') => {
                            self.bump();
                            return Token::Url(value);
                        }
                        _ => return self.bad_url(),
                    }
                }
                Some('"' | '\'' | '(') => return self.bad_url(),
                Some(ch) if ch.is_control() => return self.bad_url(),
                Some('\\') => {
                    if !self.valid_escape_at(0) {
                        return self.bad_url();
                    }
                    self.bump();
                    value.push(self.consume_escape().unwrap_or('\u{FFFD}'));
                }
                Some(ch) => {
                    self.bump();
                    value.push(ch);
                }
            }
        }
    }

    fn bad_url(&mut self) -> Token {
        while let Some(ch) = self.bump() {
            if ch == ')' {
                break;
            }
            if ch == '\\' && self.peek() != Some('\n') {
                self.consume_escape();
            }
        }
        Token::BadUrl
    }

    /// Parse a number if present; shared by all numeric entry points.
    fn consume_number_parts(&mut self) -> Option<(f64, usize)> {
        let rest = self.rest();
        let mut end = 0usize;
        let mut chars = rest.chars().peekable();
        if matches!(chars.peek(), Some('+' | '-')) {
            end += 1;
            chars.next();
        }
        let mut int_digits = 0;
        while matches!(chars.peek(), Some('0'..='9')) {
            end += 1;
            chars.next();
            int_digits += 1;
        }
        let mut frac_digits = 0;
        {
            let mut clone = chars.clone();
            let mut extra = 0usize;
            if clone.next() == Some('.') {
                extra += 1;
                let mut count = 0;
                while matches!(clone.peek(), Some('0'..='9')) {
                    clone.next();
                    extra += 1;
                    count += 1;
                }
                if count > 0 {
                    end += extra;
                    chars = clone;
                    frac_digits = count;
                }
            }
        }
        if int_digits == 0 && frac_digits == 0 {
            return None;
        }
        {
            let mut clone = chars.clone();
            let mut extra = 0usize;
            if matches!(clone.peek(), Some('e' | 'E')) {
                clone.next();
                extra += 1;
                if matches!(clone.peek(), Some('+' | '-')) {
                    clone.next();
                    extra += 1;
                }
                let mut exp_digits = 0;
                while matches!(clone.peek(), Some('0'..='9')) {
                    clone.next();
                    extra += 1;
                    exp_digits += 1;
                }
                if exp_digits > 0 {
                    end += extra;
                }
            }
        }
        // Byte length of the consumed chars (all ASCII by construction).
        let value: f64 = rest[..end].parse().ok()?;
        Some((value, end))
    }

    /// Number / percentage / dimension, or `None` (not a number).
    fn numeric_token(&mut self) -> Option<Token> {
        let (value, len) = self.consume_number_parts()?;
        self.pos += len;
        match self.peek() {
            Some('%') => {
                self.bump();
                Some(Token::Percentage(value))
            }
            _ if self.starts_ident_sequence() => {
                let unit = self.consume_name().to_ascii_lowercase();
                Some(Token::Dimension(value, unit))
            }
            _ => Some(Token::Number(value)),
        }
    }

    /// Identifier or function token start (`-` handled by caller).
    fn consume_name(&mut self) -> String {
        let mut out = String::new();
        // Leading `--` or `-` prefix is consumed by the caller except here.
        loop {
            match self.peek() {
                Some(ch) if starts_ident(ch) || ch.is_ascii_digit() || ch == '-' => {
                    self.bump();
                    out.push(ch);
                }
                Some('\\') => {
                    if self.at_newline_escape() {
                        break;
                    }
                    self.bump();
                    match self.consume_escape() {
                        Some(ch) => out.push(ch),
                        None => out.push('�'),
                    }
                }
                _ => break,
            }
        }
        out
    }

    /// True when `\` starts a newline escape (which ends the name).
    fn at_newline_escape(&self) -> bool {
        let mut chars = self.rest().chars();
        chars.next() == Some('\\') && matches!(chars.next(), Some('\n' | '\r' | '\x0C'))
    }

    /// Consume an escape after the backslash; `None` on EOF.
    fn consume_escape(&mut self) -> Option<char> {
        // Hex escape: 1-6 digits + optional whitespace.
        let mut hex = String::new();
        while hex.len() < 6 {
            match self.peek() {
                Some(ch) if ch.is_ascii_hexdigit() => {
                    hex.push(ch);
                    self.bump();
                }
                _ => break,
            }
        }
        if !hex.is_empty() {
            // Consume one trailing whitespace after a hex escape.
            if self
                .peek()
                .is_some_and(|ch| matches!(ch, ' ' | '\t' | '\n' | '\x0C' | '\r'))
            {
                if self.peek() == Some('\r') {
                    self.bump();
                    if self.peek() == Some('\n') {
                        self.bump();
                    }
                } else {
                    self.bump();
                }
            }
            let value = u32::from_str_radix(&hex, 16).unwrap_or(0);
            if value == 0 || value > 0x10FFFF || (0xD800..=0xDFFF).contains(&value) {
                return Some('�');
            }
            return char::from_u32(value);
        }
        match self.bump() {
            None => None,
            Some('\n') => None,
            Some(ch) => Some(ch),
        }
    }

    fn string_token(&mut self) -> Token {
        let quote = self.bump().expect("quote peeked");
        let mut out = String::new();
        loop {
            match self.peek() {
                None => {
                    // EOF in string: return what we have (error recorded upstream).
                    return Token::QuotedString(out);
                }
                Some(ch) if ch == quote => {
                    self.bump();
                    return Token::QuotedString(out);
                }
                Some('\n') | Some('\x0C') | Some('\r') => {
                    return Token::BadString;
                }
                Some('\0') => {
                    self.bump();
                    out.push('�');
                }
                Some('\\') => {
                    self.bump();
                    match self.peek() {
                        None => {
                            return Token::QuotedString(out);
                        }
                        Some('\n') | Some('\x0C') | Some('\r') => {
                            // Escaped newline: line continuation, skip both.
                            self.bump();
                        }
                        Some(_) => match self.consume_escape() {
                            Some(ch) => out.push(ch),
                            None => out.push('�'),
                        },
                    }
                }
                Some(_) => out.push(self.bump().expect("peeked")),
            }
        }
    }
}

/// Identifier start: letters, `_`, non-ASCII (dash handled by callers).
fn starts_ident(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_' || !ch.is_ascii()
}

fn name_char(ch: char) -> bool {
    starts_ident(ch) || ch.is_ascii_digit() || ch == '-'
}

pub(crate) fn css_whitespace(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r' | '\x0C')
}

/// Serialize tokens for diagnostics/CSSOM. Escapes are emitted safely so
/// escaped identifiers and strings cannot turn into delimiters on reparse.
pub fn serialize(tokens: &[Token]) -> String {
    let mut text = String::new();
    for token in tokens {
        match token {
            Token::Whitespace => text.push(' '),
            Token::Ident(name) => text.push_str(&escape_ident(name)),
            Token::Function(name) => {
                text.push_str(&escape_ident(name));
                text.push('(');
            }
            Token::AtKeyword(name) => {
                text.push('@');
                text.push_str(&escape_ident(name));
            }
            Token::Hash { value, .. } => {
                text.push('#');
                text.push_str(value);
            }
            Token::QuotedString(value) => {
                text.push('"');
                text.push_str(
                    &value
                        .replace('\\', "\\\\")
                        .replace('"', "\\\"")
                        .replace('\n', "\\a "),
                );
                text.push('"');
            }
            Token::Url(value) => {
                text.push_str("url(");
                text.push_str(&serialize(&[Token::QuotedString(value.clone())]));
                text.push(')');
            }
            Token::Number(value) => text.push_str(&value.to_string()),
            Token::Percentage(value) => {
                text.push_str(&value.to_string());
                text.push('%');
            }
            Token::Dimension(value, unit) => {
                text.push_str(&value.to_string());
                text.push_str(&escape_ident(unit));
            }
            Token::Delim(ch) => text.push(*ch),
            Token::Colon => text.push(':'),
            Token::Semicolon => text.push(';'),
            Token::Comma => text.push(','),
            Token::OpenSquare => text.push('['),
            Token::CloseSquare => text.push(']'),
            Token::OpenParen => text.push('('),
            Token::CloseParen => text.push(')'),
            Token::OpenCurly => text.push('{'),
            Token::CloseCurly => text.push('}'),
            Token::Cdo => text.push_str("<!--"),
            Token::Cdc => text.push_str("-->"),
            Token::BadString | Token::BadUrl | Token::Eof => {}
        }
    }
    text
}

fn escape_ident(name: &str) -> String {
    let mut out = String::new();
    for (index, ch) in name.chars().enumerate() {
        if name_char(ch) && !(ch.is_ascii_digit() && index == 0) {
            out.push(ch);
        } else {
            out.push_str(&format!("\\{:x} ", ch as u32));
        }
    }
    out
}

/// Tokenizer entry shared by the parser: numbers route here.
pub fn tokenize_all(input: &str) -> Vec<Token> {
    let mut tokenizer = Tokenizer::new(input);
    let mut tokens = Vec::new();
    loop {
        match tokenizer.next_token() {
            Token::Eof => break,
            token => tokens.push(token),
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(input: &str) -> Vec<Token> {
        tokenize_all(input)
    }

    #[test]
    fn basics() {
        assert_eq!(
            tokens("div { color: red; }"),
            vec![
                Token::Ident("div".to_owned()),
                Token::Whitespace,
                Token::OpenCurly,
                Token::Whitespace,
                Token::Ident("color".to_owned()),
                Token::Colon,
                Token::Whitespace,
                Token::Ident("red".to_owned()),
                Token::Semicolon,
                Token::Whitespace,
                Token::CloseCurly,
            ]
        );
    }

    #[test]
    fn numbers_dimensions_percentages() {
        assert_eq!(
            tokens("12px 50% -3.5e2 .5 +2"),
            vec![
                Token::Dimension(12.0, "px".to_owned()),
                Token::Whitespace,
                Token::Percentage(50.0),
                Token::Whitespace,
                Token::Number(-350.0),
                Token::Whitespace,
                Token::Number(0.5),
                Token::Whitespace,
                Token::Number(2.0),
            ]
        );
    }

    #[test]
    fn strings_hashes_at() {
        assert_eq!(
            tokens("#id @media \"a\" 'b'"),
            vec![
                Token::Hash {
                    value: "id".to_owned(),
                    is_id: true
                },
                Token::Whitespace,
                Token::AtKeyword("media".to_owned()),
                Token::Whitespace,
                Token::QuotedString("a".to_owned()),
                Token::Whitespace,
                Token::QuotedString("b".to_owned()),
            ]
        );
    }

    #[test]
    fn comments_vanish() {
        assert_eq!(
            tokens("a/* x */b"),
            vec![Token::Ident("a".to_owned()), Token::Ident("b".to_owned())]
        );
    }

    #[test]
    fn escapes_resolve() {
        assert_eq!(tokens("\\31 23"), vec![Token::Ident("123".to_owned())]);
        assert_eq!(tokens("\\41"), vec![Token::Ident("A".to_owned())]);
    }

    #[test]
    fn functions_urls_and_identifier_boundaries() {
        assert_eq!(
            tokens("calc(2px + 1em)"),
            vec![
                Token::Function("calc".into()),
                Token::Dimension(2.0, "px".into()),
                Token::Whitespace,
                Token::Delim('+'),
                Token::Whitespace,
                Token::Dimension(1.0, "em".into()),
                Token::CloseParen
            ]
        );
        assert_eq!(
            tokens("url( a\\ b.png )"),
            vec![Token::Url("a b.png".into())]
        );
        assert_eq!(
            tokens("url(\"a.png\")"),
            vec![
                Token::Function("url".into()),
                Token::QuotedString("a.png".into()),
                Token::CloseParen
            ]
        );
        assert_eq!(
            tokens("url(a b)c"),
            vec![Token::BadUrl, Token::Ident("c".into())]
        );
        assert_eq!(
            tokens("--x -1- # @\\61 bc"),
            vec![
                Token::Ident("--x".into()),
                Token::Whitespace,
                Token::Number(-1.0),
                Token::Delim('-'),
                Token::Whitespace,
                Token::Delim('#'),
                Token::Whitespace,
                Token::AtKeyword("abc".into())
            ]
        );
    }
}
