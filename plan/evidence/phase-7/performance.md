# Phase 7 performance and renderer observations

Linux aarch64 / Apple M2 Pro, portable release profile, stable Rust 1.99. No own
SIMD or assembly and no target-cpu=native. Criterion ran after fuzzing completed:

`cargo bench -p layout --bench flow -- --sample-size 20 --warm-up-time 1 --measurement-time 2`

| Workload | Phase 7 95% interval | Phase 6 reference |
|---|---|---|
| flow/100_paragraphs | 1.5232–1.5245 ms | 1.5357–1.5369 ms |
| flow/resize_100_paragraphs | 1.5382–1.5462 ms | 1.5505–1.5528 ms |
| advanced/workshop | 1.6315–1.6329 ms | 1.6327–1.6347 ms |
| advanced/incremental-card | 4.9684–4.9786 ms | 4.9586–4.9689 ms |

All changes remain far below the 20% regression gate. Criterion reported changes
within noise for flow/resize/workshop and no detected change for incremental.

The real workshop's asynchronous GPU-startup path first painted at 53.464 ms;
its forced software run first painted at 59.419 ms. Typical captured GPU frame
work was around 0.7–4.6 ms, with a 22.229 ms interaction/upload outlier; software
frames were roughly 7.9–11.2 ms in the initial capture. These are local samples,
not a universal 60fps claim. An observer/status mutation reused 66/71 boxes, and
clean updates reused all 71 boxes. Canvas upload/draw reuses the retained image
path and stable immutable frame IDs.

Scroll changes retained offsets. Intersection callbacks can then mutate their
status label and legitimately request an incremental layout; the callback's
layout is distinct from the scroll transform itself. Resize recomputes layout
and delivers updated content-box observations. Headless mode uses bounded async
settlement, so a live worker/SSE can consume its explicit test budget rather than
being treated as a pure layout-only timing.
