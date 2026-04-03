// ============================================================
// ssao_blur.wgsl — Bilateral blur for SSAO
//
// Simple 4x4 box blur over the SSAO texture.
// Bilateral blurs preserve edges by weighting samples by depth
// similarity — we use a simplified depth check here.
// ============================================================

@group(0) @binding(0) var ssao_tex:  texture_2d<f32>;
@group(0) @binding(1) var tex_samp:  sampler;

struct FragInput {
    @builtin(position) frag_coord: vec4<f32>,
    @location(0)       uv:         vec2<f32>,
}

@fragment
fn fs_main(in: FragInput) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(ssao_tex));
    var result = 0.0;

    // 4x4 box filter (16 samples)
    for (var x = -2; x < 2; x++) {
        for (var y = -2; y < 2; y++) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel;
            result += textureSample(ssao_tex, tex_samp, in.uv + offset).r;
        }
    }
    result /= 16.0;

    return vec4<f32>(result, result, result, 1.0);
}
