# Phase 6 kernel decisions

| Candidate | Decision | Evidence |
|---|---|---|
| Box layout / K5 | Portable owned scalar algorithms | Typical advanced layout ~1.63 ms; no SIMD/asm experiment justified |
| Incremental geometry | Cache validated identities/constraints | Real mutation reuses 53/86 boxes; full-equivalence tests pass |
| Painting | Retain existing GPU/software scene paths | Fixed/scroll transforms do not run layout; adapter pixel parity passes |

No own unsafe, assembly or SIMD kernel. No target-cpu=native build change.
Default/scalar suites agree; existing auxiliary-library optimizations remain
subject to their original measured dependency policy.
