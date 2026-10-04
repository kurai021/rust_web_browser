# Phase 0 evidence — Foundation (2026-10-04)

## DoD (plan/14)

- [x] `browser/` workspace compiles (binary + 12 stub crates).
- [x] `cargo test --workspace --all-targets`: **18 passed, 0 failed** (see `cargo-test.log`).
- [x] `cargo test --workspace --no-default-features`: green (scalar fallback per doc 15).
- [x] `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- [x] `cargo fmt --all --check`: clean.
- [x] `cargo deny`/`cargo audit`: configured (`deny.toml`, CI); no external dependencies to audit in Phase 0.
- [x] CLI stub honors the contract: `[url]`, `--profile-dir`, `--software-render`, `--perf`, `--headless-test`, `--help`, `--version`; unknown flag → exit 2.
- [x] Release `RUSTFLAGS` + `simd`/`no-simd` features pinned (no `target-cpu=native`).
- [x] Dual license + `licenses.md` registry (zero external deps).
- [x] Toolchain: `rustc 1.99.0`, `cargo 1.99.0`, declared MSRV `1.85`, edition `2021`.
- [x] Framework v1.2: full English translation of docs, code, comments and CLI strings.

## Gate to Phase 1

Phase 0 green. Next: Phase 1 — minimal network (`net`: URL, GET HTTPS, redirects, gzip, timeouts) + minimal `winit` window with a raw-text viewport.
Requires adding external dependencies (`winit`, `tokio`, `hyper`/`reqwest`+`rustls`, `url`, …) and therefore network access for `cargo fetch`.
