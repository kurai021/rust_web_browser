# Web API fuzzing (defensive, own code only)

Sanitized libFuzzer target for the **owned** `PageRuntime`, DOM/JS bridge, SSE
parser and page-API subsets. It fuzzes this repository's code only; the harness
creates no transport, native platform or I/O command — only data.

See [testing scope](../../../../plan/evidence/phase-9/testing-scope.md)
for the full scope, boundaries and resource discipline.

```sh
# From browser/crates/web_api; single target, serialized with heavy builds
CARGO_BUILD_JOBS=2 cargo +nightly fuzz run page_apis fuzz/corpus/page_apis fuzz/seeds -- \
  -max_total_time=600 -max_len=8192 -timeout=10 -rss_limit_mb=1536
```

Seeds cover coercions, deferred bodies, mutation observers, canvas, IDB, modern
DOM (`DOMParser`, `dataset`, `NodeList`), async fetch and structured-clone worker
messages. Generated `corpus/`, `artifacts/`, `target/` are ignored; harness and
seeds are kept.
