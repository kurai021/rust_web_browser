# Phase 7 sanitized API fuzzing

Final target: `browser/crates/web_api/fuzz`, nightly, own `PageRuntime`/DOM/JS and
SSE parser. It creates no transport or native platform and executes no I/O
commands. Seeds cover coercions, deferred bodies, mutation observers, canvas and
IDB. Generated corpus/artifacts/targets are ignored; seeds and harness are kept.

```sh
cargo +nightly fuzz run page_apis corpus/page_apis seeds -- \
  -max_total_time=300 -rss_limit_mb=1536 -max_len=8192 \
  -timeout=10 -print_final_stats=1
```

Final run after modal-wait and resource/permission fixes:

```text
#91137 DONE cov: 20124 ft: 55054 corp: 3510/344Kb lim: 238 exec/s: 302 rss: 547Mb
Done 91137 runs in 301 second(s)
stat::number_of_executed_units: 91137
stat::average_exec_per_sec: 302
stat::new_units_added: 1272
stat::slowest_unit_time_sec: 0
stat::peak_rss_mb: 547
```

No panic, sanitizer error, timeout or OOM artifact. Earlier growing-corpus runs
completed 101,629 and 93,103 executions in 301 seconds (543 / 542 MiB peak RSS).
They are supporting runs; the final run above validates the final core code.
Sanitizer/corpus RSS is distinct from the interpreter's conservative allocation
budget. Inputs also exercise the existing parser/interpreter and serialization
boundaries. CI adds this target at five minutes per change and one hour nightly.
