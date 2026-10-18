# Phase 8 sanitized fuzzing (own code only)

Two libFuzzer targets, nightly + ASan, each **single-target, time- and memory-
bounded** (`-max_total_time=600 -max_len=8192 -rss_limit_mb=1536`). They fuzz this
repository's own engine only and create no external process, transport or
network target; see [testing-scope.md](testing-scope.md).

## JS parser / bytecode / interpreter — `js/fuzz` `parse_eval`

```sh
cargo +nightly fuzz run parse_eval corpus/parse_eval seeds -- \
  -max_total_time=600 -max_len=8192 -timeout=3 -rss_limit_mb=1536
```

```text
#172910 DONE   cov: 18576 ft: 80092 corp: 5643/496Kb lim: 602 exec/s: 287 rss: 289Mb
Done 172910 runs in 601 second(s)
```

## Page runtime / DOM / JS bridge — `web_api/fuzz` `page_apis`

```sh
cargo +nightly fuzz run page_apis corpus/page_apis seeds -- \
  -max_total_time=600 -max_len=8192 -timeout=10 -rss_limit_mb=1536
```

```text
#150401 DONE   cov: 22861 ft: 65643 corp: 4330/505Kb lim: 355 exec/s: 250 rss: 225Mb
Done 150401 runs in 601 second(s)
```

No panic, sanitizer error, timeout or OOM artifact in either run; the generated
`artifacts/` directories are empty. New seeds cover async generators / `for
await…of`, Proxy/Reflect, WeakMap/Symbol, BigInt, structured clone, source-text
modules and modern DOM (`DOMParser`, `dataset`, `NodeList`).

## Resource discipline

Fuzzing is never run concurrently with heavy builds. The earlier attempt to run
both 10-minute sanitized fuzzers and then a full `cargo test --workspace` compile
on the same 15 GiB machine exhausted RAM and CPU (Rust/LLVM parallel-compiles
many memory-hungry jobs). Subsequent builds/tests use `CARGO_BUILD_JOBS=2` under
`nice`, serialized. This is an operational constraint, not a code defect.
