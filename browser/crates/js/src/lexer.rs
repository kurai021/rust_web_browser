//! Unicode identifiers, comments, numeric/string/regexp/template literals.
use crate::{ParseError, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Name(String),
    Number(f64),
    String(String),
    RegExp(String, String),
    Template(Vec<(String, Option<String>)>),
    Punct(String),
    Eof,
}
#[derive(Debug, Clone)]
pub struct Token {
    pub kind: Kind,
    pub span: Span,
    pub newline: bool,
}
pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    line: usize,
    column: usize,
    regex: bool,
    control: bool,
    parens: Vec<bool>,
}
impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Self {
            src,
            pos: 0,
            line: 1,
            column: 1,
            regex: true,
            control: false,
            parens: Vec::new(),
        }
    }
    fn peek(&self) -> Option<char> {
        self.src.get(self.pos..)?.chars().next()
    }
    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        if matches!(ch, '\n' | '\u{2028}' | '\u{2029}') {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }
    fn starts(&self, s: &str) -> bool {
        self.src.get(self.pos..).is_some_and(|r| r.starts_with(s))
    }
    fn span(&self) -> Span {
        Span {
            offset: self.pos,
            line: self.line,
            column: self.column,
        }
    }
    fn error(&self, s: &str) -> ParseError {
        ParseError::at(self.span(), s)
    }
    fn ident_start(ch: char) -> bool {
        ch == '$' || ch == '_' || ch.is_alphabetic()
    }
    fn ident_part(ch: char) -> bool {
        Self::ident_start(ch) || ch.is_numeric() || matches!(ch, '\u{200c}' | '\u{200d}')
    }
    fn hex(&mut self, digits: usize) -> Result<char, ParseError> {
        let mut value = 0;
        for _ in 0..digits {
            let n = self
                .bump()
                .and_then(|c| c.to_digit(16))
                .ok_or_else(|| self.error("invalid escape"))?;
            value = value * 16 + n;
        }
        char::from_u32(value).ok_or_else(|| self.error("invalid Unicode escape"))
    }
    fn escape(&mut self) -> Result<Option<char>, ParseError> {
        let ch = self
            .bump()
            .ok_or_else(|| self.error("unterminated escape"))?;
        Ok(match ch {
            'n' => Some('\n'),
            'r' => Some('\r'),
            't' => Some('\t'),
            'b' => Some('\x08'),
            'f' => Some('\x0c'),
            'v' => Some('\x0b'),
            '0' => Some('\0'),
            'x' => Some(self.hex(2)?),
            'u' => {
                if self.peek() == Some('{') {
                    self.bump();
                    let mut value = 0u32;
                    let mut count = 0;
                    while self.peek() != Some('}') {
                        let n = self
                            .bump()
                            .and_then(|c| c.to_digit(16))
                            .ok_or_else(|| self.error("invalid Unicode escape"))?;
                        count += 1;
                        if count > 6 {
                            return Err(self.error("Unicode escape too long"));
                        }
                        value = value * 16 + n;
                    }
                    self.bump();
                    Some(
                        char::from_u32(value)
                            .ok_or_else(|| self.error("invalid Unicode escape"))?,
                    )
                } else {
                    Some(self.hex(4)?)
                }
            }
            '\n' => None,
            '\r' => {
                if self.peek() == Some('\n') {
                    self.bump();
                }
                None
            }
            other => Some(other),
        })
    }
    fn quoted(&mut self, quote: char) -> Result<String, ParseError> {
        self.bump();
        let mut out = String::new();
        loop {
            match self.bump() {
                Some(c) if c == quote => return Ok(out),
                Some('\\') => {
                    if let Some(c) = self.escape()? {
                        out.push(c)
                    }
                }
                Some('\n' | '\r' | '\u{2028}' | '\u{2029}') => {
                    return Err(self.error("newline in string"))
                }
                Some(c) => out.push(c),
                None => return Err(self.error("unterminated string")),
            }
        }
    }
    fn template(&mut self) -> Result<Vec<(String, Option<String>)>, ParseError> {
        self.bump();
        let mut out = Vec::new();
        let mut text = String::new();
        loop {
            match self.bump() {
                Some('`') => {
                    out.push((text, None));
                    return Ok(out);
                }
                Some('\\') => {
                    if let Some(c) = self.escape()? {
                        text.push(c)
                    }
                }
                Some('$') if self.peek() == Some('{') => {
                    self.bump();
                    let start = self.pos;
                    let mut depth = 1usize;
                    let mut quote = None;
                    let mut escaped = false;
                    while depth > 0 {
                        let at = self.pos;
                        let c = self
                            .bump()
                            .ok_or_else(|| self.error("unterminated template expression"))?;
                        if let Some(q) = quote {
                            if escaped {
                                escaped = false;
                            } else if c == '\\' {
                                escaped = true;
                            } else if c == q {
                                quote = None;
                            }
                            continue;
                        }
                        match c {
                            '\'' | '"' | '`' => quote = Some(c),
                            '{' => {
                                depth += 1;
                                if depth > 128 {
                                    return Err(self.error("template nesting limit"));
                                }
                            }
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    out.push((
                                        std::mem::take(&mut text),
                                        Some(self.src[start..at].into()),
                                    ));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Some(c) => text.push(c),
                None => return Err(self.error("unterminated template")),
            }
        }
    }
    pub fn next(&mut self) -> Result<Token, ParseError> {
        let before = self.line;
        loop {
            while self
                .peek()
                .is_some_and(|c| c.is_whitespace() || c == '\u{feff}')
            {
                self.bump();
            }
            if self.starts("//") {
                while self.peek().is_some_and(|c| c != '\n' && c != '\r') {
                    self.bump();
                }
            } else if self.starts("/*") {
                self.bump();
                self.bump();
                while !self.starts("*/") {
                    if self.bump().is_none() {
                        return Err(self.error("unterminated comment"));
                    }
                }
                self.bump();
                self.bump();
            } else {
                break;
            }
        }
        let span = self.span();
        let newline = self.line != before;
        let Some(ch) = self.peek() else {
            return Ok(Token {
                kind: Kind::Eof,
                span,
                newline,
            });
        };
        let kind = if Self::ident_start(ch) || ch == '\\' {
            let mut name = String::new();
            while self
                .peek()
                .is_some_and(|c| Self::ident_part(c) || c == '\\')
            {
                let c = if self.peek() == Some('\\') {
                    self.bump();
                    if self.bump() != Some('u') {
                        return Err(self.error("identifier escape must be Unicode"));
                    }
                    self.hex(4)?
                } else {
                    self.bump().ok_or_else(|| self.error("identifier"))?
                };
                if !Self::ident_part(c) {
                    return Err(self.error("invalid identifier"));
                }
                name.push(c);
            }
            Kind::Name(name)
        } else if ch.is_ascii_digit()
            || (ch == '.'
                && self
                    .src
                    .get(self.pos + 1..)
                    .and_then(|r| r.chars().next())
                    .is_some_and(|c| c.is_ascii_digit()))
        {
            let start = self.pos;
            if self.starts("0x")
                || self.starts("0X")
                || self.starts("0b")
                || self.starts("0B")
                || self.starts("0o")
                || self.starts("0O")
            {
                self.bump();
                let radix = match self.bump() {
                    Some('x' | 'X') => 16,
                    Some('b' | 'B') => 2,
                    _ => 8,
                };
                let mut n = 0f64;
                let mut count = 0;
                while let Some(d) = self.peek().and_then(|c| c.to_digit(radix)) {
                    self.bump();
                    n = n * radix as f64 + d as f64;
                    count += 1;
                }
                if count == 0 {
                    return Err(self.error("missing radix digits"));
                }
                Kind::Number(n)
            } else {
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.bump();
                }
                if self.peek() == Some('.') {
                    self.bump();
                    while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                        self.bump();
                    }
                }
                if matches!(self.peek(), Some('e' | 'E')) {
                    self.bump();
                    if matches!(self.peek(), Some('+' | '-')) {
                        self.bump();
                    }
                    let first = self.pos;
                    while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                        self.bump();
                    }
                    if self.pos == first {
                        return Err(self.error("invalid exponent"));
                    }
                }
                Kind::Number(
                    self.src[start..self.pos]
                        .parse()
                        .map_err(|_| self.error("invalid number"))?,
                )
            }
        } else if ch == '\'' || ch == '"' {
            Kind::String(self.quoted(ch)?)
        } else if ch == '`' {
            Kind::Template(self.template()?)
        } else if ch == '/' && self.regex {
            self.bump();
            let mut pattern = String::new();
            let mut class = false;
            let mut escaped = false;
            loop {
                let c = self
                    .bump()
                    .ok_or_else(|| self.error("unterminated regexp"))?;
                if c == '\n' || c == '\r' {
                    return Err(self.error("newline in regexp"));
                }
                if !escaped && c == '/' && !class {
                    break;
                }
                if !escaped {
                    if c == '[' {
                        class = true;
                    }
                    if c == ']' {
                        class = false;
                    }
                }
                pattern.push(c);
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                }
            }
            let mut flags = String::new();
            while self.peek().is_some_and(Self::ident_part) {
                if let Some(c) = self.bump() {
                    flags.push(c);
                }
            }
            Kind::RegExp(pattern, flags)
        } else {
            let operators = [
                ">>>=", "===", "!==", ">>>", "**=", "<<=", ">>=", "...", "=>", "==", "!=", "<=",
                ">=", "++", "--", "&&", "||", "??", "+=", "-=", "*=", "/=", "%=", "<<", ">>", "&=",
                "|=", "^=", "**",
            ];
            if let Some(op) = operators.iter().find(|s| self.starts(s)) {
                let op = op.to_string();
                for _ in op.chars() {
                    self.bump();
                }
                Kind::Punct(op)
            } else {
                self.bump();
                if !"{}()[].,;:?~+-*/%<>=!&|^".contains(ch) {
                    return Err(self.error("unexpected character"));
                }
                Kind::Punct(ch.to_string())
            }
        };
        match &kind {
            Kind::Name(s) => {
                self.control = matches!(
                    s.as_str(),
                    "if" | "while" | "for" | "with" | "switch" | "catch"
                );
                self.regex = matches!(
                    s.as_str(),
                    "return"
                        | "throw"
                        | "case"
                        | "delete"
                        | "void"
                        | "typeof"
                        | "new"
                        | "in"
                        | "instanceof"
                        | "else"
                        | "yield"
                );
            }
            Kind::Punct(s) if s == "(" => {
                self.parens.push(self.control);
                self.control = false;
                self.regex = true;
            }
            Kind::Punct(s) if s == ")" => {
                self.regex = self.parens.pop().unwrap_or(false);
                self.control = false;
            }
            Kind::Punct(s) => {
                self.regex = !matches!(s.as_str(), "]" | "++" | "--" | ".");
                self.control = false;
            }
            _ => {
                self.regex = false;
                self.control = false;
            }
        }
        Ok(Token {
            kind,
            span,
            newline,
        })
    }
}
