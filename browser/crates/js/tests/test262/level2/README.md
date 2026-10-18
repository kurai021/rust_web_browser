# Level 2 Test262-adapted inputs

Pinned upstream: `tc39/test262` commit
`d53ebc8b1931d9f1541e43d59650776e4b766248`, retrieved 2026-10-18.
BSD-3-Clause: the retained license is [`../LICENSE`](../LICENSE).
Each file retains its copyright and exact upstream path relative to `test/`.

Thirty selected sources cover Promise resolution/combinators/finally, async
completion and parameters, generators, Map/Set/WeakMap (including weak symbol
keys), Symbol, BigInt, Proxy invariants, Reflect receivers and source-text modules.
Two additional files are imported module fixtures and are not counted as cases.
Spec metadata/commentary is condensed and body formatting shortened; test inputs,
assertions and expected values are retained. No assertion is removed to obtain a
pass. The owned harness supplies assert/SameValue/throws/checkSequence/$DONE.

Scripts run in fresh sloppy and strict realms; modules run once, inherently
strict, with real linking, live imports and top-level-await jobs. Asynchronous
tests must finish with exactly one successful `$DONE`; unhandled rejections and
job errors fail the case. GC is exercised between the initial task and its jobs.

Three explicitly selected sources are excluded with reasons in the report:
optional WeakRef, and two tests needing `$262.createRealm`. Excluded **sources**
are reported separately from executed case-mode units. This subset is not an
estimate of the pass rate over the complete upstream suite. Structured clone is
a Web API and has separate graph/worker/storage integration tests, not Test262.

```sh
BROWSER_TEST262_REPORT_DIR=target/test262-reports \
  cargo test -p js --test test262_level2 -- --nocapture
```
