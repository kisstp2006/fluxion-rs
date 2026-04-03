// ============================================================
// bloom_bright.wgsl — Bloom bright-pass extraction
//
// Extracts pixels above a luminance threshold for the bloom effect.
// Uses a smooth knee curve (soft threshold) to avoid hard cutoffs
// that produce ringing artifacts.
//
// This matches the bloom_bright pass from the TypeScript engine's
// PostProcessing.ts, ported from Three.js UnrealBloomPass.
// ============================================================

struct BloomBrightParams {
    threshold:  f32,  // Luminance threshold. Pixels below this are set to black.
    soft_knee:  f32,  // Smoothing width around threshold (0 = hard, 1 = very soft)
    _pad0:      f32,
    _pad1:      f32,
}

@group(0) @binding(0) var<uniform> params:   BloomBrightParams;
@group(0) @binding(1) var          hdr_tex:  texture_2d<f32>;
@group(0) @binding(2) var          tex_samp: sampler;

struct FragInput {
    @builtin(position) frag_coord: vec4<f32>,
    @location(0)       uv:         vec2<f32>,
}

// Perceptual luminance (BT.709 coefficients, same as Three.js luminance)
fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Soft knee threshold function.
// Returns 0 for lum << threshold, ramps up smoothly around threshold,
// and returns lum - threshold for lum >> threshold.
fn threshold_filter(color: vec3<f32>, threshold: f32, knee: f32) -> vec3<f32> {
    let lum = luminance(color);

    // Knee region: [threshold - knee/2, threshold + knee/2]
    let knee_low  = threshold - knee * 0.5;
    let knee_high = threshold + knee * 0.5;

    var weight: f32;
    if lum < knee_low {
        // Below knee: no bloom
        weight = 0.0;
    } else if lum >= knee_high {
        // Above knee: full bloom contribution
        weight = 1.0;
    } else {
        // Smooth transition through knee using a quadratic curve
        let t = (lum - knee_low) / max(knee_high - knee_low, 0.0001);
        weight = t * t;
    }

    return color * weight;
}

@fragment
fn fs_main(in: FragInput) -> @location(0) vec4<f32> {
    let color = textureSample(hdr_tex, tex_samp, in.uv).rgb;
    let bright = threshold_filter(color, params.threshold, params.soft_knee);
    return vec4<f32>(bright, 1.0);
}
