# Phase 4 performance baseline

Host: Linux aarch64 / Apple M2 Pro (G14S B1), Vulkan; rustc 1.99.0. Release
profile uses opt-level=3, thin LTO, 16 codegen units and unwind. No native CPU
flag or own CPU SIMD/asm. These are local baseline measurements, not a claim
of broad hardware coverage.

## Criterion

```sh
cargo bench -p layout --bench flow -- --sample-size 10 --warm-up-time 0.3 --measurement-time 1
cargo bench -p paint --bench composite -- --sample-size 10 --warm-up-time 0.3 --measurement-time 1
```

| Benchmark | 95% interval |
|---|---|
| flow/100_paragraphs | 1.3824–1.3884 ms (estimate 1.3860) |
| flow/resize_100_paragraphs | 1.3940–1.5307 ms (estimate 1.4281) |
| paint/software_scroll_1024x768 | 1.5930–1.5960 ms (estimate 1.5942) |

The typical resize goal (<100 ms) is met. The resize sample had one severe high
outlier; scalar paint had three mild/severe outliers, recorded by Criterion.

## Offline end-to-end and K4 probe

`cargo run --release -p shell --example phase4_probe -- ../plan/evidence/phase-4`
creates software snapshots and prints stage and CPU/GPU times.

Synthetic page: **1,048,754 bytes**, **61,694 DOM nodes**, **7,713 boxes**,
**7,711 lines**, **786,522 glyphs**. Representative measured stages:

| Stage | ms |
|---|---:|
| HTML parse | 14.917 |
| computed CSS | 380.585 |
| render tree + flow + glyph preparation | 234.190 |
| visible scalar paint | 4.290 |

RSS after stages: **479,280 KiB**. Full large-document CSS/layout are costlier
than typical-page layout and now run off the UI thread. They remain an explicit
optimization/memory baseline for later phases.

An initial run measured 1063.968 ms in the layout/preparation stage. Avoiding a
full computed-property-map clone for each fully opaque word reduced it by ~78%
while keeping output semantics and all tests. No CPU-specific kernel was needed.

K4 scroll workload: a scaled semitransparent image plus alpha-filled strips,
1024×768, 60 scroll positions. CPU median **9.675 ms**, p95 **9.773 ms**; GPU
**including synchronized readback** median **0.848 ms**, p95 **2.470 ms**.
Median gain **91.2%**. Hardware parity uses the same display-list source with
a maximum RGB channel tolerance of 3 at scales 1 and 1.5.

A final probe rerun after the last source changes recorded parse **18.574 ms**,
CSS **382.545 ms**, layout/preparation **231.681 ms**, paint **4.424 ms**,
RSS **476,960 KiB**. K4 CPU median/p95 **9.679/9.744 ms**, GPU with readback
**0.879/0.980 ms**, median gain **90.9%**. Both runs are retained as baseline
observations; the command includes synchronized GPU readback in its timing.

## Actual graphical counters

Controlled local HTTP streams a 1 MiB document with the tail delayed:

```text
PERF first_paint_ms=35.336 before_eof=true
PERF frame=4 renderer=software layout_passes=1 layout_ms=6.527 frame_ms=3.393 scroll=0.0 zoom=1.00
PERF renderer=gpu adapter=Apple M2 Pro (G14S B1) (Vulkan)
PERF frame=22 renderer=gpu layout_passes=5 layout_ms=246.453 frame_ms=2.269 scroll=0.0 zoom=1.00
PERF frame=26 renderer=gpu layout_passes=5 layout_ms=246.453 frame_ms=3.652 scroll=864.0 zoom=1.00
PERF frame=27 renderer=gpu layout_passes=5 layout_ms=246.453 frame_ms=5.277 scroll=1728.0 zoom=1.00
PERF frame=85 renderer=gpu layout_passes=5 layout_ms=246.453 frame_ms=5.158 scroll=51840.0 zoom=1.00
```

Sixty PageDown updates: layout counter stays **5** throughout, work per frame
**2.097–8.344 ms**. This measures frame work and retention, not a wall-clock
FPS statistic of the input generator; FIFO VSync supplies 60Hz presentation.

Forced software demo:

```text
PERF first_paint_ms=34.066 before_eof=false
PERF frame=4 renderer=software layout_passes=2 layout_ms=1.791 frame_ms=7.132 scroll=0.0 zoom=1.00
```

The small fixture ends before first paint naturally. The deterministic streaming
test holds EOF until actual pixels exist, separately proving progressive paint.
