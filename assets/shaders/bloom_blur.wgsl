// ============================================================
// bloom_blur.wgsl — Kawase dual-pass blur
//
// Used for bloom. A single pass of this shader blurs the input
// texture. Running it multiple times at decreasing then increasing
// resolutions (downsample → upsample) creates the bloom spread.
//
// Kawase blur: samples 4 neighbors at ±(offset+0.5) texels in
// both X and Y. Compared to Gaussian blur it requires fewer taps
// for the same radius and works well for bloom.
// ============================================================

struct BlurParams {
    /// How far (in texels) to offset the 4 sample positions.
    /// Increase for wider blur. Pass 0, 1, 2, 3 on successive calls.
    iteration: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<uniform> params:   BlurParams;
@group(0) @binding(1) var          src_tex:  texture_2d<f32>;
@group(0) @binding(2) var          tex_samp: sampler;

struct FragInput {
    @builtin(position) frag_coord: vec4<f32>,
    @location(0)       uv:         vec2<f32>,
}

@fragment
fn fs_main(in: FragInput) -> @location(0) vec4<f32> {
    let tex_size  = vec2<f32>(textureDimensions(src_tex));
    let texel     = 1.0 / tex_size;
    let offset    = (f32(params.iteration) + 0.5) * texel;

    // Sample 4 neighbors (bilinear, so each already averages a 2×2 block)
    var color = vec4<f32>(0.0);
    color += textureSample(src_tex, tex_samp, in.uv + vec2<f32>( offset.x,  offset.y));
    color += textureSample(src_tex, tex_samp, in.uv + vec2<f32>(-offset.x,  offset.y));
    color += textureSample(src_tex, tex_samp, in.uv + vec2<f32>( offset.x, -offset.y));
    color += textureSample(src_tex, tex_samp, in.uv + vec2<f32>(-offset.x, -offset.y));
    return color * 0.25;
}
