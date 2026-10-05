# Phase 3 — CSS and computed styles

Date: 2026-10-05 (UTC). Baseline: `099e0d2` (Phase 2).
Scope: `plan/14-roadmap-phases.md`, Phase 3, and `plan/06-css.md`.

## Recovery of interrupted work

The repository contained only the initial CSS tokenizer/manifests changes.
Its five existing tests passed before modification. The tokenizer was extended
in place; Phase 2 HTML/navigation/links were retained. The framework documents
and seal were not changed.

## Checklist

- [x] CSS Syntax 3 preprocessing, escapes, functions/URLs and tokenization.
- [x] Qualified/at-rule parsing, balanced declarations, `!important`, recovery.
- [x] Selector lists, specificity, attributes, structural/functional selectors,
  combinators and pseudo-element style targets.
- [x] Colors, relative units and dimension-checked `calc/min/max/clamp`.
- [x] Origins, inline declarations, importance, basic layers, inheritance,
  wide keywords, custom properties and cycle-safe `var()` substitution.
- [x] Media conditions and declaration-based `@supports`.
- [x] Embedded local UA stylesheet and internal getComputedStyle contract.
- [x] `<link>`, `<style>` and `style=""`, with source order and source-base URLs.
- [x] `@import` hydration with cycle/depth/request/byte bounds.
- [x] `@font-face` descriptors, local/TTF/OTF/WOFF1/WOFF2 sources, bounded
  decompression, CSS family aliases, page cache and fallback-to-font swap.
- [x] Computed typography, inline spans, colors, simple boxes/borders/margins
  painted in the existing graphical viewport; links retain shaped hit geometry.
- [x] Resize/theme restyling on the existing background worker; navigation IDs
  and revision checks reject stale page/style/font events.
- [x] WPT-adapted subset measured; fuzz and quality gates verified.

## Acceptance evidence

### Default/offline checks

`cargo test --workspace --all-targets`: **106 passed, 0 failed, 6 ignored**.
The ignored tests are five existing online network tests and one optional
real-file WOFF2 test. The latter was run explicitly and passed.

`cargo test --workspace --no-default-features`: **106 passed, 0 failed**.

`cargo fmt --all --check` and
`cargo clippy --workspace --all-targets -- -D warnings`: pass.

`cargo audit`: pass (maintenance-only informational warnings).
`cargo deny --log-level error check`: **advisories/bans/licenses/sources ok**.

Selected output/commands are recorded in `verification.md`; the larger
session command logs are represented by their reproducible commands and
crate-level result totals rather than machine-specific build paths.

### WPT

`cargo test -p css --test wpt_adapted`: **7 tests passed, 52/52 assertions**.

- Syntax: identifier start/escaped ID rules, 8 assertions.
- Selectors: complex negation, negation specificity, escaped attributes and
  selected nth-child-of-list cases, 32 assertions.
- Cascade: important versus inline and root inheritance, 12 assertions.

Actual source inputs/assertions are adapted to Rust APIs; no JS harness is
claimed to execute. Provenance, licenses and exact denominator are in
`browser/crates/css/tests/wpt/README.md`.

### Robustness

Coverage-guided CSS run (nightly, AddressSanitizer):

```text
max_total_time=300, max_len=4096, timeout=3, rss_limit_mb=1536
stat::number_of_executed_units: 103860
stat::average_exec_per_sec:     345
stat::new_units_added:          3948
stat::slowest_unit_time_sec:    0
stat::peak_rss_mb:              101
```

Exit status 0; no crash, timeout or OOM artifacts. Later small corrections
(root rem resolution, screen/print separation, universal pseudo targets) have
targeted regressions plus the 1,000-input malformed-CSS test. Fuzz config,
lock and reproducible seeds are retained; generated corpus/target/artifacts
are ignored.

Font verification: a native SFNT is wrapped into compressed WOFF1, decoded
and parsed by fontdb. The registry's Lato WOFF2 fixture also decoded and
registered its CSS alias successfully. Binary font fixtures are not vendored.

### Graphical/headless end to end

Fixture: `browser/crates/shell/tests/fixtures/css-demo/`.

```text
HEADLESS OK status=200 url=http://127.0.0.1:8877/ bytes=703
HEADLESS CSS sheets=2 blocks=3 warnings=0 title=Some("CSS Phase 3 Demo")
```

The page combines external/imported/embedded/inline styles. Verified in the
real winit window: blue heading, centered content, green border, bold/italic
inline runs, green underlined link, hidden element excluded.

The same window was resized from 1261×1030 to **420×700**. Its below-500px media
query changed the paragraph background to yellow, reduced text size and
changed page margins. Cropped screenshots: `browser-wide.png` and
`browser-narrow.png`. Only this session's test window/processes were operated.

## Required build/CI plumbing

The first actual deny run exposed missing metadata in the earlier setup:

- Local path dependency versions are explicit (`0.1.0`).
- `Zlib` (existing slotmap arena) and `CDLA-Permissive-2.0` (existing Mozilla
  roots data bundle) are allowed with provenance in the license registry.
- Maintenance advisories for paste, rustybuzz and ttf-parser have narrowly
  documented exceptions in deny.toml. They are not vulnerability exemptions;
  replacement review belongs to Phase 11 compatibility/dependency work.
- The GitHub workflow moved to repository-root `.github/workflows/ci.yml`,
  with Cargo commands in `browser/`. CSS fuzz runs five minutes on push/PR
  and one hour on the scheduled job. Remote CI execution is not claimed here.

## Phase boundaries / remaining compatibility work

The viewport still uses Phase 2's article blocks. Full nested flow layout,
margin collapsing, generated pseudo boxes, background image/gradient paint,
and advanced positioning remain the specified subsequent phases. Keyframes
are retained as metadata, not animated yet.

Nested cascade layers currently use flattened declaration order; the selected
WPT result is not a full CSS conformance claim. Font decoding accepts single
SFNT fonts, not WOFF2 TTC collections. Font CORS and redirect-chain mixed-
content enforcement remain part of the network/Web-API security work; direct
scheme checks and final-response checks are already applied to CSS/fonts.

No additional product features or framework scope were introduced.

## Gate

Phase 3 acceptance criteria verified. Next is Phase 4: flow layout,
render/display tree and GPU/software paint pipeline.
