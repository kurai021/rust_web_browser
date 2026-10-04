# Phase 1 evidence — Minimal network + minimal window (2026-10-04)

## DoD (plan/14)

- [x] `cargo test --workspace --all-targets`: **54 passed, 0 failed** (see `cargo-test.log`; 5 online tests `#[ignore]`d, run separately below).
- [x] `cargo test --workspace --no-default-features`: 54 passed, 0 failed.
- [x] `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- [x] `cargo fmt --all --check`: clean.
- [x] `cargo deny`/`cargo audit`: binaries unavailable in this environment (no CI yet); manual license scan instead — all direct deps permissive, full-tree scan clean except dual-licensed `self_cell` used under Apache-2.0 (see licenses registry). `deny.toml` unchanged and still accurate.
- [x] WPT: N/A (starts in Phase 2 per plan/13).
- [x] `unwrap`/`expect` audit: only startup invariants (worker spawn, runtime build, TLS init, surface presence) and test code. Zero on network/page input paths, which all return `Result`.
- [x] Exit criteria demonstrated (see below).

## Exit criteria (plan/14 Phase 1)

- [x] Browse to `https://example.com`, see the HTML source as text. Verified two ways:
  - Headless: `browser --headless-test https://example.com` → `HEADLESS OK status=200 … bytes=577`.
  - Graphical: screenshot `shot3.png` (archived out-of-tree at `/tmp/shot3.png`): toolbar with back/forward/reload-stop buttons, omnibox showing the final `https://example.com/`, wrapped raw source with URL/status/type/size header, `Done — 4 lines` status bar.
- [x] Valid/invalid certificates shown: online tests against badssl.com all green — `expired.badssl.com → Tls(Expired)`, `wrong.host.badssl.com → Tls(WrongHost)`, `self-signed.badssl.com → Tls(UnknownIssuer)` (run: `cargo test -p net --test fetch_online -- --ignored` → 5 passed).
- [x] UI never freezes: fetch runs on a dedicated worker thread (tokio runtime); UI polls an mpsc queue and only repaints. Window stayed responsive while loading (title updates, progress in status bar).

## What was built

`net` (plan/04): WHATWG URL parse/classify (+ bare-host https upgrade), `reqwest`+`rustls`+`tokio` fetch (HTTP/1.1+HTTP/2, system+bundled roots, gzip/br/zstd, cookies), 10-hop redirects, 10 MB body cap, 10s/15s/60s timeouts, typed errors incl. TLS mapping, in-memory HSTS + JSON save/load, https→http fallback for bare hosts (never on TLS errors).

`shell` (plan/10 partial): `winit` window, toolbar + editable omnibox (caret, history-aware), scrollable raw-text viewport (`cosmic-text` + `softbuffer`), status bar, local error pages (network/TLS/HTTP/search-stub), single-context back/forward history, stop/reload, CLI wiring incl. working `--headless-test`.

## New dependencies (pinned in `browser/Cargo.lock`)

- `net`: url 2.5.8, tokio 1.53.2, reqwest 0.12.28 (rustls webpki+native roots, http2, stream, gzip/brotli/zstd/deflate, cookies), futures 0.3.34, thiserror 2.0.21, serde 1.0.229, serde_json 1.0.151 (+dev: flate2, rustls 0.23).
- `shell`: winit 0.30.13, softbuffer 0.4.8, cosmic-text 0.12.1, tokio, url, thiserror.
- `browser` bin: shell, net, url, tokio (current-thread runtime for headless).

## Bugs found and fixed during the phase (record)

1. `reqwest` TLS root features: `rustls-tls-manual-roots` exposes only `tls_built_in_root_certs`; system+bundle needs `rustls-tls-webpki-roots` + `rustls-tls-native-roots` (both default-on).
2. `reqwest` streaming needs the `stream` feature (`bytes_stream`); `futures` is at 0.3, not 1.
3. TLS error mapping: `&(dyn Error)` cannot downcast without `'static`, and `request_ref` is unavailable — mapping is keyword-based over the source chain, gated on TLS markers (validated by badssl tests).
4. Test helper bug: `?` inside the chain-walk loop returned `None` instead of ending the walk.
5. `cosmic-text` `shape_until_scroll` never terminates on extreme scroll offsets (walks lines one by one from `usize::MAX/2`) — spun the UI thread at 100% CPU. `TextBuffer::full_height` now grows the layout box temporarily instead. Documented on the function.
6. First frame never committed: stale 1100×750 guess vs compositor-assigned size. `render()` now reads `window.inner_size()` every frame and logs surface errors instead of failing silently.
7. `Surface` borrow vs `&mut self` paint: solved with field-disjoint borrows (surface stays in `self`, helpers are free functions), no take/restore.

## Known limitations (tracked, not scope creep)

- Single browsing context (tabs in Phase 9); no HiDPI scaling (physical pixels); IME composition unhandled (`KeyEvent.text` only); no horizontal scroll in the omnibox (long URLs clip the caret); scrollbar is drag+wheel only (no click-to-jump); HSTS persistence wired only when `--profile-dir` is passed; `cargo deny`/`audit` runs pending CI with network.
- Visual check was manual (screenshots); automated GUI tests land with `test_utils` from Phase 2.

## Gate to Phase 2

Phase 1 green. Next: Phase 2 — HTML tokenizer + tree + DOM per plan/05, WPT html/syntax subset, structured text rendering in this window.
