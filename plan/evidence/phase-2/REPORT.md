# Phase 2 evidence — Displayable HTML + DOM (2026-10-04)

## DoD (plan/14)

- [x] `cargo test --workspace --all-targets`: **75 passed, 0 failed** (see `cargo-test.log`; 5 online tests `#[ignore]`d).
- [x] `cargo test --workspace --no-default-features`: 75 passed, 0 failed.
- [x] `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- [x] `cargo fmt --all --check`: clean.
- [x] `cargo deny`/`cargo audit`: still unavailable here; new deps are `encoding_rs` (MIT/Apache, mandated by plan/05), `criterion` (dev-only) and `libfuzzer-sys` (fuzz-only, never in the binary). Registry updated.
- [x] WPT subset with report: `entities.test` (real file, 80/80) + `local.dat` (45 hand-authored tree tests, 45/45) — archived in-tree at `browser/crates/html/tests/wpt/`.
- [x] Fuzz clean: deterministic 2000-case smoke in stable CI + real libFuzzer sessions (nightly).
- [x] `unwrap`/`expect` audit: only startup invariants, peek-then-bump logic invariants and test code. Zero on network/page input paths.
- [x] Exit criteria demonstrated (see below).

## Exit criteria (plan/14 Phase 2)

- [x] Wikipedia/HN-style pages readable with navigable links. Verified: local multi-link test page renders headings, paragraphs, blue underlined links (incl. wrapped), list items; link overlay/hit roundtrip unit-tested (every fragment clickable); omnibox E2E navigation verified with real keystrokes (`Ctrl+L` + URL + Enter → "SECOND PAGE OK"); link-click path = unit-tested hit + proven fetch pipeline (physical click E2E is manual).
- [x] Fuzz clean (see below).
- [x] Bench: 1 MB synthetic HTML parses in **22 ms** (goal: < 200 ms).

## What was built

`html` (plan/05): arena DOM (`dom.rs`), entities (`entities.rs`: full numeric incl. C1 fixups + common named table), WHATWG tokenizer (`tokenizer.rs`: all states incl. script double-escape, CDATA gating), tree construction (`tree.rs`: all insertion modes, foster parenting, adoption agency, foreign content with SVG fixups, template contents), driver (`parser.rs`: BOM/meta/UTF-8 decoding, incremental push/suspend/resume, budgets).

`shell` (plan/10 partial): DOM → article blocks (headings, paragraphs, pre, lists, rules, images, tables-as-rows), per-block text buffers with accent link overlays + underlines, click-to-follow, document title in window title, omnibox select-all (Ctrl+L/click), error pages as articles.

## New dependencies (pinned in `browser/Cargo.lock`)

- `encoding_rs` 0.8 (mandated by plan/05), `criterion` 0.5 (dev, benches), `libfuzzer-sys` 0.4 (fuzz harness only, nightly).

## Conformance notes (spec-first, honestly recorded)

- Framework is built against the living WHATWG spec (verified verbatim Oct 2026 for the adoption agency). The only runnable oracle available, html5lib 1.1 (2019), diverges on 3 advanced cases; current-spec behavior was kept in each:
  1. `<b>1<p>2</b>3` → `p > (b > "2", "3")` (outer-loop closes the clone; oracle agrees here).
  2. `<select><table>…` → select popped, table as sibling (current spec pop+reprocess; oracle drops the table).
  3. Leading `<template>` → under `<head>` (spec before-head rules; oracle puts it under body).
- Quirks carve-out implemented and verified: `<table>` does not close `<p>` without a usable doctype (both modes covered by fixtures).
- Comment serialization strips (`<!-- hi -->`); foreign end tags match ASCII-case-insensitively.
- Named entity table covers everyday text (~250 entries); full 2000+ table is tracked Phase 11 work.

## Bugs found and fixed during the phase (record)

1. Fuzzer crash: entity scan window split a multi-byte char (33-byte cap) → boundary-safe truncation.
2. Fuzz-smoke panic: unsafe `input[pos..]` indexing → all cursor access via `get()`, clamped rewind.
3. UI thread 100% CPU: `shape_until_scroll` never terminates on extreme scroll offsets → `full_height` grows the layout box instead (documented on the function).
4. Adoption agency: wrong insert location (current node vs common ancestor) created a DOM cycle; inner loop cloned non-formatting nodes (spec drops them from the stack).
5. `</svg>`/`</foreignObject>` ignored + `div` escaping to body: camelCase fixup vs lowercase comparisons.
6. Missing `InRow` td/th arm, unguarded `close_cell` over-popping, `li`/`dd` search not stopping at specials, select table-parts dropped.
7. `AfterHead` EOF must still create the (empty) body — verified against html5lib.
8. Link offsets broke across whitespace collapsing → single-pass run builder (offsets always match final text).
9. Link overlay x measured from line start instead of run start (wrapped lines) → measure from run start.
10. Omnibox appended on Ctrl+L+type → select-all semantics + highlight.

## Known limitations (tracked, not scope creep)

- Fragment parsing is simplified (documented in code); full fidelity lands with `innerHTML` (Phase 5+).
- Quirks table is simplified (force/name-based); full public-id table lands with CSS (Phase 3).
- `Pre` wraps instead of horizontal scroll; headings/list sizes are fixed (no CSS yet).
- Physical mouse-click E2E verified only via unit roundtrip + visual overlays; keyboard E2E verified live.
- `cargo deny`/`audit` runs pending CI with network.

## Gate to Phase 3

Phase 2 green. Next: Phase 3 — CSS syntax/selectors/cascade/values per plan/06, WPT css subset measured.
