# Phase 6 layout fuzzing

Own HTML/CSS parsing → render tree → advanced layout → finite geometry and hit
testing. The harness caps HTML to 256 nodes/depth 24, uses an 8 KiB input cap,
and reuses a test-only font system. Four seeds cover flex, grid, positioned huge
extents and table spans. Product track/placement/depth/cache caps also apply.

From `browser/crates/layout`:

```sh
ASAN_OPTIONS=quarantine_size_mb=16 cargo +nightly fuzz run layout fuzz/corpus/layout fuzz/seeds -- -max_total_time=300 -max_len=8192 -timeout=5 -rss_limit_mb=1536
```

First campaign: 23,186 executions in 301 seconds, peak 444 MiB RSS, coverage
15,704 / features 54,184; no crash/timeout/OOM.

Final campaign after navbar intrinsic-gap, margin/alignment and extent hardening:
21,932 executions in 301 seconds, peak 387 MiB RSS, coverage 16,436 / features
60,419, no crash/timeout/OOM. The writable corpus is separate from the four
source seeds. Generated inputs were merged into the ignored corpus (2724 inputs
processed, 1837 retained) and removed from the source-seed directory. CI uses
the separate corpus path in the reproducible command above.

Corpus/build/artifacts are ignored; manifests/lock/seeds are retained. CI runs
five minutes on push/PR and one hour on schedule. Deterministic resource and
incremental equivalence tests supplement this full-layout fuzzer.
