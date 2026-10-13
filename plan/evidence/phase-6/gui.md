# Phase 6 graphical verification

Offline workshop: `shell/tests/fixtures/advanced-demo/`, local HTTP port 8766.
Real winit/worker/paint path; the explicit Wayland test pointer and wtype drive
interactions. GPU selects Apple M2 Pro Vulkan; software is forced with its flag.

Acceptance: desktop flex navbar does not wrap unexpectedly; cards occupy separate
grid tracks; named areas span correctly; positioned badge stays in its panel;
text excludes float margin boxes and clear starts below them; high-z red box stays
above later blue; table rows/columns/spans align; acid-like cells have equal sizes.
Mutation changes the first card and publishes reuse counters. Scroll keeps the
fixed header in the viewport without a layout pass. Narrow resize reevaluates
media styles and creates one-column cards. All saved captures were read back to
verify browser-only cropping, expected content and backend status. A preliminary
narrow crop shifted with window centering and was replaced after explicitly
repositioning the test window.

Capture index:

- `gpu-wide.png`, `software-wide.png`: desktop navbar, three separate cards,
  named-area dashboard and absolute badge.
- `gpu-mutation.png`: longer card content and stretched row after real JS click.
- `gpu-scroll.png`, `software-scroll.png`: fixed header under scroll, ordered
  overlapping layers, aligned table spans and four green acid-like grid cells.
- `gpu-narrow.png`, `software-narrow.png`: 480px media styles, static column
  navbar and single-column cards, with the mutated paragraph retained.

GPU/software parity is separately checked by the shared display-list adapter
test; screenshot similarity is not used as a substitute for that pixel test.
