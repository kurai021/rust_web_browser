# Phase 4 implementation-side kernel registry

Living implementation appendix to sealed plan/15. The plan's closed list is
preserved. The planned GPU renderer supplies K4; the portable CPU path remains
the fallback. No hand-written CPU SIMD/asm or target-cpu=native was enabled.

| Kernel | Source | Backends | Verification | Status |
|---|---|---|---|---|
| K4 fill/blend/bilinear composite | `browser/crates/paint/shaders/quads.wgsl`, `src/gpu.rs`, `src/software.rs` | Vulkan/GL wgpu + scalar CPU | parity at 1×/1.5×, Criterion scalar, 60-sample CPU/GPU probe | active Phase 4 baseline |
| K5 decode/glyph reuse | `paint/src/images.rs`, `paint/src/lib.rs`, cosmic-text/image/resvg | approved auxiliary crates | format tests, glyph/scene tests, decoder fuzz | library reuse; no custom CPU kernel |

Measured K4 median gain: **91.2%** on the recorded workload, with synchronized
GPU readback included. It is not a scalar-vs-SIMD result: there is no custom
SIMD implementation to compare. See `performance.md` for workloads and timings.
`--software-render` is the backend kill switch. Both root feature configurations
pass the same suite; shaders remain versioned and CPU/GPU source lists identical.
