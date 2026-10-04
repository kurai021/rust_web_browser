# 13 — Testing, compatibility and performance

> Without this, "functional" is an opinion. Here it becomes measurable.

## 13.1 Strategy

- **Per-crate unit + integration:** each crate with `tests/` and fixtures. Coverage goal >70% for `net`/`html`/`css`/`js`, >50% for the rest in v1.
- **Golden/fixtures:** HTML/CSS/layout/paint with snapshots (serialized DOM, computed styles, display-list hash, tolerant screenshots).
- **Differential (oracles, never in the binary):** `html5ever`, `boa`/`node`, `servo/selectors` as references to catch divergences in tests.
- **WPT (web-platform-tests):** submodule or partial vendor. Committed subsets:
  - `html/syntax`, `dom`, `css/css-syntax`, `css/selectors`, `css/cascade`, `fetch`, `xhr`, `events`.
  - In-house harness in `crates/test_utils` running WPT headless (`--headless-test`) and reporting pass/fail/skip. Publish the rate per phase; it must rise, never drop >1% without justification.
- **Fuzz:** `cargo fuzz` for the `url`/`html`/`css`/`js`/`image` parsers. 1h in nightly CI, 5 min in PRs for fast targets. Zero panics/OOM.
- **Security/privacy:** "zero connections at startup" test (mock DNS/socket counter), SOP/CORS/CSP tests, `cargo audit` + `cargo deny` green.
- **Performance:** `criterion` benches per crate + e2e (load a synthetic local 1 MB page, measure parse/layout/paint/first-paint, RSS memory). Initial goals (non-blocking except >20% regressions):
  - Parse 1 MB HTML < 200 ms, typical layout < 100 ms, startup < 2s, 5 tabs < 500 MB.
- **SIMD/asm differential (doc 15):** every optimized kernel runs the same suite as the scalar fallback (`--features simd` vs `--no-default-features`). Divergence = fail. Scalar-vs-simd benches archived in `plan/evidence/<phase>/simd-<kernel>.md`.

## 13.2 Per-phase quality gates (transversal DoD)

A phase (doc 14) is done only when:

- [ ] `cargo test --workspace`, `clippy -- -D warnings`, `fmt --check`, `audit`/`deny` green.
- [ ] That phase's WPT subset with a report archived in `plan/evidence/<phase>/`.
- [ ] No `unwrap`/`expect`/`panic` on network/page input paths (grep audit).
- [ ] Doc updated if behavior changed.

## 13.3 Minimum demonstrable compatibility

Leaving "experimental" (doc 01) requires a recorded demo + tests loading with no intervention:

- `example.com`, a Wikipedia article, Hacker News, a CSS blog, MDN-like docs, form login + fetch, vanilla TodoMVC, progressive `<video mp4>`, an ads page (visible adblock).

Without this, no daily-driver is declared even if the code "compiles".
