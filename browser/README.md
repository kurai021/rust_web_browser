# browser — Graphical web browser in Rust (Phase 9)

Sealed theoretical framework in `../plan/` (see `../plan/SEALED.md`, v1.2).
Sequential phased implementation per `../plan/14-roadmap-phases.md`.

Rust 1.89+ is required by the resolved audited dependency set. The workspace
minimum now reflects that requirement; current verification also uses stable
Rust 1.99.

## Current status: Phase 9 — Tabs, sessions and downloads

Cargo workspace with 12 component crates. `net`, `html`, `css`, `layout`,
`paint` and `shell` now render nested block/inline flow, shaped text, generated
content and asynchronous PNG/JPEG/GIF/WebP/SVG images. A backend-independent
display list drives wgpu/Vulkan/GL and the software fallback. Page layout,
glyph preparation and image decoding run on workers; scroll and pinch/Ctrl+wheel
zoom change layer offsets/scale without re-layout. Resize re-evaluates CSS and
queues a new flow layout, discarding obsolete revisions.

`js` supplies the owned bytecode VM, bounded modern-language parser, Promises,
async/await and generators, modules, collections, Symbol, BigInt, Proxy/Reflect,
structured clone and tracing GC. The retained AST path is a test oracle.
`dom_bindings` connects selectors, mutation,
inline CSSOM, events, basic form controls and timers to the existing HTML/CSS
engines. A dedicated JS worker preserves a realm across interactions and cancels
it when navigation changes. Classic blocking/defer scripts run with basic CSP,
nonce/hash and redirect/mixed-content checks. Script resource exhaustion stops
that page's scripts and displays a visible notice while navigation stays usable.

The owned layout engine now implements relative/absolute/fixed positioning,
stacking contexts, float/clear, flex lines, grid tracks/areas/auto-placement and
basic table rows/cells/spans. The layout worker retains a bounded subtree and
measurement cache: DOM/style changes mark ancestor dependencies dirty, reuse
compatible siblings, and translate retained geometry including glyph positions.
Resize/fonts/navigation invalidate the relevant cache state. Painting and hit
testing consume the same stacking traversal; fixed layers stay in the viewport.

`web_api::PageRuntime` now connects fetch/XHR with shared CORS/preflight rules,
history/location, profile-backed localStorage/cookies/IndexedDB, sessionStorage,
dedicated workers, WebSocket/SSE, layout-driven observers and CPU Canvas2D frames.
Requests and worker messages cross bounded, cancellable service channels; VM/DOM
identities stay on their owner thread. Canvas pixels reuse the retained image
path in both renderers. Native origin-labelled dialogs provide Once/Always/Never
permission choices; `about:permissions` revokes current-origin or all saved grants.
The API inventory, quotas and explicit compatibility boundaries are in
[`crates/web_api/README.md`](crates/web_api/README.md).

Start the graphical browser with `cargo run -- https://example.com/`.
`--software-render` forces software presentation; GPU initialization is otherwise
asynchronous with fallback on failure. `--perf` prints local frame/layout and
first-paint counters. `--headless-test` is only the test harness and now fetches,
executes classic scripts and bounded asynchronous service work, lays out and
paints HTML/CSS/images/canvas, reporting a deterministic pixel hash. The measured
language/DOM surface and current compatibility boundaries are documented in
`crates/js/README.md`, `crates/dom_bindings/README.md` and the Phase 5 evidence.
This remains experimental. Measured modern-JS compatibility is recorded in the
Phase 8 evidence; the Phase 9 report covers the browser/session interface. The
blocker/privacy integration and final compatibility hardening follow in Phases
10 and 11.

The graphical shell supports independently running tabs, reorder/duplicate/close,
URL-based session restore, local history/bookmarks and omnibox suggestions,
per-site zoom, rendered-text find, native preferences and basic A4 PDF export.
Downloads stream to disk through a five-transfer queue with validated Range /
If-Range resumption, pause/cancel, native save paths and persistent metadata.
Read-only Elements/Console and a bounded in-memory Network log are available via
F12. All chrome is drawn by the application; no webview or external page engine
is embedded.

GUI profiles default to `$XDG_CONFIG_HOME/navegador-rust` or
`~/.config/navegador-rust`; `--profile-dir` selects another directory.
[`crates/profile/README.md`](crates/profile/README.md) describes formats and caps.
Session restore loads the selected page first and leaves background tabs dormant
until selected. A clean `about:blank` startup makes no connections. Explicit
`--headless-test about:blank` exercises that same startup path.

### Browser shortcuts

| Action | Shortcut |
| --- | --- |
| New / close / cycle tab | Ctrl+T / Ctrl+W / Ctrl+Tab (Shift reverses) |
| Select tab / last tab | Ctrl+1…8 / Ctrl+9 |
| Reorder selected tab | Ctrl+Shift+PageUp / PageDown, or drag the tab |
| Omnibox / reload / force reload | Ctrl+L / Ctrl+R / Ctrl+Shift+R |
| Back / forward | Alt+Left / Alt+Right |
| History / bookmarks / bookmark page | Ctrl+H / Ctrl+B / Ctrl+D |
| Downloads / save / print PDF | Ctrl+J / Ctrl+S / Ctrl+P |
| Find / next / previous | Ctrl+F / Enter or F3 / Shift+Enter or Shift+F3 |
| Per-site zoom / reset | Ctrl++ / Ctrl+- / Ctrl+0 (80–200%) |
| Preferences / menu / developer tools | Ctrl+, / F10 / F12 |

Native managers also accept Tab/Shift+Tab and Enter/Space. Preferences, history,
bookmarks and downloads use local `about:` pages. Downloaded files never open
automatically; the explicit folder button opens their directory.

### Phase 9 acceptance

```sh
CARGO_BUILD_JOBS=2 cargo run -p web_api --example demo_server -- 8099
cargo run -- --profile-dir /tmp/browser-workshop http://127.0.0.1:8099/browser/
cargo run -p shell --example phase9_probe -- http://127.0.0.1:8099/
cargo test -p shell --test browser_session --test browser_tools --test downloads -- --test-threads=2
```

The loopback workshop covers five tabs, independent page state, a 100 MiB
streamed download, fetch→Blob→native save, bookmarks/history, find and PDF export.
Use a temporary download directory when recording it. Robustness tests exercise
owned code with bounded inputs/runs; compile with `CARGO_BUILD_JOBS=2`, limit test
threads and serialize heavy checks. See `../plan/evidence/phase-9/` for results.

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
export CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2  # bounded local builds/tests
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

Serve `crates/shell/tests/fixtures/advanced-demo/` for the Phase 6 workshop:
fixed flex navbar, responsive grid cards, named dashboard areas, float/clear,
z-index layers, table spans and a small acid-like grid. The card button exercises
JS mutation and incremental layout counters in `--perf` output.

```sh
python -m http.server 8766 --bind 127.0.0.1 --directory crates/shell/tests/fixtures/advanced-demo
cargo run --release -- http://127.0.0.1:8766/ --perf
cargo run --release -p shell --example phase6_probe
cargo test -p layout --test advanced --test incremental
cargo test -p paint --test wpt_adapted
```

Phase 4 is the measured Stage A implementation described in `crates/layout/README.md`
and `crates/paint/README.md`. GIF/WebP decode their first frame. SVG-as-image
uses local font fallback and rejects external resource references. Resource
caps fail to visible alt/placeholder content rather than blocking the UI.

The Phase 7 workshop needs its loopback API server, rather than a static server:

```sh
cargo run -p web_api --example demo_server -- 8097
cargo run --release -- http://127.0.0.1:8097/ --profile-dir /tmp/browser-workshop --perf
cargo run --release -p shell --example phase7_probe -- http://127.0.0.1:8097/ /tmp/browser-probe
cargo test -p web_api --test wpt_adapted -- --nocapture
```

The fixture exercises JSON fetch, SPA routing/back, persistent visits and login
(`demo` / `rust`), IDB stores/indexes, worker messages, socket echo, SSE, observers,
drawImage/text/paths and native permission controls. Its authentication is fixed
synthetic test data. Desktop clipboard adapters use `wl-copy`/`wl-paste` on Linux
Wayland, and notifications use `notify-send`; missing backends report an error.
Geolocation grants return unavailable when there is no local provider and make
no hidden location-service request. See the fixture README and Phase 7 evidence.

CI lives at the repository root `.github/workflows/ci.yml`; Cargo commands
run inside `browser/`. Current evidence is under `../plan/evidence/phase-7/`;
the retained Stage A graphics evidence remains in `../plan/evidence/phase-4/`.
