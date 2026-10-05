# Phase 4 — Flow layout, paint and GPU window

Date: 2026-10-05. Base: `1cb5517` (Phase 3). Scope: roadmap Phase 4,
plan/07 Stage A. The sealed theoretical documents are preserved; this is their
implementation/evidence appendix.

## Implementation

- `layout`: DOM/computed-style render tree, stable element/pseudo identities,
  display pruning/contents, generated content, anonymous blocks and inline splits.
  Nested box model, auto/min/max/percentage sizing, centering, positive/negative
  sibling/parent/empty margin collapse, baseline word lines, shaped text and bidi.
- Replaced images: intrinsic/CSS/attribute sizing and aspect ratio; bounded alt
  text on failure. Video/input placeholders. Dimension attributes enter the CSS
  cascade as presentation hints, allowing author auto sizing to override them.
- `paint`: backend-independent fills/text/images/borders/clips/scroll/opacity
  commands; retained raster scene, glyph sharing and viewport culling. Vulkan/GL
  wgpu backend, FIFO VSync, glyph/fill atlas, image textures and scalar fallback.
- The window reuses navigation/history/omnibox/status. Network, bounded image
  decoding, page layout and glyph preparation run on workers. Revision IDs and
  page identity reject stale navigation, CSS, font, image and resize results.
- Scroll/pinch/Ctrl+wheel use retained scene offsets/scale. Nested overflow clips,
  hit-testing and two-axis offsets agree. Resize re-evaluates media and layout.
- The existing transport streams decoded chunks into the incremental HTML parser.
  Partial snapshots paint before EOF; the finished DOM is reused for final CSS.
  Images/fonts are deferred and cached; they do not delay fallback first paint.
- CLI `--software-render` and `--perf` now operate; headless harness lays out and
  paints, with stage times and deterministic pixel hash. Offline image-bearing
  documentation/blog fixture under `shell/tests/fixtures/flow-demo/`.

## Quality gates

| Gate | Result |
|---|---|
| `cargo test --workspace` | 131 passed, 0 failed; 7 intentional ignores |
| `cargo test --workspace --no-default-features` | 131 passed, 0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo fmt --all --check` | pass |
| `cargo build --release` | pass; portable profile, no target-cpu=native |
| `cargo audit` / `cargo deny --log-level error check` | pass; existing maintenance-only notices, no vulnerability exemption |
| Explicit GPU parity | pass on Apple M2 Pro (G14S B1), Vulkan; max allowed channel delta 3, scales 1 and 1.5 |
| WPT/html5lib-derived retained subset | 80/80 entity cases; 45/45 custom tree-format cases |
| WPT-adapted CSS retained subset | 52/52 assertions in 7 Rust tests |
| New flow/custom tests | 17 pass; model/geometry/overflow/bidi/depth/resize/images |
| Progressive/cancellation tests | pass; actual software pixels before a held EOF and deferred image decode |
| New input-path audit | no unwrap/expect/panic/unsafe in layout/paint source |

The seven ignores are five online transport cases, optional external WOFF2
fixture and hardware adapter parity (the latter was explicitly run). The WPT
rates describe the recorded adapted/headless subset, not full WPT/JS execution.

## Performance and demonstrated exit

Linux aarch64, Apple M2 Pro, rustc 1.99.0, release profile. Details and commands
in `performance.md`, `kernel-registry.md` and `verification.md`.

- Criterion: 100-paragraph layout ~1.386 ms; narrow resize ~1.428 ms.
- Scalar 1024×768 scroll benchmark ~1.594 ms.
- K4 image/blend scroll workload (60 samples): CPU median ~9.675 ms, GPU with
  readback ~0.848 ms. Conservative measured gain ~91.2%; parity passes.
- Actual 1 MiB streamed GUI document: **first paint 35.336 ms, before EOF**.
  Sixty PageDown frames keep layout passes at **5**; frame work **2.097–8.344 ms**,
  within the ~60fps frame budget. Large layout completes on the CPU worker;
  frames/input continue independently. FIFO presentation is enabled.
- Synthetic offline 1,048,754-byte document: parse ~14.917 ms, CSS ~380.585 ms,
  layout+glyph preparation ~234.190 ms, paint ~4.290 ms; RSS ~479,280 KiB.
  This large-page memory/style cost is recorded as the later compatibility/perf
  baseline, not hidden by a typical-page claim.
- Removing a full ComputedStyle clone per opaque text word reduced the same
  large-page layout stage from ~1063.968 ms to ~234.190 ms (~78%). No SIMD/asm.
- Real GPU window and forced software window were verified with images, scroll
  and narrow media-query resize; screenshots and offline snapshots accompany
  the report. Window/test-server processes are closed after verification.

## Fuzzing and defects caught during verification

See `fuzz.md` for final run counts. Both targets run five-minute ASan campaigns;
CI schedules one hour and runs five-minute push/PR smoke checks.

- SVG: huge-radius arcs expanded into excessive cubic segments in the auxiliary
  path simplifier. XML depth/node and numeric geometry budgets now reject this
  input before simplification; regression seed retained. Raster axes/allocation,
  encoded/page limits and 100 MiB LRU remain enforced.
- HTML: implied unmatched `</p>` did not push its recovery element, causing
  infinite reprocessing. Fixed insertion/closure and regression case.
- HTML: malformed raw end-tag solidus handling rewound without advancing.
  Fixed boundary/progress behavior and raw-tag regression seed.
- HTML: adoption-agency inner-loop removals invalidated a saved formatting-list
  index. It now follows stable identities and adjusts its bookmark, retaining
  the common ancestor and enforcing clone budgets. Minimized regression retained.
- Streaming: incomplete tags/entities/raw closing tags and CRLF/UTF-8 chunks
  now wait for more data without injecting EOF; differential tests cover splits
  at 1/2/3/7/31/1024 bytes, script escape state and partial snapshots.
- Integration: Wayland explicit-sync failure during software→GPU switch fixed
  by deferring swapchain configuration until the software surface is released.

## Stage boundaries

This is the measured Stage A flow implementation. Flex/grid/positioning/tables
and subtree dirty layout follow the roadmap. Simplified word breaking, baseline
alignment, intrinsic inline-block sizing, inline decorations and uniform radii
are documented in crate READMEs. Opacity currently multiplies primitive alpha;
isolated offscreen group surfaces and advanced stacking are later paint work.
GIF/WebP decode the first frame; SVG image resources are isolated and no SVG
scripts execute. No CPU SIMD/asm kernel was introduced.

## Outcome

Phase 4 implementation and measured Stage A exit are complete with the archived
gates. The next ordered span is Phase 5: MVP JavaScript and minimal DOM bindings.
