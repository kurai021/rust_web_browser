# Stage A flow layout

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
- Visible/hidden/auto/scroll overflow, nested clipping and two-axis offsets.
  Hit-testing applies exactly the same ancestor clips/offsets as paint.
- 512-depth flatten-to-block fallback, finite extents and a one-million-glyph
  budget. Render-tree generation is iterative; hostile nesting is regression-tested.

The shell queues layout/glyph preparation on its CPU worker, coalesces pending
requests and rejects obsolete navigation/content/font/viewport revisions.
Scrolling and compositing zoom use the retained result; they do not call layout.

## Phase boundaries and measured subset

This is Stage A, not flex/grid/positioned/table layout (Phase 6). Those display
types use their block-flow fallback here. Word breaking is simplified UAX#14;
tabs use four spaces, baseline alignment is simplified and inline-block sizing
uses a bounded content-width approximation. Full intrinsic-sizing algorithms,
inline fragmentation/decorations and complex script line breaking remain
compatibility work. Subtree dirty layout follows the later mutation phases.

Tests: `cargo test -p layout`. Benches: `cargo bench -p layout --bench flow`.
