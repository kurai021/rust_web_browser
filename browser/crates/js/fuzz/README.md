# JS engine fuzzing (defensive, own code only)

Sanitized libFuzzer target for the **owned** JavaScript parser, bytecode compiler
and interpreter. It fuzzes this repository's engine only — no external process,
library or network target.

See [../../../plan/evidence/phase-8/testing-scope.md](../../../plan/evidence/phase-8/testing-scope.md)
for the full scope, boundaries and resource discipline.

```sh
# single target, time- and memory-bounded; do not run alongside heavy builds
CARGO_BUILD_JOBS=2 cargo +nightly fuzz run parse_eval corpus/parse_eval seeds -- \
  -max_total_time=600 -max_len=8192 -timeout=3 -rss_limit_mb=1536
```

Seeds cover modern constructs: async generators / `for await`, Proxy/Reflect,
WeakMap/Symbol, BigInt, structured clone and source-text modules. The harness
bounds step/duration/call-depth/object/environment/string/memory budgets and
re-collects GC on every iteration. Generated `corpus/`, `artifacts/`, `target/`
are ignored; harness and seeds are kept.
