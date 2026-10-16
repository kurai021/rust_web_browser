# Owned flow and advanced layout — Stages A/B/C

In-house layout over the existing DOM and computed CSS. Public entry points:

```rust,ignore
let mut tree = build_render_tree(&document, &styles);
let result = layout(viewport, &mut tree, LayoutOpts { font_system, images });
```

- Stable `ElementId { node, pseudo }`; display:none pruning, display:contents,
  before/after/markers, anonymous blocks and inline splitting around blocks.
- Nested content/padding/border boxes; auto widths/heights, min/max constraints,
  border-box sizing, auto-margin centering and collapsing positive/negative
  sibling/parent/empty margins. Percentage heights require a definite parent.
- Baseline-aligned word lines, mixed fonts, Unicode bidi, normal/pre/nowrap and
  preserved/newline whitespace modes, alignment, inline padding/backgrounds,
  generated text and hit regions derived from shaped content.
- Replaced images with intrinsic/attribute/CSS dimensions and aspect ratios;
  video/input placeholders. CSS dimension attributes are presentational hints,
  so author width/height:auto can restore natural sizing.
- Phase 7 canvas elements carry immutable document bitmaps into the same retained
  image path. Password controls paint masked labels. Layout box/scroll geometry
  also feeds observer checkpoints on the script worker.
- Visible/hidden/auto/scroll overflow, nested clipping and two-axis offsets.
  Hit-testing applies exactly the same ancestor clips/offsets as paint.
- 512-depth flatten-to-block fallback, finite extents and a one-million-glyph
  budget. Render-tree generation is iterative; hostile nesting is regression-tested.

The shell queues layout/glyph preparation on its CPU worker, coalesces pending
requests and rejects obsolete navigation/content/font/viewport revisions.
Scrolling and compositing zoom use the retained result; they do not call layout.

## Phase boundaries and measured subset

Phase 6 adds bounded owned flex/grid/positioned/table algorithms. Word breaking is simplified UAX#14;
tabs use four spaces, baseline alignment is simplified and inline-block sizing
uses a bounded content-width approximation. Full intrinsic-sizing algorithms,
inline fragmentation/decorations and complex script line breaking remain
compatibility work.

## Stage B/C and invalidation

- Relative offsets preserve normal-flow geometry; deferred absolute/fixed boxes
  resolve block containing rectangles and inset/auto sizing. Fixed display-list
  scopes ignore document scroll and outer scroll clips. One stacking traversal
  handles z-index/opacity contexts and feeds both painting and hit-testing.
- Floats create margin-box exclusions, wrapping inline text and supporting clear.
- Flex supports row/column and reversals, wrap, grow/shrink/basis, min/max freezing,
  gap, main/cross alignment, baseline and auto margins. Container intrinsic bases
  include nested flex gaps, avoiding premature navbar wrapping.
- Grid supports length/percentage/auto/fr/minmax/fit-content tracks, bounded
  repeat(auto-fit/fill), areas, spans, implicit tracks and row/column/dense
  placement, item alignment and auto margins.
- Basic tables align row/cell tracks, captions/groups and row/column spans;
  fixed equal tracks and auto content contributions, spacing, middle/bottom
  alignment and uniform collapsed cell borders are supported.
- `LayoutCache::layout` preserves the fixed full-layout entry point and adds
  snapshot-derived dirty bits per render node, ancestor propagation, matching
  constraints and resource identities. Clean small bundles and measurements
  are reused; viewport/fonts/navigation invalidate broader dependencies. Geometry
  tests compare mutation/structure/asset/resize results against full layout.

Limits: 128 grid columns/tracks, 512 implicit grid rows, 4096 table rows, bounded
placement/freezing work, 512 box depth and finite coordinate/glyph handling.
Subtree geometry cache budget is 32 MiB; measurement entries are bounded.

The reported subset does not claim full CSS layout conformance. Intrinsic sizing,
inline positioned containing blocks/fragmentation, sticky positioning, named
grid lines/subgrid, advanced table column rules/border conflict resolution and
some float/overflow corner cases remain compatibility work. Resource exhaustion
marks the retained result truncated instead of allocating unbounded tracks.

Tests: `cargo test -p layout`. Benches: `cargo bench -p layout --bench flow`.
