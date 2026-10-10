# Phase 5 bounded parser/runtime fuzzing

The cargo-fuzz/libFuzzer target parses arbitrary UTF-8, evaluates accepted ASTs
in the own interpreter, then collects the heap. The oracle is not in this target.
Six committed seeds exercise language/classes/control/references/budgets/dates.

Harness limits: 8 KiB source, 5,000 tokens, 2,000 parser work nodes, depth 32;
10,000 runtime ticks, 100 ms per execution, 16 calls, 10,000 objects/environments,
64 KiB strings and 32 MiB accounted memory. Product limits are documented in
`browser/crates/js/README.md`.

From `browser/crates/js/`:

```sh
ASAN_OPTIONS=quarantine_size_mb=16 cargo +nightly fuzz run parse_eval fuzz/corpus/parse_eval fuzz/seeds -- -max_total_time=300 -max_len=8192 -timeout=3 -rss_limit_mb=1536
```

Earlier stabilized campaign: 501,877 executions in 301 seconds, 219 MiB peak RSS,
coverage 13,378 / features 58,932, no crash/timeout/OOM. This followed the parser,
reference and native allocation hardening.

Final campaign on current code (2026-10-10, includes strict-eval/CSP/calendar,
native checkpoint and keyboard-focus fixes): 395,078 executions in 301 seconds,
256 MiB peak RSS, coverage 13,972 / features 66,021, no crash/timeout/OOM and no
artifacts written. Raw log is local-only; the command above reproduces it.

Corpus/build/artifact output is ignored. CI runs five minutes on push/PR and one
hour on schedule; the deterministic language/DOM tests supplement the fuzzer
with larger/deeper and host-interaction cases.
