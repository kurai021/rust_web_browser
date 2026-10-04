# browser — Graphical web browser in Rust (Phase 0)

Sealed theoretical framework in `../plan/` (see `../plan/SEALED.md`, v1.2).
Sequential phased implementation per `../plan/14-roadmap-phases.md`.

## Current status: Phase 0 — Foundation

Compiling workspace: `browser` binary + 12 stub crates.
No external dependencies yet (`std` only); added per phase.

## Layout

```
browser/
  src/main.rs          # binary: CLI stub (--headless-test included)
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
```

## Release

`[profile.release]` in `Cargo.toml`: `opt-level=3`, `lto=thin`,
`codegen-units=16`, `panic=unwind`. `target-cpu=native` forbidden
(plan/15 §15.3): SIMD uses runtime dispatch.
