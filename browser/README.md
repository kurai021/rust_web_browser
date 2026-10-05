# browser — Graphical web browser in Rust (Phase 3)

Sealed theoretical framework in `../plan/` (see `../plan/SEALED.md`, v1.2).
Sequential phased implementation per `../plan/14-roadmap-phases.md`.

## Current status: Phase 3 — CSS and computed styles

Cargo workspace with 12 component crates. `net`, `html`, `css` and `shell`
now load and render structured HTML with CSS colors, typography, inline styles,
simple margins/borders, responsive media rules and page-owned font sources.
The other component crates remain the planned extension points.

Start the graphical browser with `cargo run -- https://example.com/`.
`--headless-test` is only the test harness and also loads document CSS.
Full flow layout/GPU painting is next (Phase 4).

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

## Local CSS demo

Serve `crates/shell/tests/fixtures/css-demo/` over local HTTP, then open its
URL in the browser. It demonstrates external/imported/embedded/inline CSS,
inline typography, clickable links and the 500px media breakpoint.

CI lives at the repository root `.github/workflows/ci.yml`; Cargo commands
run inside `browser/`. Evidence is under `../plan/evidence/phase-3/`.
