# browser — Graphical web browser in Rust (Phase 5)

Sealed theoretical framework in `../plan/` (see `../plan/SEALED.md`, v1.2).
Sequential phased implementation per `../plan/14-roadmap-phases.md`.

## Current status: Phase 5 — In-house JavaScript MVP and DOM scripting

Cargo workspace with 12 component crates. `net`, `html`, `css`, `layout`,
`paint` and `shell` now render nested block/inline flow, shaped text, generated
content and asynchronous PNG/JPEG/GIF/WebP/SVG images. A backend-independent
display list drives wgpu/Vulkan/GL and the software fallback. Page layout,
glyph preparation and image decoding run on workers; scroll and pinch/Ctrl+wheel
zoom change layer offsets/scale without re-layout. Resize re-evaluates CSS and
queues a new flow layout, discarding obsolete revisions.

`js` now supplies a bounded lexer/parser, lexical AST interpreter, Level 1
builtins and tracing arena GC. `dom_bindings` connects selectors, mutation,
inline CSSOM, events, basic form controls and timers to the existing HTML/CSS
engines. A dedicated JS worker preserves a realm across interactions and cancels
it when navigation changes. Classic blocking/defer scripts run with basic CSP,
nonce/hash and redirect/mixed-content checks. Script resource exhaustion stops
that page's scripts and displays a visible notice while navigation stays usable.

Start the graphical browser with `cargo run -- https://example.com/`.
`--software-render` forces software presentation; GPU initialization is otherwise
asynchronous with fallback on failure. `--perf` prints local frame/layout and
first-paint counters. `--headless-test` is only the test harness and now fetches,
executes initial classic scripts, lays out and paints HTML/CSS/images, reporting
a deterministic pixel hash. Flex/grid/positioning/tables are Phase 6. The measured
language/DOM surface and current compatibility boundaries are documented in
`crates/js/README.md`, `crates/dom_bindings/README.md` and the Phase 5 evidence.

## Layout

```
browser/
  src/main.rs          # binary: GUI entry and headless test harness
  crates/
    net/ html/ css/ layout/ paint/ js/
    dom_bindings/ web_api/ adblock/ shell/ profile/ test_utils/
```

## Commands

```sh
cargo test --workspace                  # tests with simd feature (default)
cargo test --workspace --no-default-features  # scalar-fallback tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo run -- [url] [--profile-dir p] [--software-render] [--perf] [--headless-test url]
cargo audit
cargo deny check
```

## Release

`[profile.release]` in `Cargo.toml`: `opt-level=3`, `lto=thin`,
`codegen-units=16`, `panic=unwind`. `target-cpu=native` forbidden
(plan/15 §15.3): SIMD uses runtime dispatch.

## Local demos and performance

Serve `crates/shell/tests/fixtures/css-demo/` over local HTTP, then open its
URL in the browser. It demonstrates external/imported/embedded/inline CSS,
inline typography, clickable links and the 500px media breakpoint.

Serve `crates/shell/tests/fixtures/flow-demo/` for nested cards, intrinsic images,
inline styling/bidi, generated content and an overflow scroll frame. Wheel,
PageUp/PageDown and the document scrollbar scroll; Shift+wheel/horizontal
touchpad deltas pan wide content. Ctrl+wheel and pinch scale the scene.

```sh
cargo run --release -- http://127.0.0.1:8764/ --perf
cargo run --release -- http://127.0.0.1:8764/ --software-render --perf
cargo bench -p layout --bench flow
cargo bench -p paint --bench composite
cargo run --release -p shell --example phase4_probe
cargo test -p paint --test backends gpu_and_software_render_the_same_display_list -- --ignored --exact
```

Serve `crates/shell/tests/fixtures/js-demo/` to try the counter, menu toggle, form
validation and vanilla-JS simple TodoMVC. Text controls support typing/Backspace;
Tab/Shift+Tab moves focus, Enter activates buttons or the Todo entry handler.
The timer and infinite-loop buttons exercise the persistent worker and killer.

```sh
python -m http.server 8765 --bind 127.0.0.1 --directory crates/shell/tests/fixtures/js-demo
cargo run --release -- http://127.0.0.1:8765/ --perf
cargo run --release -- http://127.0.0.1:8765/ --software-render --perf
cargo bench -p js --bench interpreter
cargo run --release -p shell --example phase5_probe
cargo test -p js --test differential -- --ignored --exact representative_language_results_match_node
```

Node is a test oracle only. The explicit `wayland_input` example is an optional
GUI-test pointer for compositors exposing the virtual-pointer protocol; it is
not a product input service. No external JS engine is embedded.

Phase 4 is the measured Stage A implementation described in `crates/layout/README.md`
and `crates/paint/README.md`. GIF/WebP decode their first frame. SVG-as-image
uses local font fallback and rejects external resource references. Resource
caps fail to visible alt/placeholder content rather than blocking the UI.

CI lives at the repository root `.github/workflows/ci.yml`; Cargo commands
run inside `browser/`. Current evidence is under `../plan/evidence/phase-5/`;
the retained Stage A graphics evidence remains in `../plan/evidence/phase-4/`.
