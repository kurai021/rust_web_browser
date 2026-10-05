// K4: GPU fill, straight-alpha blending and bilinear image/glyph scaling.
// CPU parity is verified by paint/tests/backends.rs; no CPU SIMD/asm enabled.
struct Screen { size: vec4<f32> };
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var tex_sampler: sampler;
@group(0) @binding(2) var<uniform> screen: Screen;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) rounded: vec4<f32>,
    @location(3) @interpolate(flat) radius: f32,
};
@vertex fn vertex(
    @location(0) position: vec2<f32>, @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>, @location(3) rounded: vec4<f32>,
    @location(4) radius: f32,
) -> Out {
    var out: Out;
    out.position = vec4<f32>(position.x / screen.size.x * 2.0 - 1.0, 1.0 - position.y / screen.size.y * 2.0, 0.0, 1.0);
    out.uv = uv; out.color = color; out.rounded = rounded; out.radius = radius;
    return out;
}
@fragment fn fragment(in: Out) -> @location(0) vec4<f32> {
    if in.radius > 0.0 {
        let radius = min(in.radius, min(in.rounded.z, in.rounded.w) * 0.5);
        let center = clamp(in.position.xy, in.rounded.xy + vec2<f32>(radius), in.rounded.xy + in.rounded.zw - vec2<f32>(radius));
        let d = in.position.xy - center;
        if dot(d, d) > radius * radius { discard; }
    }
    return textureSample(tex, tex_sampler, in.uv) * in.color;
}
