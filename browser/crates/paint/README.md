# Display lists and GPU/software paint

`build_display_list(&LayoutResult)` emits fills, text runs, images, borders,
clips, scroll frames and opacity contexts independently of any window.
`Rasterizer::prepare` rasterizes/shares new glyphs once. A retained `Scene`
resolves viewport/scroll/zoom offsets without text shaping or layout.
`composite(&mut Backend, &DisplayList, Rect)` accepts GPU or software targets.

- GPU: wgpu with Vulkan/GL, FIFO VSync, atlas-batched glyphs/fills and cached
  large images. Versioned K4 WGSL under `shaders/`; no own CPU SIMD/asm.
- Scalar: clipped fills, straight-alpha sRGB blending, bilinear image scaling
  and basic uniform rounded rectangles. GPU parity is tested at two scales,
  with text, images, nested scroll offsets, backgrounds and borders.
- Window initialization runs off the UI. Wayland swapchain configuration is
  deferred until softbuffer releases its surface, preserving explicit-sync
  acquire-fence semantics. Device/presentation failure selects the fallback.
- Auxiliary PNG/JPEG/GIF/WebP decoders and SVG-as-image via resvg/usvg;
  8 MiB encoded, 4096px axes, 64 MiB decoded, 100 MiB decoded-image LRU.
  The shell caps a page at 128 requests and 64 MiB encoded image bodies.
- SVG external/file/network image references are disabled. Local system fonts
  are available for SVG text. XML depth/nodes, geometry magnitude/number count
  and output pixel caps are checked before expensive path expansion/rasterization.
- Glyph raster cache capped at 64 MiB, with oversized glyphs rasterized at a
  bounded size and scaled. Resource-limited image failures keep visible alt text.

## Measured Stage A boundaries

Full-frame painting is retained; damage tracking and advanced stacking/layers
arrive later. Opacity currently multiplies primitive alpha rather than using
isolated offscreen group surfaces. Borders use four sides and uniform rounded
outer bounds; full elliptical/corner-specific radii and advanced border styles
are compatibility work. GIF/WebP use their first frame. SVG nested images are
isolated along with external resource references. No SVG scripts execute.

Run CPU tests with `cargo test -p paint`; explicitly run adapter parity with
`cargo test -p paint --test backends gpu_and_software_render_the_same_display_list -- --ignored --exact`.
`cargo bench -p paint --bench composite` measures the scalar baseline. The shell
`phase4_probe` example compares CPU with GPU including readback and creates
offline snapshots when passed an existing output directory.
