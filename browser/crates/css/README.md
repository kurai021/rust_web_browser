# CSS engine — Phase 3

In-house Rust implementation of `plan/06-css.md`. The existing tokenizer
has been extended, not replaced with an external engine.

## Public entry points

```rust
let sheet = css::parse_stylesheet("p { color: green; }", document.url.as_ref());
let cascade = css::Cascade {
    environment: css::Environment { width: 800.0, height: 600.0, dark: false },
    ..css::Cascade::default()
};
let styles = cascade.compute(&document, &[sheet]);
if let Some(style) = styles.get(element_id) {
    let color = style.get_property_value("color"); // internal getComputedStyle contract
}
```

The DOM uses `NodeId`, so matching receives a document and ID rather than
the framework's illustrative standalone `Element`. Styles are immutable
`Arc<ComputedStyle>` entries. Edges use top/right/bottom/left order.

## Modules

- `token`: Syntax 3 preprocessing, tokens, escapes, functions and URL states.
- `syntax`: balanced rules/declarations, `!important`, bounded error recovery.
- `selectors`: selector lists, specificity, structural/functional selectors,
  attributes, combinators and pseudo-element targets; visited history stays private.
- `values` / `style`: colors, relative lengths and dimension-checked math,
  property grammars, shorthands and typed computed values.
- `conditions`: media queries and basic declaration-based `@supports`.
- `stylesheet`: nested rules, imports, layers, font-face and keyframe metadata.
- `cascade`: origins, importance, layers, inline declarations, inheritance,
  wide keywords, custom properties and cycle-safe `var()` substitution.
- `ua.css`: embedded local user-agent sheet; no network assets.

CSS performs no I/O. `shell/page.rs` loads sheets in document order and fills
import nodes. `shell/fonts.rs` loads/decompresses fonts after fallback paint.
Resize/theme changes recompute styles on the existing background worker.

## Phase boundaries and current coverage

Phase 3 paints computed colors, fonts, inline typography, backgrounds,
padding, simple borders and vertical block margins in the Phase 2 viewport.
Nested flow layout, margin collapsing, generated pseudo-element boxes,
background images/gradient rasterization and advanced positioning are the
subsequent `layout`/`paint` phases. Parsed keyframes do not animate yet.

The grammar and selector matcher cover the tested subset in `tests/wpt`;
this is not a full CSS/WPT conformance claim. Nested cascade-layer precedence
is currently flattened in declaration order. Full nested-layer edge cases and
less common value/selector grammar cases stay in compatibility testing.

## Resource bounds

- 5 MiB per sheet; 50,000 rules; 64 nested blocks.
- 32 nested selector/math/variable expressions; 128 combinator steps;
  20,000 matcher steps per selector; 8,192 substituted tokens.
- Shell: import depth 5, 64 CSS requests, 16 MiB aggregate CSS per page.
- Fonts: 8 MiB encoded/decoded font limit, 16 active faces, 16 source fallbacks
  per face and 64 font requests per page; SFNT/WOFF1 and single-font WOFF2.
- Remote schemes and MIME are checked; only explicit page resources are fetched.

## Verification

```sh
cargo test -p css
cargo test -p shell --test css_loading
# From browser/crates/css:
cargo +nightly fuzz run parse -- -max_total_time=300 -max_len=4096 -timeout=3
```

`tests/wpt_adapted.rs`: 52 actual WPT assertions adapted to the Rust API,
with source paths, retained licenses and explicit adaptation notes.
