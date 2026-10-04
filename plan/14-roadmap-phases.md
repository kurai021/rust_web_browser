# 14 — Phase roadmap (bridge, no dates)

> Strict order. No skipping. Each phase has Entry / Work / Exit (DoD). No dates: what matters is that each span makes sense and holds up the next one.

## Phase 0 — Foundation

- **Entry:** this approved `plan/`.
- **Work:** empty compiling `crates/*` workspace (doc 03), pinned toolchain/MSRV, `rustfmt`/`clippy`, CI (test/clippy/fmt/audit/deny), profile `config.toml`, `--headless-test` stub, license (MIT/Apache-2.0 by default; if GPL/AGPL enters as a doc-02 exception, it is documented and the affected binary inherits copyleft) + minimal `README` + initial registry `plan/evidence/phase-0/licenses.md`. Pin release `RUSTFLAGS` + `simd`/`no-simd` features (doc 15 §15.3), no `target-cpu=native`.
- **Exit:** green `cargo test --workspace` on Linux (with and without `--no-default-features`). Evidence in `plan/evidence/phase-0/`.

## Phase 1 — Minimal network + minimal window

- **Entry:** Phase 0.
- **Work (doc 04 + partial doc 10):** `net`: URL, GET https http/1.1+h2, redirects, gzip, timeouts, UI-visible errors. `shell`: `winit` window with omnibox + raw-text viewport (no HTML yet) + visible net/cert errors.
- **Exit:** browse to `https://example.com` and see the HTML source as text + valid/invalid certificate shown. UI never freezes. Doc 13 DoD.

## Phase 2 — Displayable HTML + DOM

- **Entry:** Phase 1.
- **Work (doc 05):** tokenizer + tree + DOM + `parse_full`/`push`/`finish` with caps + WPT html/syntax subset tests + structured text rendering (basic clickable headings/links, no CSS).
- **Exit:** readable, link-navigable Wikipedia/HN. Clean fuzzing.

## Phase 3 — CSS + computed styles

- **Entry:** Phase 2.
- **Work (doc 06):** Syntax/selectors/cascade/values + `@media`/`@import`/`@font-face` + UA sheet + internal `getComputedStyle` stub. Wire `<link>`/`<style>`/`style=""`.
- **Exit:** pages with correct basic colors/fonts/margins (no advanced layout). Measured WPT css subset.

## Phase 4 — Flow layout + paint + GPU window

- **Entry:** Phase 3.
- **Work (doc 07 Stage A + paint + doc 15 K1/K4/K5 if benches demand):** render tree, block/inline, images/text, display list, `wgpu` + software fallback, scroll, resize re-layout.
- **Exit:** image-bearing blogs/docs render and scroll at ~60fps. First paint before EOF on large pages. Base benches + first scalar-vs-simd report if any kernel was enabled.

## Phase 5 — MVP JS + minimal DOM bindings

- **Entry:** Phase 4.
- **Work (doc 08 Level 1 + partial doc 09):** lexer/parser/AST/ES5+let/const/arrow/template interpreter, `getElement`/`querySelector`/events/timers DOM + blocking/defer `<script>`. Long-script killer.
- **Exit:** counter, menu toggle, form validation and simple TodoMVC work. Level 1 Test262 subset reported.

## Phase 6 — Advanced layout (flex/grid/position)

- **Entry:** Phase 5.
- **Work (doc 07 Stages B/C):** absolute/fixed/z-index/float + flex + grid + tables. Incremental layout with dirty bits.
- **Exit:** common modern layouts (flex navbar, grid cards) with no overlaps. Custom acid-like fixtures pass.

## Phase 7 — Daily-driver Web APIs

- **Entry:** Phase 6.
- **Work (doc 09):** `fetch`/XHR/CORS, `history`/`location`, `local`/`sessionStorage`, basic `WebSocket`/SSE, dedicated `Worker`s, observers, `Canvas2D` subset, `IndexedDB` subset, permissions UI.
- **Exit:** SPA with `fetch`+`pushState`+storage, demo login, demo canvas chart. Measured WPT fetch/xhr/events/dom.

## Phase 8 — Full modern JS + event loop

- **Entry:** Phase 7.
- **Work (doc 08 Level 2 + bytecode VM + doc 15 K3):** Promises/async/microtasks, modules, Map/Set/BigInt/basic Proxy, migration to bytecode VM + GC. `structuredClone`. VM asm dispatch evaluated here only if benches justify it.
- **Exit:** simple real SPAs + `async`/`fetch`/DOM with no ordering bugs. Level 2 Test262 reported.

## Phase 9 — Complete browser (tabs/session/downloads)

- **Entry:** Phase 8.
- **Work (doc 10 + doc 11.1):** full tabs/session/history/bookmarks/find/zoom/omnibox, downloads engine+UI, `about:preferences`, error pages, final CLI.
- **Exit:** daily use possible on non-DRM sites: 5 tabs, history, pausable/resumable downloads. Green zero-connections-at-startup test.

## Phase 10 — Adblock + privacy by default

- **Entry:** Phase 9.
- **Work (doc 11.2/11.3 + doc 15 K2):** filter engine + EasyList/Privacy + cosmetic + counter + per-site toggle + visible updater + blocked 3rd-party cookies + minimal headers. Matcher SIMD at real scale allowed here.
- **Exit:** demo ads/trackers blocked before connecting, no background calls with the updater off.

## Phase 11 — Hardening + compat + perf (exit from experimental)

- **Entry:** Phase 10.
- **Work (docs 12+13+15):** full SOP/CSP/mixed-content/popup/permissions, mandated WPT suites with thresholds (set the exact % on arrival, e.g. >90% on core subsets), nightly fuzz, no-regression benches, per-tab `catch_unwind`, `unsafe`/asm/deps audit + published final scalar-vs-simd report.
- **Exit:** doc 01 "non-experimental" checklist all green + recorded demos. Daily-driver beta declared here.

## Phase 12 — Beta polish (optional v1.1)

- **Entry:** Phase 11.
- **Work:** DevTools Elements/Console/Network, print-to-PDF, bookmark import/export, light/dark themes, minimal i18n en/es, packaging (.deb/AppImage/tarball).
- **Exit:** installable beta release.

---

## Roadmap rules

1. Never start phase N+1 while phase N's DoD (incl. doc 13.2 gates) is not green.
2. Cuts made to move forward are documented by editing `01`/`09`/the affected phase — never silently in code.
3. Forbidden in every phase: FTP/Gopher/Torrent/DRM/extensions/crypto/telemetry (doc 01.4). Cite this file when rejecting.
4. Each phase leaves evidence in `plan/evidence/phase-<n>/` (test logs, WPT reports, screenshots). No evidence, no progress.
