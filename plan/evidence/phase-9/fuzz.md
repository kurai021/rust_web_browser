# Phase 9 sanitized page/runtime run

Owned-code target: `browser/crates/web_api/fuzz`, `page_apis`, nightly +
ASan. The added object-URL activation seed covers Blob creation, native download
command handoff, revocation and detached-node cleanup. No transport/platform or
filesystem sink is created in the harness. See [testing-scope.md](testing-scope.md).

```sh
CARGO_BUILD_JOBS=2 ASAN_OPTIONS=quarantine_size_mb=16 \
  cargo +nightly fuzz run page_apis fuzz/corpus/page_apis fuzz/seeds -- \
  -max_total_time=300 -max_len=8192 -timeout=10 -rss_limit_mb=1536 \
  -print_final_stats=1
```

```text
#68702 DONE cov: 23310 ft: 67997 corp: 4356/508Kb lim: 363 exec/s: 228 rss: 233Mb
Done 68702 runs in 301 second(s)
stat::number_of_executed_units: 68702
stat::average_exec_per_sec: 228
stat::new_units_added: 757
stat::slowest_unit_time_sec: 0
stat::peak_rss_mb: 233
```

No sanitizer, panic, timeout or OOM artifact. This bounded run was supervised and
completed before the release build; no fuzz campaign overlapped compilation.

After the transport-only Hickory upgrade and MSRV-compatible standard API
adjustments, the final sanitized binary replayed the growing corpus under both
`-runs=5000` and a 60-second deadline. Corpus initialization accounts for the
reported execution count exceeding the requested run limit:

```text
#8985 DONE cov: 23295 ft: 67996 corp: 4190/477Kb lim: 363 exec/s: 152 rss: 206Mb
Done 8985 runs in 59 second(s)
stat::number_of_executed_units: 8985
stat::new_units_added: 0
stat::slowest_unit_time_sec: 0
stat::peak_rss_mb: 206
```

No error/artifact in the final replay. DNS/transport remain uninstantiated by
this target, so the dependency audit is its separate verification boundary.
