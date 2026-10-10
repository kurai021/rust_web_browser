//! Parser driver (plan/05 §5.2–§5.3).
//!
//! Owns decoding (BOM → `<meta>` prescan → UTF-8 fallback), drives the
//! tokenizer/tree-builder loop with content-model synchronization, and
//! enforces the resource budgets. Suspension pauses token consumption at
//! token boundaries; buffered input waits for `resume()`.

use url::Url;

use crate::dom::Document;
use crate::tokenizer::{Token, Tokenizer};
use crate::tree::{Step, TreeBuilder};

/// Encoding labels the prescan recognizes (ASCII-case-insensitive).
fn find_meta_charset(head: &[u8]) -> Option<String> {
    // Scan the first 1024 bytes for <meta charset=…> / http-equiv.
    let len = head.len().min(1024);
    let text = std::str::from_utf8(&head[..len]).unwrap_or("");
    let lower = text.to_ascii_lowercase();
    let mut search_from = 0usize;
    while let Some(meta_at) = lower[search_from..].find("<meta") {
        let tag_start = search_from + meta_at;
        let tag_end = lower[tag_start..]
            .find('>')
            .map(|pos| tag_start + pos)
            .unwrap_or(len);
        let tag = &lower[tag_start..tag_end];
        if let Some(charset) = attr_value(tag, "charset") {
            return Some(charset);
        }
        search_from = tag_end;
    }
    None
}

/// `name=value` / `name="value"` / `name='value'` inside a lowercased tag.
fn attr_value(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    while let Some(eq_at) = rest.find(name) {
        let after_name = &rest[eq_at + name.len()..];
        // Name must end here (not a prefix of a longer name).
        if eq_at > 0 {
            let prev = rest.as_bytes()[eq_at - 1];
            if prev.is_ascii_alphanumeric() || prev == b'-' || prev == b'_' {
                rest = after_name;
                continue;
            }
        }
        let mut value = after_name.trim_start_matches([' ', '\t', '\n', '\x0C', '\r']);
        if let Some(stripped) = value.strip_prefix('=') {
            value = stripped.trim_start_matches([' ', '\t', '\n', '\x0C', '\r']);
            if let Some(quoted) = value.strip_prefix('"') {
                return quoted.find('"').map(|end| quoted[..end].to_owned());
            }
            if let Some(quoted) = value.strip_prefix('\'') {
                return quoted.find('\'').map(|end| quoted[..end].to_owned());
            }
            let end = value
                .find([' ', '\t', '\n', '\x0C', '\r', '>'])
                .unwrap_or(value.len());
            if end > 0 {
                return Some(value[..end].to_owned());
            }
        }
        rest = after_name;
    }
    None
}

/// Pick a decoding for the buffered head of the document.
fn detect_encoding(head: &[u8]) -> &'static encoding_rs::Encoding {
    if head.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return encoding_rs::UTF_8;
    }
    if head.starts_with(&[0xFF, 0xFE]) {
        return encoding_rs::UTF_16LE;
    }
    if head.starts_with(&[0xFE, 0xFF]) {
        return encoding_rs::UTF_16BE;
    }
    if let Some(label) = find_meta_charset(head) {
        if let Some(encoding) = encoding_rs::Encoding::for_label(label.as_bytes()) {
            return encoding;
        }
    }
    encoding_rs::UTF_8
}

/// Incremental HTML parser (fixed API from plan/05 §5.3).
pub struct Parser {
    url: Url,
    opts: crate::ParseOpts,
    sink: Box<dyn DomSink>,
    raw: Vec<u8>,
    decoder: Option<encoding_rs::Decoder>,
    encoding_determined: bool,
    finished_input: bool,
    suspended: bool,
    eof_processed: bool,
    tree: Option<TreeBuilder>,
    tokenizer: Tokenizer,
    /// Fragment context element (spec fragment case).
    fragment_context: Option<String>,
    previous_cr: bool,
}

impl Parser {
    /// New parser. Nothing is parsed until input arrives.
    #[must_use]
    pub fn new(url: Url, sink: Box<dyn DomSink>, opts: crate::ParseOpts) -> Self {
        let mut tokenizer = Tokenizer::new();
        tokenizer.begin_stream();
        Self {
            url,
            opts,
            sink,
            raw: Vec::new(),
            decoder: None,
            encoding_determined: false,
            finished_input: false,
            suspended: false,
            eof_processed: false,
            tree: None,
            tokenizer,
            fragment_context: None,
            previous_cr: false,
        }
    }

    /// New parser for the fragment case under `context_tag`.
    #[must_use]
    pub fn new_fragment(
        url: Url,
        sink: Box<dyn DomSink>,
        opts: crate::ParseOpts,
        context_tag: &str,
    ) -> Self {
        let mut parser = Self::new(url, sink, opts);
        parser.fragment_context = Some(context_tag.to_ascii_lowercase());
        parser
    }

    /// Append input bytes (streaming; cheap for few large pushes).
    pub fn push(&mut self, chunk: &[u8]) {
        if self.finished_input {
            return;
        }
        self.raw.extend_from_slice(chunk);
        self.decode_available(false);
        if !self.suspended {
            self.drive();
        }
    }

    /// A consistent partial DOM without injecting EOF or consuming the parser.
    /// Used for progressive painting; encoding detection still waits for the
    /// prescribed head prescan. Detached templates remain inert.
    pub fn snapshot(&self) -> Option<Document> {
        if !self.encoding_determined {
            return None;
        }
        let mut document = self.tree.as_ref()?.document().clone();
        enforce_attr_budget(&mut document, self.opts.max_attr_len);
        Some(document)
    }
    pub fn is_suspended(&self) -> bool {
        self.suspended
    }
    /// A blocking-script host returns the mutated arena before parsing resumes.
    /// Node identities stay stable for the builder's open-element stack.
    pub fn replace_document(&mut self, document: Document) {
        if let Some(tree) = &mut self.tree {
            *tree.document_mut() = document;
        }
    }

    /// Pause token consumption (blocking `<script>` with scripting on).
    /// Buffered input waits; already-emitted tokens are unaffected.
    pub fn suspend_for_script(&mut self) {
        self.suspended = true;
    }

    /// Resume after the script ran.
    pub fn resume(&mut self) {
        if !self.suspended {
            return;
        }
        self.suspended = false;
        self.drive();
    }

    /// End of input: decode the tail, drain all tokens, return the document.
    #[must_use]
    pub fn finish(mut self) -> Document {
        if !self.finished_input {
            self.finished_input = true;
            self.decode_available(true);
            self.tokenizer.finish_input();
        }
        self.suspended = false;
        self.drive();
        self.into_document()
    }
    /// Signal real EOF without consuming the parser, retaining script pauses.
    pub fn end_input(&mut self) {
        if !self.finished_input {
            self.finished_input = true;
            self.decode_available(true);
            self.tokenizer.finish_input();
        }
        self.drive();
    }

    /// Decode whatever is buffered (determining the encoding first).
    fn decode_available(&mut self, final_chunk: bool) {
        if !self.encoding_determined && (final_chunk || self.raw.len() >= 1024) {
            let encoding = detect_encoding(&self.raw);
            // Strip a BOM the decoder would otherwise expose.
            if *encoding == *encoding_rs::UTF_8 && self.raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
                self.raw.drain(..3);
            }
            self.decoder = Some(encoding.new_decoder());
            self.encoding_determined = true;
        }
        if !self.encoding_determined {
            return;
        }
        let decoder = self.decoder.as_mut().expect("set above");
        let input = std::mem::take(&mut self.raw);
        // Streaming decode; malformed sequences become U+FFFD. An incomplete
        // tail is kept back for the next push (never dropped).
        let mut output = String::with_capacity(input.len() + input.len() / 8 + 32);
        let (_, read, _) = decoder.decode_to_string(&input, &mut output, final_chunk);
        self.raw.extend_from_slice(&input[read..]);
        // WHATWG preprocessing: CRLF/CR → LF.
        let previous_cr = self.previous_cr;
        if !output.is_empty() {
            self.previous_cr = output.ends_with('\r');
        }
        let output = if previous_cr {
            output.strip_prefix('\n').unwrap_or(&output)
        } else {
            &output
        };
        let normalized = normalize_newlines(output);
        self.tokenizer.feed(&normalized);
    }

    /// Lazily create the tree builder on first drive.
    fn ensure_tree(&mut self) {
        if self.tree.is_none() {
            let mut doc = Document::new();
            doc.url = Some(self.url.clone());
            let mut tree = match &self.fragment_context {
                Some(context) => {
                    TreeBuilder::new_fragment(doc, context, self.opts.scripting_enabled)
                }
                None => TreeBuilder::new(doc, self.opts.scripting_enabled),
            };
            tree.set_limits(self.opts.max_nodes, self.opts.max_depth);
            self.tree = Some(tree);
        }
    }

    /// Consume tokens until input runs out, suspension, or a fatal limit.
    /// The tokenizer persists across calls (cursor + model + stash), so
    /// suspend/resume and multi-push parsing stay correct.
    fn drive(&mut self) {
        self.ensure_tree();
        loop {
            if self.suspended {
                break;
            }
            if self.tree.as_ref().expect("created above").stopped() {
                break;
            }
            // Sync CDATA permission with tree state before pulling.
            sync_tokenizer_pre(
                self.tree.as_ref().expect("created above"),
                &mut self.tokenizer,
            );
            let token = self.tokenizer.next_token();
            // Sync the content model for the token after a start tag.
            sync_tokenizer_post(
                self.tree.as_ref().expect("created above"),
                &mut self.tokenizer,
                &token,
            );
            self.sink.note_token();
            // Merge tokenizer errors into the document.
            {
                let tree = self.tree.as_mut().expect("created above");
                for err in self.tokenizer.errors.drain(..) {
                    tree.document_mut().errors.push(err);
                }
            }
            match token {
                Token::Eof => {
                    if !self.finished_input || self.eof_processed {
                        // Park: more input may arrive via push().
                        break;
                    }
                    self.eof_processed = true;
                    let tree = self.tree.as_mut().expect("created above");
                    tree.process_token(Token::Eof);
                    break;
                }
                _ => {
                    let tree = self.tree.as_mut().expect("created above");
                    match tree.process_token(token) {
                        Step::Continue => {}
                        Step::SuspendForScript => {
                            self.sink.note_script();
                            self.suspended = true;
                        }
                    }
                }
            }
        }
    }

    fn into_document(mut self) -> Document {
        let tree = self.tree.take().expect("drive created it");
        let is_fragment = self.fragment_context.is_some();
        if is_fragment {
            let mut doc = tree.finish_fragment();
            enforce_attr_budget(&mut doc, self.opts.max_attr_len);
            doc
        } else {
            let mut doc = tree.finish();
            enforce_attr_budget(&mut doc, self.opts.max_attr_len);
            doc
        }
    }
}

/// Sync the tokenizer's CDATA permission with tree state (before pulling).
fn sync_tokenizer_pre(tree: &TreeBuilder, tokenizer: &mut Tokenizer) {
    tokenizer.set_cdata_allowed(tree.current_node_is_foreign());
}

/// Sync the content model after a start tag (before its content is read).
/// Foreign `<style>`/`<title>`-likes stay PCDATA; HTML ones switch models.
fn sync_tokenizer_post(tree: &TreeBuilder, tokenizer: &mut Tokenizer, token: &Token) {
    if let Token::StartTag {
        name,
        self_closing: false,
        ..
    } = token
    {
        const RAW: [&str; 10] = [
            "title",
            "textarea",
            "style",
            "iframe",
            "noembed",
            "noframes",
            "noscript",
            "xmp",
            "script",
            "plaintext",
        ];
        // `noscript` is raw text only when scripting is enabled.
        let raw = RAW.contains(&name.as_str()) && (name != "noscript" || tree.scripting_enabled());
        if raw && !tree.current_node_is_foreign() {
            tokenizer.set_state_for_tag(name);
        } else {
            tokenizer.set_pcdata();
        }
    }
}

/// Normalize CRLF/CR to LF (spec input preprocessing).
fn normalize_newlines(input: &str) -> String {
    if !input.contains('\r') {
        return input.to_owned();
    }
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(ch);
        }
    }
    out
}

/// Enforce the attribute-length budget on a finished document (defense in
/// depth; overlong values are dropped here with an error).
fn enforce_attr_budget(doc: &mut Document, max_attr_len: usize) {
    use crate::dom::{NodeData, NodeId};
    fn visit(doc: &mut Document, id: NodeId, max: usize) {
        let children = doc.children_of(id);
        let overlong = match &mut doc.nodes_mut(id).data {
            NodeData::Element(el) => {
                let mut overlong = false;
                for attr in &mut el.attributes {
                    if attr.value.len() > max {
                        attr.value.truncate(max);
                        overlong = true;
                    }
                }
                overlong
            }
            _ => false,
        };
        if overlong {
            doc.error(0, "attribute-value-too-long");
        }
        for child in children {
            visit(doc, child, max);
        }
    }
    let root = doc.root();
    visit(doc, root, max_attr_len);
}

/// Sink for parser lifecycle events (script boundaries, token flow).
/// All methods have no-op defaults; the shell overrides what it needs.
pub trait DomSink: Send {
    /// Called once per pulled token (progress/heartbeat hook).
    fn note_token(&mut self) {}
    /// Called when a blocking script suspends parsing.
    fn note_script(&mut self) {}
}

/// No-op sink for tests and headless use.
#[derive(Debug, Default)]
pub struct NullSink;

impl DomSink for NullSink {}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8]) -> Document {
        crate::parse_full(
            bytes,
            &Url::parse("https://example.com/").unwrap(),
            crate::ParseOpts::default(),
        )
    }

    #[test]
    fn hello_world() {
        let doc = parse(b"<!DOCTYPE html><title>T</title><p>Hello</p>");
        assert_eq!(doc.title().as_deref(), Some("T"));
        assert_eq!(
            doc.body().map(|id| doc.text_content(id)).as_deref(),
            Some("Hello")
        );
    }

    #[test]
    fn utf16_bom_decodes() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "<p>hi</p>".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let doc = parse(&bytes);
        // Raw UTF-16 bytes must decode to the same tree, not mojibake.
        let body = doc.body().expect("body");
        assert_eq!(doc.text_content(body), "hi");
    }

    #[test]
    fn meta_charset_wins_over_utf8() {
        let text = "<meta charset=\"windows-1252\"><p>\u{93}hi\u{94}</p>";
        let latin: Vec<u8> = text
            .chars()
            .map(|ch| match ch {
                '\u{93}' => 0x93,
                '\u{94}' => 0x94,
                other => {
                    assert!(other.is_ascii());
                    other as u8
                }
            })
            .collect();
        let doc = crate::parse_full(
            &latin,
            &Url::parse("https://example.com/").unwrap(),
            crate::ParseOpts::default(),
        );
        let body = doc.body().expect("body");
        assert!(doc.text_content(body).contains("hi"));
    }

    #[test]
    fn suspend_resume_roundtrip() {
        let url = Url::parse("https://example.com/").unwrap();
        let mut parser = Parser::new(url, Box::new(NullSink), crate::ParseOpts::default());
        parser.push(b"<p>one</p>");
        parser.suspend_for_script();
        parser.push(b"<p>two</p>");
        parser.resume();
        let doc = parser.finish();
        let body = doc.body().expect("body");
        assert_eq!(doc.text_content(body), "onetwo");
    }

    /// Deterministic fuzz smoke (stable CI): PRNG-mutated inputs must never
    /// panic, hang (bounded size) or exceed budgets. Nightly `cargo fuzz`
    /// runs the same entry point with coverage guidance (see `fuzz/`).
    #[test]
    fn fuzz_smoke_no_panic_no_hang() {
        let seeds: &[&[u8]] = &[
            b"<p>hello",
            b"<table><tr><td>",
            b"<script>if(a<b){}</script>",
            b"<svg><g>",
            b"<a href=\"&amp;\">",
            b"<!DOCTYPE html>",
            b"<template><div>",
            b"\xff\xfe<\x00p\x00>",
            b"<p>&notanentity;&x;",
            b"<select><option>",
        ];
        let mut state: u64 = 0x1234_5678_9ABC_DEF0;
        let mut next_byte = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 33) as u8
        };
        for iteration in 0..2000 {
            let seed = seeds[iteration % seeds.len()];
            let mut input = seed.to_vec();
            // 1-8 random mutations.
            for _ in 0..1 + (next_byte() % 8) as usize {
                if input.is_empty() {
                    input.push(next_byte());
                    continue;
                }
                match next_byte() % 4 {
                    0 => input.push(next_byte()),
                    1 => {
                        input.remove(next_byte() as usize % input.len());
                    }
                    2 => {
                        let pos = next_byte() as usize % input.len();
                        input[pos] = next_byte();
                    }
                    _ => {
                        let pos = next_byte() as usize % (input.len() + 1);
                        input.insert(pos, next_byte());
                    }
                }
            }
            input.truncate(4096);
            let url = Url::parse("https://example.com/").unwrap();
            let opts = crate::ParseOpts {
                max_nodes: 10_000,
                max_depth: 64,
                ..crate::ParseOpts::default()
            };
            let mut parser = Parser::new(url, Box::new(NullSink), opts);
            // Split pushes exercise incremental paths.
            let mid = input.len() / 2;
            parser.push(&input[..mid]);
            parser.push(&input[mid..]);
            let doc = parser.finish();
            assert!(doc.node_count() <= 10_001, "budget respected");
        }
    }
}
