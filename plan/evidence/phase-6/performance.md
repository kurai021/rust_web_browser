# Phase 6 performance baseline

Linux aarch64, Apple M2 Pro, stable Rust 1.99. Portable release profile; no
target-cpu=native, own SIMD or assembly. Criterion command:

`cargo bench -p layout --bench flow -- --sample-size 20 --warm-up-time 1 --measurement-time 2`

| Workload | 95% interval |
|---|---|
| flow/100_paragraphs | 1.5357–1.5369 ms |
| flow/resize_100_paragraphs | 1.5505–1.5528 ms |
| advanced/workshop, layout only | 1.6327–1.6347 ms |
| advanced/incremental-card, including CSS/tree rebuild | 4.9586–4.9689 ms |

The incremental benchmark includes more stages than layout-only; they are not
a direct speedup comparison. The old Stage A baseline (~1.386 ms / ~1.428 ms)
changes by about 11% / 9%, below the 20% regression gate. Typical layout remains
well below the 100 ms goal. No measured bottleneck requires an optimized kernel.

`phase6_probe` observed initial layout 2.180 ms, one-card mutation 1.571 ms,
full layout 1.772 ms and resized layout 2.017 ms. Its 86-box workshop mutation
marks eight render nodes dirty and reuses 54 boxes across 25 subtrees. Timings
are local observations, not universal hardware claims. The geometry comparison
suite independently verifies the reuse result against full layout.

Graphical verification: first paint 51.459 ms with asynchronous GPU initialization,
44.546 ms with forced software. A real card mutation marked 11 nodes dirty and
reused 53 boxes in 24 subtrees. GPU layout/preparation took 8.969 ms and mutation
frame work 1.1–1.2 ms; software layout/preparation took 14.946 ms, with ~11–12 ms
frame work. Controlled scroll changed offset to 921.5 while the GPU layout counter
stayed 5 (frame work 1.9–6.1 ms); fixed header stayed in place. Software scroll
likewise did not increment the layout counter. Narrow media restyle/resize stayed
under 13 ms in the captured worker measurements.
