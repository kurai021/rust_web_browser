# Phase 6 measured layout compatibility

- 12 custom advanced-layout scenarios: flex grow/shrink/wrap/reverse/column,
  nested navbar gap sizing, auto margins, grid fractions/areas/auto-fit/implicit
  rows, positioning/normal-flow separation, floats/clear, tables/spans, fixed
  viewport hit tests, context hoisting/isolation and adversarial extents/tracks.
- Three incremental scenarios compare geometry/text/glyph coordinates and
  stacking order with full layout after mutation, insertion/removal, style,
  resource and viewport changes. They verify clean-subtree reuse and no stale
  box indices; geometric float comparisons allow only 0.001 CSS px rounding.
- Two CSS grammar/shorthand scenarios verify atomic invalid-declaration handling,
  bounded track/area/placement parsing and CSSOM/typed-style agreement.
- One paint test verifies z-order pixels, fixed scroll composition and hit order.
- Two WPT-adapted flex reftests (`flex-shrink-001/002`): positive shrink ratios and
  invalid negative factors. Each checks 10,000 green-square reference pixels.
  Source/provenance/license live in `paint/tests/wpt/`.

Pass rates refer only to these published subsets. Existing HTML/CSS/DOM/JS suites
are retained. There is no whole-WPT or full flex/grid/table conformance percentage.
Remaining compatibility boundaries: full intrinsic sizing/inline fragmentation,
inline positioned containing blocks, sticky, named grid lines/subgrid, table
column rules/complete collapsed-border conflict resolution, and complex float/
overflow/positioning interactions. Those are not advertised as measured features.
