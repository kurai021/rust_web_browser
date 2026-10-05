//! WHATWG tokenizer (13.2.5, plan/05 §5.2).
//!
//! Driven token-by-token by the tree builder, which adjusts the content
//! model (`set_state_for_tag`) and the CDATA permission before each token.
//! Character references resolve via `entities.rs`. Offsets are UTF-8 byte
//! offsets into the decoded input.

use crate::dom::ParseError;
use crate::entities::{consume_reference, ReferenceBadness};

/// One emitted token. Adjacent `Characters` runs are maximal; the `.test`
/// harness joins them before comparing (like html5ever's driver).
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// `<!DOCTYPE …>`.
    Doctype {
        name: Option<String>,
        public_id: Option<String>,
        system_id: Option<String>,
        force_quirks: bool,
    },
    /// `<tag …>` / `<tag …/>`.
    StartTag {
        name: String,
        attributes: Vec<(String, String)>,
        self_closing: bool,
    },
    /// `</tag>`.
    EndTag { name: String },
    /// `<!-- … -->` (and bogus variants).
    Comment(String),
    /// Character run.
    Characters(String),
    /// End of input.
    Eof,
}

/// Tag-name based content model switch requested by the tree builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentModel {
    Pcdata,
    Rcdata,
    Rawtext,
    ScriptData,
    Plaintext,
}

/// Result of probing for an appropriate end tag (`<` already consumed).
enum EndTagAttempt {
    /// Complete `</name>` / `</name/>` (attributes skipped per spec).
    Tag(Token),
    /// Not an end tag; cursor rewound to just after `<`.
    NotATag,
}

/// The tokenizer: incremental pull API over decoded text.
///
/// Owns its input: the driver appends decoded chunks via `feed`, and the
/// cursor, content model and stash persist across pushes. Byte offsets in
/// errors are stable because text is only ever appended.
pub struct Tokenizer {
    input: String,
    /// Byte offset cursor. Always reset via `rewind` (clears lookahead).
    pos: usize,
    /// One-char lookahead (byte offset, char).
    peeked: Option<(usize, char)>,
    model: ContentModel,
    /// Last start tag name (for RCDATA/RAWTEXT appropriate end tags).
    last_start_tag: Option<String>,
    /// CDATA sections only tokenize inside foreign content.
    cdata_allowed: bool,
    /// One-token stash for the text-then-end-tag split.
    stashed: Option<Token>,
    /// Double-escape depth inside `<script><!--…`.
    escaped_depth: u32,
    /// Collected errors.
    pub errors: Vec<ParseError>,
}

impl Default for Tokenizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Tokenizer {
    /// New empty tokenizer; feed decoded text before pulling.
    #[must_use]
    pub fn new() -> Self {
        Self {
            input: String::new(),
            pos: 0,
            peeked: None,
            model: ContentModel::Pcdata,
            last_start_tag: None,
            cdata_allowed: false,
            stashed: None,
            escaped_depth: 0,
            errors: Vec::new(),
        }
    }

    /// Append decoded text (byte offsets stay stable).
    pub fn feed(&mut self, chunk: &str) {
        self.input.push_str(chunk);
    }

    /// True when all fed input is consumed.
    #[must_use]
    pub fn is_exhausted(&self) -> bool {
        self.pos >= self.input.len() && self.stashed.is_none()
    }

    /// Switch content model after a start tag (called by the tree builder).
    pub fn set_state_for_tag(&mut self, tag: &str) {
        self.last_start_tag = Some(tag.to_owned());
        self.escaped_depth = 0;
        self.model = match tag {
            "textarea" | "title" => ContentModel::Rcdata,
            "style" | "iframe" | "noembed" | "noframes" | "noscript" | "xmp" => {
                ContentModel::Rawtext
            }
            "script" => ContentModel::ScriptData,
            "plaintext" => ContentModel::Plaintext,
            _ => ContentModel::Pcdata,
        };
    }

    /// Reset to PCDATA (called after the matching end tag / text section).
    pub fn set_pcdata(&mut self) {
        self.model = ContentModel::Pcdata;
        self.escaped_depth = 0;
    }

    /// Permit `<![CDATA[` sections (foreign content only).
    pub fn set_cdata_allowed(&mut self, allowed: bool) {
        self.cdata_allowed = allowed;
    }

    /// Pull the next token.
    pub fn next_token(&mut self) -> Token {
        if let Some(token) = self.stashed.take() {
            return token;
        }
        match self.model {
            ContentModel::Pcdata => self.data_state(),
            ContentModel::Rcdata => self.rcdata_state(),
            ContentModel::Rawtext => self.rawtext_state(),
            ContentModel::ScriptData => self.script_data_state(),
            ContentModel::Plaintext => self.plaintext_state(),
        }
    }

    // ---------- cursor helpers ----------

    fn peek(&mut self) -> Option<char> {
        if self.peeked.is_none() {
            let ch = self.input.get(self.pos..)?.chars().next()?;
            self.peeked = Some((self.pos, ch));
        }
        self.peeked.map(|(_, ch)| ch)
    }

    fn bump(&mut self) -> Option<char> {
        if let Some((offset, ch)) = self.peeked.take() {
            self.pos = offset + ch.len_utf8();
            return Some(ch);
        }
        let ch = self.input.get(self.pos..)?.chars().next()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    /// Reset the cursor (always clears lookahead).
    fn rewind(&mut self, pos: usize) {
        self.pos = pos.min(self.input.len());
        self.peeked = None;
    }

    fn starts_with(&self, text: &str) -> bool {
        self.input
            .get(self.pos..)
            .is_some_and(|rest| rest.starts_with(text))
    }

    fn starts_with_ignore_case(&self, text: &str) -> bool {
        let Some(rest) = self.input.get(self.pos..) else {
            return false;
        };
        let end = text.len().min(rest.len());
        // Compare on a char-boundary-safe prefix.
        let mut end = end;
        while end > 0 && !rest.is_char_boundary(end) {
            end -= 1;
        }
        rest[..end].eq_ignore_ascii_case(text)
    }

    fn consume(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.input.len());
        self.peeked = None;
    }

    fn error(&mut self, kind: &'static str) {
        if self.errors.len() < 100 {
            self.errors.push(ParseError {
                offset: self.pos,
                kind,
            });
        }
    }

    /// Consume while predicate holds; returns the text.
    fn take_while(&mut self, mut pred: impl FnMut(char) -> bool) -> String {
        let mut out = String::new();
        while let Some(ch) = self.peek() {
            if !pred(ch) {
                break;
            }
            out.push(ch);
            self.bump();
        }
        out
    }

    fn is_ws(ch: char) -> bool {
        matches!(ch, ' ' | '\t' | '\n' | '\x0C')
    }

    fn skip_ws(&mut self) {
        self.take_while(Self::is_ws);
    }

    // ---------- data state ----------

    fn data_state(&mut self) -> Token {
        let mut out = String::new();
        loop {
            match self.peek() {
                None => {
                    return if out.is_empty() {
                        Token::Eof
                    } else {
                        Token::Characters(out)
                    };
                }
                Some('&') => {
                    self.bump();
                    match self.consume_char_ref(false) {
                        Some(text) => out.push_str(&text),
                        None => out.push('&'),
                    }
                }
                Some('<') => {
                    if out.is_empty() {
                        return self.tag_open_state();
                    }
                    return Token::Characters(out);
                }
                Some('\0') => {
                    self.bump();
                    self.error("unexpected-null-character");
                    out.push('�');
                }
                Some(_) => {
                    let ch = self.bump().expect("peeked");
                    out.push(ch);
                }
            }
        }
    }

    /// `&` handling shared by data and RCDATA.
    fn consume_char_ref(&mut self, in_attribute: bool) -> Option<String> {
        let rest = self.input.get(self.pos..)?;
        match consume_reference(rest, in_attribute) {
            Some((text, consumed, badness)) => {
                match badness {
                    ReferenceBadness::None => {}
                    ReferenceBadness::MissingSemicolon => {
                        self.error("missing-semicolon-after-character-reference");
                    }
                    ReferenceBadness::Invalid => {
                        self.error("invalid-character-reference");
                    }
                }
                self.consume(consumed);
                Some(text)
            }
            None => None,
        }
    }

    fn tag_open_state(&mut self) -> Token {
        debug_assert_eq!(self.peek(), Some('<'));
        self.bump();
        match self.peek() {
            Some('!') => {
                self.bump();
                self.markup_declaration_open()
            }
            Some('/') => {
                self.bump();
                self.end_tag_open()
            }
            Some(ch) if ch.is_ascii_alphabetic() => self.tag_name_state(false),
            Some('?') => {
                self.bump();
                self.error("unexpected-question-mark-instead-of-tag-name");
                self.bogus_comment()
            }
            _ => {
                self.error("invalid-first-character-of-tag-name");
                Token::Characters("<".to_owned())
            }
        }
    }

    fn end_tag_open(&mut self) -> Token {
        match self.peek() {
            Some(ch) if ch.is_ascii_alphabetic() => self.tag_name_state(true),
            Some('>') => {
                self.bump();
                self.error("missing-end-tag-name");
                self.data_state()
            }
            None => {
                self.error("eof-before-tag-name");
                Token::Characters("</".to_owned())
            }
            _ => {
                self.error("invalid-first-character-of-tag-name");
                self.bogus_comment()
            }
        }
    }

    /// Tag name: anything except whitespace, `/`, `>` (spec appends the
    /// rest, including quotes, to the name).
    fn tag_name_state(&mut self, is_end: bool) -> Token {
        let mut name = String::new();
        loop {
            match self.peek() {
                None => {
                    self.error("eof-in-tag");
                    return self.emit_tag(is_end, name.to_ascii_lowercase(), Vec::new(), false);
                }
                Some(ch) if Self::is_ws(ch) || ch == '/' || ch == '>' => break,
                Some('\0') => {
                    self.bump();
                    self.error("unexpected-null-character");
                    name.push('�');
                }
                Some(ch) => {
                    self.bump();
                    name.push(ch.to_ascii_lowercase());
                }
            }
        }
        self.before_attribute_name(is_end, name, Vec::new())
    }

    fn before_attribute_name(
        &mut self,
        is_end: bool,
        name: String,
        attributes: Vec<(String, String)>,
    ) -> Token {
        self.skip_ws();
        match self.peek() {
            None => {
                self.error("eof-in-tag");
                self.emit_tag(is_end, name, attributes, false)
            }
            Some('/') | Some('>') => self.self_closing_or_end(is_end, name, attributes),
            Some('=') => {
                self.error("unexpected-equals-sign-before-attribute-name");
                self.bump();
                self.attribute_name_state(is_end, name, attributes, "=".to_owned())
            }
            Some(_) => self.attribute_name_state(is_end, name, attributes, String::new()),
        }
    }

    fn attribute_name_state(
        &mut self,
        is_end: bool,
        tag: String,
        mut attributes: Vec<(String, String)>,
        first: String,
    ) -> Token {
        let mut name = first;
        loop {
            match self.peek() {
                None => {
                    self.error("eof-in-tag");
                    self.push_attribute(&mut attributes, name, String::new());
                    return self.emit_tag(is_end, tag, attributes, false);
                }
                Some(ch) if Self::is_ws(ch) || ch == '/' || ch == '>' || ch == '=' => break,
                Some('\0') => {
                    self.bump();
                    self.error("unexpected-null-character");
                    name.push('�');
                }
                Some('"') | Some('\'') | Some('<') => {
                    self.error("unexpected-character-in-attribute-name");
                    name.push(self.bump().expect("peeked"));
                }
                Some(_) => name.push(self.bump().expect("peeked")),
            }
        }
        let name = name.to_ascii_lowercase();
        self.skip_ws();
        match self.peek() {
            Some('=') => {
                self.bump();
                self.before_attribute_value(is_end, tag, attributes, name)
            }
            _ => {
                self.push_attribute(&mut attributes, name, String::new());
                self.before_attribute_name(is_end, tag, attributes)
            }
        }
    }

    fn before_attribute_value(
        &mut self,
        is_end: bool,
        tag: String,
        mut attributes: Vec<(String, String)>,
        name: String,
    ) -> Token {
        self.skip_ws();
        let value = match self.peek() {
            Some('"') => {
                self.bump();
                self.quoted_value('"')
            }
            Some('\'') => {
                self.bump();
                self.quoted_value('\'')
            }
            Some('>') => {
                self.error("missing-attribute-value");
                String::new()
            }
            None => {
                self.error("eof-in-tag");
                String::new()
            }
            _ => self.unquoted_value(),
        };
        self.push_attribute(&mut attributes, name, value);
        self.before_attribute_name(is_end, tag, attributes)
    }

    fn quoted_value(&mut self, quote: char) -> String {
        let mut out = String::new();
        loop {
            match self.peek() {
                None => {
                    self.error("eof-in-tag");
                    break;
                }
                Some(ch) if ch == quote => {
                    self.bump();
                    break;
                }
                Some('&') => {
                    self.bump();
                    match self.consume_char_ref(true) {
                        Some(text) => out.push_str(&text),
                        None => out.push('&'),
                    }
                }
                Some('\0') => {
                    self.bump();
                    self.error("unexpected-null-character");
                    out.push('�');
                }
                Some(_) => out.push(self.bump().expect("peeked")),
            }
        }
        out
    }

    fn unquoted_value(&mut self) -> String {
        let mut out = String::new();
        loop {
            match self.peek() {
                None => {
                    self.error("eof-in-tag");
                    break;
                }
                Some(ch) if Self::is_ws(ch) || ch == '>' => break,
                Some('&') => {
                    self.bump();
                    match self.consume_char_ref(true) {
                        Some(text) => out.push_str(&text),
                        None => out.push('&'),
                    }
                }
                Some('\0') => {
                    self.bump();
                    self.error("unexpected-null-character");
                    out.push('�');
                }
                Some('"') | Some('\'') | Some('<') | Some('=') | Some('`') => {
                    self.error("unexpected-character-in-unquoted-attribute-value");
                    out.push(self.bump().expect("peeked"));
                }
                Some(_) => out.push(self.bump().expect("peeked")),
            }
        }
        out
    }

    fn self_closing_or_end(
        &mut self,
        is_end: bool,
        name: String,
        attributes: Vec<(String, String)>,
    ) -> Token {
        if self.peek() == Some('/') {
            self.bump();
            match self.peek() {
                Some('>') => {
                    self.bump();
                    if is_end {
                        self.error("unexpected-solidus-in-tag");
                    }
                    return self.emit_tag(is_end, name, attributes, !is_end);
                }
                _ => {
                    self.error("unexpected-null-character");
                    return self.before_attribute_name(is_end, name, attributes);
                }
            }
        }
        // Must be '>'.
        self.bump();
        self.emit_tag(is_end, name, attributes, false)
    }

    /// Push an attribute; duplicates are dropped with an error (first wins).
    fn push_attribute(
        &mut self,
        attributes: &mut Vec<(String, String)>,
        name: String,
        value: String,
    ) {
        if name.is_empty() {
            return;
        }
        if attributes.iter().any(|(existing, _)| *existing == name) {
            self.error("duplicate-attribute");
            return;
        }
        attributes.push((name, value));
    }

    fn emit_tag(
        &mut self,
        is_end: bool,
        name: String,
        attributes: Vec<(String, String)>,
        self_closing: bool,
    ) -> Token {
        if is_end {
            if self_closing {
                self.error("unexpected-solidus-in-tag");
            }
            if !attributes.is_empty() {
                self.error("end-tag-with-attributes");
            }
            Token::EndTag { name }
        } else {
            Token::StartTag {
                name,
                attributes,
                self_closing,
            }
        }
    }

    // ---------- markup declarations ----------

    fn markup_declaration_open(&mut self) -> Token {
        if self.starts_with("--") {
            self.consume(2);
            return self.comment_state();
        }
        if self.starts_with_ignore_case("doctype") {
            self.consume(7);
            return self.doctype_state();
        }
        if self.cdata_allowed && self.starts_with("[CDATA[") {
            self.consume(7);
            return self.cdata_state();
        }
        self.error("incorrectly-opened-comment");
        self.bogus_comment()
    }

    fn comment_state(&mut self) -> Token {
        // `<!-->` and `<!--->` close immediately with empty text.
        if self.starts_with(">") {
            self.consume(1);
            self.error("abrupt-closing-of-empty-comment");
            return Token::Comment(String::new());
        }
        if self.starts_with("->") {
            self.consume(2);
            self.error("abrupt-closing-of-empty-comment");
            return Token::Comment(String::new());
        }
        let mut out = String::new();
        loop {
            if self.starts_with("-->") {
                self.consume(3);
                return Token::Comment(out);
            }
            if self.starts_with("--!>") {
                self.consume(4);
                self.error("incorrectly-closed-comment");
                return Token::Comment(out);
            }
            match self.bump() {
                None => {
                    self.error("eof-in-comment");
                    return Token::Comment(out);
                }
                Some('\0') => {
                    self.error("unexpected-null-character");
                    out.push('�');
                }
                Some(ch) => out.push(ch),
            }
        }
    }

    fn bogus_comment(&mut self) -> Token {
        let mut out = String::new();
        loop {
            match self.bump() {
                None | Some('>') => return Token::Comment(out),
                Some('\0') => out.push('�'),
                Some(ch) => out.push(ch),
            }
        }
    }

    fn cdata_state(&mut self) -> Token {
        let mut out = String::new();
        loop {
            if self.starts_with("]]>") {
                self.consume(3);
                break;
            }
            match self.bump() {
                None => {
                    self.error("eof-in-cdata");
                    break;
                }
                Some(ch) => out.push(ch),
            }
        }
        Token::Characters(out)
    }

    // ---------- doctype ----------

    fn doctype_state(&mut self) -> Token {
        self.skip_ws();
        let name = match self.peek() {
            Some('>') | None => {
                if self.peek().is_none() {
                    self.error("eof-in-doctype");
                } else {
                    self.error("missing-doctype-name");
                }
                None
            }
            Some(_) => Some(
                self.take_while(|ch| !Self::is_ws(ch) && ch != '>')
                    .to_ascii_lowercase(),
            ),
        };
        self.skip_ws();
        let mut public_id = None;
        let mut system_id = None;
        let mut force_quirks = name.is_none();
        if self.starts_with_ignore_case("public") {
            self.consume(6);
            match self.quoted_string() {
                Some(id) => public_id = Some(id),
                None => force_quirks = true,
            }
            self.skip_ws();
            if !self.starts_with(">") && self.peek().is_some() {
                match self.quoted_string() {
                    Some(id) => system_id = Some(id),
                    None => force_quirks = true,
                }
            }
        } else if self.starts_with_ignore_case("system") {
            self.consume(6);
            match self.quoted_string() {
                Some(id) => system_id = Some(id),
                None => force_quirks = true,
            }
        } else if name.is_some() && !self.starts_with(">") && self.peek().is_some() {
            self.error("invalid-character-sequence-after-doctype-name");
            force_quirks = true;
        }
        // Skip to `>`.
        loop {
            match self.peek() {
                None => {
                    self.error("eof-in-doctype");
                    force_quirks = true;
                    break;
                }
                Some('>') => {
                    self.bump();
                    break;
                }
                Some(_) => {
                    self.bump();
                }
            }
        }
        if name.as_deref().is_some_and(|n| n != "html") {
            force_quirks = true;
        }
        Token::Doctype {
            name,
            public_id,
            system_id,
            force_quirks,
        }
    }

    /// Quoted string after PUBLIC/SYSTEM (`None` when missing/malformed).
    fn quoted_string(&mut self) -> Option<String> {
        self.skip_ws();
        let quote = match self.peek() {
            Some('"') | Some('\'') => self.bump().expect("peeked"),
            _ => {
                self.error("missing-quote-before-doctype-public-identifier");
                return None;
            }
        };
        let mut out = String::new();
        loop {
            match self.peek() {
                None => {
                    self.error("eof-in-doctype");
                    return None;
                }
                Some('>') => {
                    self.error("abrupt-doctype-public-identifier");
                    return None;
                }
                Some(ch) if ch == quote => {
                    self.bump();
                    return Some(out);
                }
                Some('\0') => {
                    self.bump();
                    self.error("unexpected-null-character");
                    out.push('�');
                }
                Some(_) => out.push(self.bump().expect("peeked")),
            }
        }
    }

    // ---------- raw text models ----------

    /// Probe for an appropriate end tag with `<` already consumed.
    /// Success emits the end tag (model back to PCDATA); failure rewinds
    /// to just after `<` so the caller emits `<` literally and continues.
    fn try_appropriate_end_tag(&mut self, expected: &str) -> EndTagAttempt {
        if self.peek() != Some('/') {
            return EndTagAttempt::NotATag;
        }
        let after_lt = self.pos;
        self.bump();
        let name: String = self
            .take_while(|ch| ch.is_ascii_alphanumeric())
            .to_ascii_lowercase();
        if name.is_empty() || name != expected {
            self.rewind(after_lt);
            return EndTagAttempt::NotATag;
        }
        // Attributes on raw end tags are skipped per spec (with an error).
        loop {
            self.skip_ws();
            match self.peek() {
                Some('>') => {
                    self.bump();
                    self.set_pcdata();
                    return EndTagAttempt::Tag(Token::EndTag { name });
                }
                Some('/') => {
                    let save = self.pos;
                    self.bump();
                    if self.peek() == Some('>') {
                        self.bump();
                        self.set_pcdata();
                        return EndTagAttempt::Tag(Token::EndTag { name });
                    }
                    self.rewind(save);
                    // Fall through to attribute skipping.
                    self.skip_raw_attribute();
                }
                None => {
                    self.rewind(after_lt);
                    return EndTagAttempt::NotATag;
                }
                Some(_) => self.skip_raw_attribute(),
            }
        }
    }

    /// Skip one attribute-ish chunk inside a raw end tag probe.
    fn skip_raw_attribute(&mut self) {
        // Name.
        self.take_while(|ch| !Self::is_ws(ch) && ch != '/' && ch != '>' && ch != '=');
        self.skip_ws();
        if self.peek() == Some('=') {
            self.bump();
            self.skip_ws();
            match self.peek() {
                Some('"') | Some('\'') => {
                    let quote = self.bump().expect("peeked");
                    loop {
                        match self.peek() {
                            None => return,
                            Some(ch) if ch == quote => {
                                self.bump();
                                return;
                            }
                            Some(_) => {
                                self.bump();
                            }
                        }
                    }
                }
                _ => {
                    self.take_while(|ch| !Self::is_ws(ch) && ch != '/' && ch != '>');
                }
            }
        }
        self.error("end-tag-with-attributes");
    }

    /// Shared `<` handling for RCDATA/RAWTEXT/unescaped script.
    /// `<` is already consumed. Either returns the end tag, or pushes the
    /// literal `<` plus any rewound text and reports `None`.
    fn raw_less_than(&mut self, out: &mut String, expected: &str) -> Option<Token> {
        match self.try_appropriate_end_tag(expected) {
            EndTagAttempt::Tag(token) => {
                if out.is_empty() {
                    Some(token)
                } else {
                    self.stashed = Some(token);
                    None
                }
            }
            EndTagAttempt::NotATag => {
                out.push('<');
                None
            }
        }
    }

    fn rcdata_state(&mut self) -> Token {
        if let Some(token) = self.stashed.take() {
            return token;
        }
        let expected = self.last_start_tag.clone().unwrap_or_default();
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return self.split_characters(out),
                Some('&') => {
                    self.bump();
                    match self.consume_char_ref(false) {
                        Some(text) => out.push_str(&text),
                        None => out.push('&'),
                    }
                }
                Some('<') => {
                    self.bump();
                    if let Some(token) = self.raw_less_than(&mut out, &expected) {
                        return token;
                    }
                    if !out.is_empty() && self.stashed.is_some() {
                        return self.split_characters(out);
                    }
                }
                Some('\0') => {
                    self.bump();
                    self.error("unexpected-null-character");
                    out.push('�');
                }
                Some(_) => out.push(self.bump().expect("peeked")),
            }
        }
    }

    fn rawtext_state(&mut self) -> Token {
        if let Some(token) = self.stashed.take() {
            return token;
        }
        let expected = self.last_start_tag.clone().unwrap_or_default();
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return self.split_characters(out),
                Some('<') => {
                    self.bump();
                    if let Some(token) = self.raw_less_than(&mut out, &expected) {
                        return token;
                    }
                    if !out.is_empty() && self.stashed.is_some() {
                        return self.split_characters(out);
                    }
                }
                Some('\0') => {
                    self.bump();
                    self.error("unexpected-null-character");
                    out.push('�');
                }
                Some(_) => out.push(self.bump().expect("peeked")),
            }
        }
    }

    fn split_characters(&self, out: String) -> Token {
        if out.is_empty() {
            Token::Eof
        } else {
            Token::Characters(out)
        }
    }

    fn script_data_state(&mut self) -> Token {
        if let Some(token) = self.stashed.take() {
            return token;
        }
        let expected = self.last_start_tag.clone().unwrap_or_default();
        let mut out = String::new();
        let mut escaped = false;
        let mut dashes = 0u8;
        loop {
            match self.peek() {
                None => return self.split_characters(out),
                Some('<') => {
                    self.bump();
                    if !escaped && self.starts_with("!--") {
                        self.consume(3);
                        out.push_str("<!--");
                        escaped = true;
                        dashes = 0;
                        continue;
                    }
                    if escaped {
                        // Double-escape bookkeeping before any end-tag attempt.
                        if self.starts_with("script") && boundary_after(self.after(6)) {
                            self.escaped_depth += 1;
                            out.push('<');
                            continue;
                        }
                        if self.starts_with("/script") && boundary_after(self.after(7)) {
                            self.escaped_depth = self.escaped_depth.saturating_sub(1);
                            out.push('<');
                            continue;
                        }
                        // A real `</script>` ends the dance only at depth 0.
                        if self.escaped_depth == 0 {
                            if let Some(token) = self.script_end_tag(&mut out) {
                                return token;
                            }
                            if !out.is_empty() && self.stashed.is_some() {
                                return self.split_characters(out);
                            }
                            continue;
                        }
                        out.push('<');
                        dashes = 0;
                        continue;
                    }
                    if let Some(token) = self.raw_less_than(&mut out, &expected) {
                        return token;
                    }
                    if !out.is_empty() && self.stashed.is_some() {
                        return self.split_characters(out);
                    }
                }
                Some('-') if escaped => {
                    self.bump();
                    out.push('-');
                    dashes += 1;
                    if dashes >= 2 && self.peek() == Some('>') {
                        self.bump();
                        out.push('>');
                        escaped = false;
                        dashes = 0;
                    }
                }
                Some('\0') => {
                    self.bump();
                    self.error("unexpected-null-character");
                    out.push('�');
                    dashes = 0;
                }
                Some(_) => {
                    let ch = self.bump().expect("peeked");
                    if ch != '-' {
                        dashes = 0;
                    }
                    out.push(ch);
                }
            }
        }
    }

    /// `</script …>` probe honoring the escape depth.
    fn script_end_tag(&mut self, out: &mut String) -> Option<Token> {
        match self.try_appropriate_end_tag("script") {
            EndTagAttempt::Tag(token) => {
                self.escaped_depth = 0;
                if out.is_empty() {
                    Some(token)
                } else {
                    self.stashed = Some(token);
                    None
                }
            }
            EndTagAttempt::NotATag => {
                out.push('<');
                None
            }
        }
    }

    /// Text after skipping `n` bytes (empty when truncated or mid-char).
    fn after(&self, n: usize) -> &str {
        self.input.get(self.pos + n..).unwrap_or("")
    }

    fn plaintext_state(&mut self) -> Token {
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return self.split_characters(out),
                Some('\0') => {
                    self.error("unexpected-null-character");
                    out.push('�');
                }
                Some(ch) => out.push(ch),
            }
        }
    }
}

/// Word boundary after `<script` / `</script` inside escaped script.
fn boundary_after(rest: &str) -> bool {
    matches!(
        rest.chars().next(),
        Some(' ' | '\t' | '\n' | '\x0C' | '/' | '>')
    )
}
