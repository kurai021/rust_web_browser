# Phase 9 startup and idle resource measurements

Platform: Linux aarch64, Apple M2 Pro / Vulkan, 15 GiB RAM. Portable release flags
remain unchanged; no own SIMD/asm or `target-cpu=native` was introduced.

`shell/examples/phase9_probe.rs` uses the production tab/worker paths and the real
loopback workshop. It loads five contexts, including the modern board and Web API
workshop with their dedicated workers, settles work, then samples four seconds of
process CPU. The Linux probe gates idle usage below 10% of one core and five-tab
RSS below 500 MiB. A clean blank startup is checked before any navigation.

```text
debug:   startup_ms=261.4 idle_cpu_percent=0.788 rss_kib=181056
release (final audited dependencies): startup_ms=61.5 idle_cpu_percent=0.334 rss_kib=161408
```

The release sample is about 160 MiB RSS. The five-tab GPU GUI observed 235520 KiB
(230 MiB), with the native managers and document snapshots present. These are
local warm-system fixture measurements, not claims for arbitrary remote pages.
The first software workshop paint was approximately 316 ms in the debug GUI;
individual restored fixture paints were also below two seconds.

## Idle-GC finding

The first GUI run exposed about 56% of one core in an otherwise idle page worker.
Timer polling ran full GC even without a due timer, and the worker repeated that
collection. After correcting that path, the five-tab run exposed the same pattern
in two dedicated workers (roughly 25% of one core each). Both worker loops now
collect after work, preserve checkpoints around actual tasks, and sleep when idle.
Dedicated-worker messages and timers still wake promptly. The standalone idle
probe now runs in CI to catch this operational regression.

The scope is a correctness/resource fix in ordinary Rust. No assembly kernel or
CPU-specific dispatch is justified by this work.

## Relevant layout Criterion check

Ten samples, one-second warmup/measurement, compared with the locally cached
Criterion baseline on the same host (not a fresh isolated historical checkout):

| Case | Estimate | Median change |
|---|---|---|
| 100 paragraphs | 1.5486 ms | +1.64% |
| resize 100 paragraphs | 1.5794 ms | +2.66% |
| advanced workshop | 1.6462 ms | +0.85% |
| incremental card | 4.9877 ms | +0.28% |

The two small flow deltas are statistically detectable; the advanced cases stay
within Criterion's noise threshold. No observed regression reaches the 20%
review gate. Retained glyph cluster metadata supports native find geometry.
