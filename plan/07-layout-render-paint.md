# 07 — Layout, paint and compositing

> `layout` + `paint` crates. Consume the DOM + computed styles. Produce pixels in a `winit`/`wgpu` window.

## 7.1 Goal

Turn render tree → boxes → positions → draw list → GPU/software frames, with correct text and images and smooth scrolling.

## 7.2 Render tree

- Built from DOM + `ComputedStyles`: drop `display:none`, generate `::before`/`::after` pseudo-elements, anonymous boxes (block/inline wrapping), `img`/`video`/`input` as replaced elements.
- Each node: stable `ElementId` for JS-driven invalidation.

## 7.3 Layout (stages)

**Stage A (MVP):** block + inline flow.

- Block: margin collapsing, `width: auto`, centering `margin: auto`, `padding`/`border`, `height: auto`, `overflow` (visible/hidden/scroll/auto → scrollable).
- Inline: basic line breaking (word wrap, `white-space: normal/pre/nowrap`), `line-height`, simplified `vertical-align: baseline`, `text-align`.
- Replaced: `img` with intrinsic dimensions + `width`/`height` attrs, `video` placeholder + aspect-ratio.

**Stage B:** positioning.

- `static`/`relative`/`absolute`/`fixed`, `z-index` + stacking contexts, basic `float`/`clear`, real `overflow` scrolling with bars.

**Stage C (committed for daily-driver):** flexbox + grid.

- Flex: `flex-direction`/`wrap`, `justify-content`, `align-items`/`content`, `flex-grow`/`shrink`/`basis`, `gap`.
- Grid: `fr`/`px`/`%`/`auto` tracks, `gap`, `grid-template-areas`, basic auto-placement, `justify`/`align` items.
- Tables: basic `table`/`table-row`/`table-cell` layout (foster parenting already handled in HTML).

**Cross-cutting rules:**

- Incremental layout: dirty bits per subtree. JS mutation → mark dirty → re-layout that subtree only, unless viewport/fonts change.
- Viewport units (`vw`/`vh`) and media queries re-evaluate on resize.
- Caps: box depth 512, fall back to block if exceeded.

## 7.4 Text, fonts, images

- Text: `cosmic-text`/`parley` + `fontdb`. Basic shaping (harfbuzz via `rustybuzz` if needed). `font-family` fallback, `font-display: swap`, basic bidi (Unicode Bidi), simplified UAX#14 line breaking.
- Images: `image` crate (png/jpeg/gif/webp) + `resvg`/`usvg` for SVG-as-image. Async decoding off the UI thread, in-memory cache (100 MB LRU). `alt` on failure. `srcset`/`sizes` subset (w descriptors) in an advanced phase.
- Color/alpha, basic `border-radius`, simplified `box-shadow` (late phase), `opacity`, `transform: translate/scale` for compositing only (no 3D in v1).

## 7.5 Paint + composite

- Display list: `FillRect`, `TextRun`, `Image`, `Border`, `Clip`, `ScrollFrame`, `StackingContext`. Backend-independent.
- v1 backend: `wgpu` (GPU) + `softbuffer` (software) fallback, switchable via flag/`--software-render`. VSync, damage-region repainting only once mature; v1 may be full-frame but must never block the UI.
- Compositing: layers per stacking context/scroll; 60fps scrolling with no re-layout (layer offsets).
- Scroll/zoom: wheel/touchpad scrolling, self-drawn native-like scrollbar, basic pinch-zoom (viewport scale, no full typographic zoom in v1).

## 7.6 Fixed API

```rust
// crates/layout
pub fn build_render_tree(dom: &Document, styles: &ComputedStyles) -> RenderTree;
pub fn layout(viewport: Size, tree: &mut RenderTree, opts: LayoutOpts) -> LayoutResult;
// crates/paint
pub fn build_display_list(layout: &LayoutResult) -> DisplayList;
pub fn composite(backend: &mut Backend, list: &DisplayList, viewport: Rect);
```

## 7.7 Acceptance criteria

- [ ] Acid2-like + custom flex/grid fixtures render with no major overlaps.
- [ ] Scrolling does not trigger full re-layout (counters).
- [ ] Readable text with font fallback; async images never block first paint.
- [ ] Window resize re-layout < 100 ms on typical pages (goal).
