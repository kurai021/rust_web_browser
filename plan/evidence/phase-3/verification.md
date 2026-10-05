# Phase 3 verification record

Host: Linux aarch64, stable Rust 1.99.0; nightly fuzz harness 1.101.0-nightly.
Commands run in `browser/` except the fuzz run (`browser/crates/css/`).

## Workspace results

| Component | Default passed | Scalar-config passed |
|---|---:|---:|
| browser binary | 8 | 8 |
| css unit + robustness + WPT-adapted | 27 | 27 |
| html unit + existing conformance | 16 | 16 |
| net unit + offline integration | 27 | 27 |
| shell unit + CSS loading integration | 20 | 20 |
| 8 remaining workspace crates (stubs) | 8 | 8 |
| **Total** | **106** | **106** |

```sh
cargo test --workspace --all-targets
cargo test --workspace --no-default-features
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build
cargo audit
cargo deny --log-level error check
```

Existing HTML benchmark target also reports `Testing parse_1mb: Success` in
test mode; no new performance claim is inferred from that smoke check.

## Focused integration checks

```sh
cargo test -p shell --test css_loading
cargo test -p css --test robustness --test wpt_adapted
cargo test -p shell woff1_tables_decompress_into_a_usable_sfnt
BROWSER_WOFF2_FIXTURE="<registry>/woff2-patched-0.4.0/src/test_resources/lato-v22-latin-regular.woff2" \
  cargo test -p shell real_woff2_fixture_decodes_and_registers_css_alias -- --ignored
```

All passed. Coverage includes source ordering, inline/important precedence,
invalid-value recovery, import-cycle/depth rejection, MIME rejection, no fetch
for unused font faces, real font decoding, actual border pixels and link
geometry under computed typography, plus narrower-viewport recomputation.

## Fuzz reproduction

```sh
ASAN_OPTIONS=quarantine_size_mb=16 cargo +nightly fuzz run parse -- \
  -max_total_time=300 -max_len=4096 -timeout=3 -rss_limit_mb=1536 \
  -verbosity=0 -print_final_stats=1
```

Result: 103,860 executions; 101 MB peak RSS; exit 0. Generated corpus stays
local. Retained seed files are in `css/fuzz/seeds/`.

## Graphical fixture reproduction

```sh
python3 -m http.server 8877 --bind 127.0.0.1 \
  --directory crates/shell/tests/fixtures/css-demo
cargo run -- http://127.0.0.1:8877/
cargo run -- --headless-test http://127.0.0.1:8877/ --perf
```

Resize the browser below 500px. Wide/narrow captures in this directory show
the responsive visual transition. The server/browser spawned for verification
were terminated after capturing evidence.
