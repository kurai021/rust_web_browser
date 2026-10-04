# 06 — CSS engine (parser, cascade, values)

> `css` crate. In-house engine from scratch.

## 6.1 Goal

Parse CSS and compute per-element computed style — enough for daily-driver rendering, including flex/grid in an advanced phase.

## 6.2 Scope (specs to follow)

- **CSS Syntax 3:** tokenizer + parser (stylesheet, qualified rules, at-rules, declarations, `!important`, per-rule error recovery).
- **Selectors 4 (v1 subset):** type, universal, id, class, attribute (`[=~=|=^=$*=]`), structural pseudo-classes (`:first-child`, `:last-child`, `:nth-child()`, `:not()`, `:root`, `:empty`, `:link`/`:visited` with privacy restrictions), combinators (` `, `>`, `+`, `~`). Pseudo-elements `::before`/`::after`/`::first-line`/`::first-letter`/`::selection`/marker (box generation in layout).
- **Cascade 5 + Inheritance:** origin (UA, user, author), `!important`, specificity, order, inheritance, `initial`/`inherit`/`unset`/`revert`.
- **Values and units:** `px/em/rem/%/vw/vh/vmin/vmax`, colors (`#hex`, `rgb()`, `rgba()`, `hsl()`, named, `currentColor`, `transparent`), basic `calc()`/`clamp()`/`min()`/`max()`, `var()` (custom properties) in an advanced but committed phase.
- **Basic box/background/text/font:** `display`, `margin`/`padding`/`border`, `width`/`height`/`min`/`max`, `box-sizing`, `background-color`/`image` (image = fetch, no complex gradients in v1 except basic linear-gradient), `color`, `font-family`/`size`/`weight`/`style`/`line-height`/`text-align`/`decoration`/`transform` (uppercase/lowercase)/`white-space`/`overflow`/`visibility`.
- **Media Queries 4:** `width`/`height`, `min-`/`max-`, `orientation`, `prefers-color-scheme`, `print`/`screen` (print renders as screen in v1).
- **At-rules:** `@media`, `@import` (top-only, fetched with a cap), `@font-face` (woff/woff2/ttf, `font-display: swap`), basic `@supports`, `@keyframes` (for late-phase transitions/animations; parse even if not animated at first).
- **UA stylesheet:** embedded, documented default sheet (html.css-like).

Not in initial v1 (but not to be confused with "never"):

- Complex `position: sticky`, `subgrid`, container queries, `has()`, `layers (@layer)`? `@layer` IS parsed (cascade layers are part of Cascade 5) but applied in simplified form at first.
- Smooth animations/transitions: parse and apply the final/instant state first; interpolation in a late phase.

## 6.3 Fixed API

```rust
// crates/css
pub struct Stylesheet { /* rules */ }
pub fn parse_stylesheet(css: &str, base_url: Option<&Url>) -> Stylesheet;
pub struct Cascade { ua: Stylesheet, user: Stylesheet }
impl Cascade {
  pub fn compute(&self, dom: &Document, author_sheets: &[Stylesheet]) -> ComputedStyles;
  pub fn matching_rules(&self, el: &Element) -> Vec<MatchedRule>;
}
pub struct ComputedStyles; // map ElementId -> ComputedStyle
```

- Immutable, shareable (`Arc`) computed styles. Invalidation on DOM mutation (dirty set from `dom_bindings`).
- Indexed selectors (by id/class/tag) to avoid naive O(N*M) once mature; v1 may be naive but with a documented cap.

## 6.4 Robustness

- Never panic on arbitrary CSS. A malformed rule is dropped, a malformed declaration ignored, keeping the rest.
- Caps: 5 MB stylesheet, `@import` depth 5, `url()` http/https/data only (data capped at 2 MB), no fetching other schemes.
- `data:` URLs allowed only for small fonts/images, with strict MIME sniffing.

## 6.5 Acceptance criteria

- [ ] WPT `css/css-syntax`, `css/selectors`, `css/cascade` subsets with measured pass rate.
- [ ] Specificity, `!important`, inheritance and `inherit` correct on custom fixtures.
- [ ] `@media (max-width)` changes layout on window resize.
- [ ] CSS parser fuzzed with no panics.
