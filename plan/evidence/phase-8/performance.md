# Phase 8 Criterion measurements

Comparable Criterion runs for `js/benches/interpreter.rs`, against the Phase 8
baseline saved in `/tmp/opencode/phase8-before-bench.log`. New cases added this
phase: `compile-numeric-bytecode`, `numeric-ast-reference`,
`numeric-bytecode-cached`, `map-1000-lookups`, `gc-5000-ephemeron-chain`.

Selected deltas (after vs. before; negative = faster):

```text
js/1000-closure-calls   change: [-12.948% -12.798% -12.646%] (p = 0.00 < 0.05)
js/parse-44KiB          change: [ -1.093%  -0.946%  -0.802%] (p = 0.00 < 0.05)
js/parse-notebook       change: [ -0.161%  +0.159%  +0.581%] (p = 0.38 > 0.05)
js/gc-1000-cycles       change: [-10.224%  -4.169%  +2.951%] (p = 0.24 > 0.05)
```

Plus the added cases (no panic; times recorded by Criterion):
`compile-numeric-bytecode`, `numeric-bytecode-cached`, `map-1000-lookups-500-entries`
and `gc-5000-ephemeron-chain` complete within budget, and
`numeric-ast-reference` vs. `numeric-bytecode-cached` confirm the bytecode path
does not regress against the retained AST oracle.

No regression exceeds the 20% review threshold. No `target-cpu=native`, `unsafe`
or hand-written asm was introduced; the interpreter remains portable safe Rust.
