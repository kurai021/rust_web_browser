# Phase 5 kernel registry

| Candidate | Decision | Evidence |
|---|---|---|
| JS parser scanning | Keep portable scalar Rust | Notebook parse ~135 µs; initial baseline only |
| Interpreter dispatch / K3 | Keep bounded tree walking | 1,000 closure calls ~1.37 ms; bytecode migration belongs to Phase 8 |
| DOM mutation/GC | Keep tracing arena implementation | 10,000 node/listener churn ~81 ms; collection ~8 ms |

No own SIMD or assembly was introduced. Default and scalar-feature workspace
suites agree. GPU/software display-list parity remains independently verified.
Auxiliary libraries may use their existing optimized paths; there is no new
project-specific optimized kernel or target-cpu=native build setting.
