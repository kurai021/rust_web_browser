# Phase 8 — modern JavaScript and event-loop verification

Implementation and verification against Phase 7 baseline `9ec0944`, on Linux
aarch64 / Apple M2 Pro / stable Rust 1.99, 2026-10-18.

The owned VM now covers the committed Phase 8 Level 2 surface: Promises and
async/await, sync and async generators with `for await…of` and async `yield*`,
source-text modules (static and dynamic import, `import.meta`, top-level await),
Map/Set/WeakMap with ephemeron symbol keys, `Symbol.*`/well-known symbols with
the `toPrimitive`/`hasInstance`/`isConcatSpreadable`/`toStringTag` hooks, exact
BigInt, Proxy/Reflect with define-partial descriptors, structured clone
(including boxed/regexp/error/BigInt), and page-level FIFO task/microtask
ordering with `unhandledrejection`/`rejectionhandled`. The interpreter migrated
to an owned bytecode chunk (with the AST retained as a differential oracle),
insertion-ordered properties and labelled statements.

Evidence index:

* [spa.md](spa.md): async SPA probe and upstream TodoMVC acceptance.
* [fuzz.md](fuzz.md): final bounded sanitized runs for the JS and page targets.
* [performance.md](performance.md): Criterion deltas against the Phase 8 baseline.
* [testing-scope.md](testing-scope.md): defensive scope and boundaries of the
  fuzzing, native-stack and GC stress testing.

Quality gates run on the final tree: `cargo fmt --all`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
(283 tests, 0 failed), plus the differential oracles and the ignored TodoMVC
acceptance. This is an experimental phase increment, not full web-platform
conformance; browser/session completion and final hardening follow the sealed
roadmap.
