# 05 — HTML5 engine (in-house parser)

> `html` crate. In-house engine from scratch. `html5ever` only as a test oracle, never as a product dependency.

## 5.1 Goal

Turn bytes → chars → tokens → WHATWG-compliant HTML DOM tree (tokenizer + tree construction), with defined error handling, incremental/streaming parsing, and hostile-content resistance.

## 5.2 Exact scope

Included:

- Encoding detection: BOM, `<meta charset>`, `Content-Type` header, UTF-8 fallback. Use `encoding_rs`. No exotic encodings beyond what `encoding_rs` provides.
- WHATWG tokenizer: DOCTYPE, open/close tags, attributes (quoted/unquoted), comments, entities (`&amp;` etc.), CDATA in SVG/Math, RCDATA/RAWTEXT (`<script>`, `<style>`, `<textarea>`, `<title>`).
- Tree construction: insertion modes, foster parenting (tables), form pointer, adoption agency (`b/i/a` formatting), fragments, `<template>`, auto-closing of `p/li/td/etc.`, implicit `<html>/<head>/<body>` handling.
- DOM core here (no layout/style): `Node`, `Document`, `DocumentType`, `Element`, `Text`, `Comment`, `DocumentFragment`, attributes, namespaces (HTML/SVG/MathML), indexed `id`/`class`.
- Minimal scripting hooks: `<script src/defer/async>` reports events upward; the parser pauses/resumes around `document.write`? `document.write` supported only in limited form (see 5.5).
- Incremental parsing: `push(bytes)` + `finish()`, pause on blocking `<script>`, resume. Capable of progressive rendering (first paint before EOF).

Not included here (covered by other docs):

- Styles/layout/paint (docs 06–07). JS/DOM APIs (docs 08–09). Subresource fetching (doc 04, orchestrated by shell/navigation).

## 5.3 Fixed API (so nothing is assumed)

```rust
// crates/html
pub struct Parser { /* ... */ }
pub fn parse_full(bytes: &[u8], url: &Url, opts: ParseOpts) -> Document;
impl Parser {
  pub fn new(url: Url, sink: DomSink, opts: ParseOpts) -> Self;
  pub fn push(&mut self, chunk: &[u8]);
  pub fn suspend_for_script(&mut self);
  pub fn resume(&mut self);
  pub fn finish(self) -> Document;
}
pub struct ParseOpts {
  pub max_nodes: usize,      // default 500_000
  pub max_depth: usize,      // default 512
  pub max_attr_len: usize,   // default 64KB
  pub scripting_enabled: bool,
}
```

- All parsing returns a `Document`, never panics on arbitrary input. Errors → `ParseError` list with positions, no abort (except limits → abort with a typed error like `TooManyNodes`).
- `DomSink` is a trait letting `dom_bindings` observe mutations (to invalidate style/layout).

## 5.4 Robustness rules

- Default limits above, configurable. Exceeding them = "document too complex" error page, not OOM.
- Bounded depth and entity expansion (billion-laughs style).
- Mandatory fuzzing (`cargo fuzz`): tokenizer + tree builder with no `panic`/`unwrap` on input paths.
- Tests: fixtures from WPT `html/syntax` + custom cases (broken tables, `a` nesting, unclosed `p`, `script` with `</script>` inside strings).

## 5.5 Locked decisions

- `document.write`: supported only during initial synchronous parsing; if called after `DOMContentLoaded` → ignore + console warning (no destructive implicit re-parse in v1).
- `<template>` contents: parsed but inert (no subresource fetch until cloned).
- SVG/MathML: parse namespaces and attributes, build the DOM; full SVG rendering is a late phase (as image: yes; as interactive DOM: not in initial v1). No inventing SVG layout in this crate.
- Custom elements (`customElements.define`) are JS/DOM scope (doc 09), not this parser's — except that hyphenated-tag parsing must not break.

## 5.6 Acceptance criteria

- [ ] Passes the WPT html/syntax subset defined in doc 13 with no crashes and a documented, growing pass rate.
- [ ] `example.com`, Wikipedia, Hacker News parse and show navigable text in the minimal shell (Phase 2).
- [ ] Benchmark: parsing 1 MB of HTML < 200 ms on average hardware (initial goal, non-blocking).
- [ ] 1h fuzzing with no panics/OOM.
