# 03 — General architecture

## 3.1 High-level view (pipeline)

```
User (window/tabs/omnibox)
   │ events
   ▼
Shell UI (UI thread)
   │ navigation commands
   ▼
Navigation/Session (tabs, history) ──► Local storage (profile)
   │ per-tab fetch
   ▼
Net stack (URL → DNS → TCP → TLS → HTTP) ──► HTTP cache ──► Adblock (request blocking)
   │ document bytes
   ▼
HTML tokenizer → Tree builder → DOM
   │ + CSS (fetch css → parse → cascade) ──► Render tree (DOM+styles)
   │ + JS (parse → execute, may mutate DOM/CSSOM) [may block/reenter]
   ▼
Layout (box/flow/flex/grid) → Paint (display list) → Composite → GPU/Window
```

Pipeline rules:

- Everything is per tab/page (browsing context). No DOM shared across tabs.
- Network and parsing on background threads; layout/paint may run on dedicated threads; compositing/UI on the main thread.
- JS owns an event loop per page and may force re-layout/re-paint (dirty bits, no naive full re-layout once mature).

## 3.2 Workspace of crates (fixed structure)

Mandatory initial layout (exact names, so nothing is assumed):

```
Cargo.toml           # workspace
crates/
  net/               # URL, fetch, http, tls wrapper, cache, cookies, HSTS (uses hyper/rustls)
  html/              # tokenizer + tree builder + DOM core (no layout)
  css/               # tokenizer/parser, cascade, selectors, values, media queries
  layout/            # box tree, flow, flex, grid, positioning
  paint/             # display list, raster/composite, text, images (uses wgpu)
  js/                # lexer/parser/AST/interpreter/bytecode VM, GC/event-loop base bindings
  dom_bindings/      # JS<->DOM glue (hand-rolled WebIDL-like), base BOM
  web_api/           # fetch/XHR, storage, minimal workers (see doc 09)
  adblock/           # filter engine + matcher + list updater
  shell/             # graphical window, tabs, omnibox, history UI, downloads UI
  profile/           # on-disk profile: history, bookmarks, cookies, storage, config
  test_utils/        # harness, fixtures, headless driver for tests
src/main.rs          # binary: boots the shell
```

Allowed dependencies between crates:

- `shell` → all. `layout` → `css`+`html`. `paint` → `layout`. `dom_bindings` → `html`+`css`+`js`. `web_api` → `net`+`dom_bindings`.
- Forbidden: `html` → `js`, `css` → `net`, cycles. `net` never knows the DOM.

This avoids the monolith and lets each engine be tested in isolation.

## 3.3 Concurrency and processes

v1 model (no OS multiprocessing yet):

- **1 process, N threads:** UI thread, IO/net pool (tokio), parser pool, layout/paint workers, JS thread per tab (or pool), compositor thread.
- Communication via channels (`tokio::mpsc`, `crossbeam`) + `Arc<RwLock>` only where unavoidable. Prefer message-passing.
- Isolation: a tab crash/panic never kills the process; a "tab crashed" page is shown and can be reloaded. `catch_unwind` at tab boundaries in a late phase.

Future model (not v1, just keep the door open): multi-process with per-tab OS sandbox. Not implemented in v1, but APIs must not preclude it (context IDs, command serialization).

## 3.4 Allowed external dependencies (closed list)

Principle: reuse only what is "breaking stones" (sockets/crypto/compression/media/OS), never the engines.

Allowed (concrete examples, pin versions in Phase 0):

- Async/runtime: `tokio`, `futures`.
- HTTP/TLS/DNS: `hyper` or `reqwest` (with `rustls`), `hickory-resolver` if a custom DNS is needed, `rustls`, `webpki`.
- URL: `url`, `idna`, `punycode`.
- Compression: `flate2`, `brotli`, `zstd`.
- GUI/window/GPU: `winit`, `wgpu`, `softbuffer` (software fallback), `cosmic-text` / `parley` + `fontdb` for text, `image` (png/jpeg/gif/webp), `usvg/resvg` for SVG-as-image.
- Utilities: `serde`, `serde_json`, `regex`, `once_cell`, `thiserror`, `anyhow`, `tracing`, `clap` (minimal args).

Forbidden as product dependencies:

- Foreign web engines: `servo`, `chromium`, `v8`, `spidermonkey`, `boa`, `quickjs`, `swc` as the final engine (`swc` may be used only as a test oracle, not in the binary), `html5ever`/`markup5ever` as a final shortcut (allow at bootstrap? No: the HTML engine is in-house; `html5ever` only as a differential test oracle, never in the product's `crates/html`).
- App frameworks imposing a webview: `tauri`, `electron`, `cef`.
- Toolkits blocking render control: `egui/iced/gtk` allowed only for chrome UI if it speeds things up, but the page viewport is painted by our own `paint`. Exact decision in doc 10; default is direct `winit+wgpu`.

## 3.5 Data and state

- On-disk profile at `~/.config/<name-pending>/` or `~/.<name>/`: `history.sqlite` or `redb`, `cookies.json` (documented in-house format), `bookmarks.json`, `downloads/`, `adblock_lists/`, `config.toml`.
- Nothing leaves the profile without a user action. No sync in v1.

## 3.5b Aggressive performance (see doc 15)

- Rust-first. Each crate exposes a portable pure API; SIMD/asm/GPU paths live in `*_simd.rs` modules / `shaders/` with runtime dispatch + scalar fallback.
- Release `RUSTFLAGS` and `simd`/`no-simd` features pinned in Phase 0. `target-cpu=native` forbidden in release builds.
- Closed list of optimizable kernels in doc 15 §15.2. Outside that list, portable Rust only.

## 3.6 Errors and observability (no telemetry)

- Local logs with `tracing`, configurable levels, no remote shipping.
- Internal error pages (`about:net-error`, `about:crash`) rendered locally, no remote assets.
- Local performance counters (parse/layout/paint time, memory) visible under the `--perf` flag, never uploaded.
