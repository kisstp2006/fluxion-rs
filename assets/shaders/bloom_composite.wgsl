// ============================================================
// bloom_composite.wgsl — Additive bloom blend
//
// Adds the blurred bloom texture on top of the HDR scene.
// This is the final bloom step. The scene remains HDR — tonemapping
// happens later in tonemap.wgsl.
// ============================================================

struct BloomCompositeParams {
    strength: f32,  // Bloom intensity multiplier (0.3 = subtle, 1.0 = strong)
    _pad0:    f32,
    _pad1:    f32,
    _pad2:    f32,
}

@group(0) @binding(0) var<uniform> params:      BloomCompositeParams;
@group(0) @binding(1) var          hdr_tex:     texture_2d<f32>;
@group(0) @binding(2) var          bloom_tex:   texture_2d<f32>;
@group(0) @binding(3) var          tex_samp:    sampler;

struct FragInput {
    @builtin(position) frag_coord: vec4<f32>,
    @location(0)       uv:         vec2<f32>,
}

@fragment
fn fs_main(in: FragInput) -> @location(0) vec4<f32> {
    let scene = textureSample(hdr_tex,   tex_samp, in.uv).rgb;
    let bloom = textureSample(bloom_tex, tex_samp, in.uv).rgb;
    // Additive blend — bloom adds luminance without changing hue
    return vec4<f32>(scene + bloom * params.strength, 1.0);
}
