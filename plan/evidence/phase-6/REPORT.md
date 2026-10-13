# Phase 6 — Advanced and incremental layout

Local implementation over base `c61bb39`, 2026-10-13. The sealed v1.2 framework
is preserved. Scope: plan/07 Stages B/C and roadmap Phase 6.

Local Phase 6 acceptance is verified: responsive desktop/narrow fixtures,
GPU/software interactions, incremental mutation, retained fixed scrolling,
published compatibility subsets and transversal quality gates pass.

Delivered: owned CSS layout grammars/shorthands, relative/absolute/fixed boxes,
z-index/opacity stacking, float/clear exclusions, flex sizing/lines/alignment,
grid tracks/areas/placement, basic table rows/cells/spans, bounded subtree and
measurement caches with render-node dirty flags. GPU/software reuse the same
retained scene and stacking order. The script/runtime/network architecture is
reused for mutation and deferred resources.

Evidence:

- [verification.md](verification.md): quality commands and regressions.
- [performance.md](performance.md): portable scalar benchmarks and invalidation probe.
- [compatibility.md](compatibility.md): measured WPT/custom subsets and boundaries.
- [fuzz.md](fuzz.md): instrumented HTML/CSS-to-layout campaigns.
- [gui.md](gui.md): responsive workshop, mutation and fixed-scroll captures.

The workshop uses generic engine features: a flex navbar, responsive cards,
named dashboard areas, floats, overlapping z-index boxes, table spans and an
acid-like four-cell grid. This is the published basic Stage B/C subset, not
full CSS layout conformance or a daily-driver claim. Crate README lists remaining
compatibility edges. No external layout engine, new product dependency or own
SIMD/assembly kernel was introduced.
