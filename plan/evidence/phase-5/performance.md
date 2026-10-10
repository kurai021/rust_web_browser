# Phase 5 performance baseline

2026-10-09, Linux aarch64 / Apple M2 Pro, stable Rust 1.99, portable release
profile (opt-level 3, thin LTO; no target-cpu=native). No own SIMD/asm kernel.

## Criterion

Command: `cargo bench -p js --bench interpreter -- --sample-size 20 --warm-up-time 1 --measurement-time 2`.

| Workload | Estimate interval |
|---|---|
| Parse notebook app | 135.00–135.09 µs |
| Parse 44 KiB declarations | 3.1659–3.1684 ms |
| 1,000 lexical closure calls | 1.4052–1.4085 ms |
| Collect 1,000 object cycles | 228.54–272.42 µs |

These are baseline measurements, not a pre-existing JS performance comparison.
VM setup/workload generation is outside the batched execution/GC measurements.

## End-to-end DOM/GC probe

`cargo run --release -p shell --example phase5_probe`:

```json
{"parse_ms":0.357,"eval_ms":0.252,"churn_10000_ms":78.424,"gc_ms":6.861,"event_ms":0.030,"kill_ms":0.022,"killed":true,"objects":[273,20274,273],"accounted_bytes":[407988,34060692,4700564],"remaining_listeners":12}
```

All 20,001 temporary JS objects are reclaimed; the remaining 12 listeners belong
to the notebook. DOM arena capacities stay reserved for slot reuse, explaining
the 4.7 MB post-collection accounting versus 0.4 MB initial accounting. This is
accounted allocation, not RSS. A separate regression verifies slot reuse.
The killer probe uses a reduced 2,000-tick limit; GUI verification uses defaults.

## GUI/headless observations

The controlled GPU notebook run first painted in 32.043 ms; GPU initialization
then selected Apple M2 Pro (Vulkan). Subsequent mutation frames generally took
about 0.7–4.3 ms, with asynchronous layout commonly 0.8–4.2 ms. First paint for
this small completed response is after EOF; the slow-stream Phase 4 regression
still proves previews before EOF.

The infinite-loop button stopped on the environment cap before five seconds,
showed the notice, and allowed immediate omnibox navigation. Time/instruction/
allocation caps can stop scripts earlier than the five-second ceiling.

Headless notebook loading, initial JS, CSS, layout and painting also pass. Its
snapshot precedes the delayed 100 ms timer, so timer behavior is verified in the
worker/GUI tests rather than inferred from a headless pixel hash.
