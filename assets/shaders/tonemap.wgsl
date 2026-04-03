// ============================================================
// tonemap.wgsl — ACES tonemapping + post-process compositing
//
// Final full-screen pass. Reads the HDR scene texture (output of the
// lighting and post-processing chain) and writes the final LDR result
// to the surface.
//
// Operations (in order):
//   1. ACES filmic tonemapping — compresses HDR to [0,1] display range
//   2. Gamma correction        — linear → sRGB (^(1/2.2))
//   3. Vignette                — darkens screen edges
//   4. Chromatic aberration    — color fringe at edges (optional, stylistic)
//   5. Film grain              — subtle noise for film-like look (optional)
//
// This matches the composite pass design from the TypeScript engine's
// PostProcessing.ts, ported to WGSL.
// ============================================================

// ── Uniforms ──────────────────────────────────────────────────────────────────

struct TonemapParams {
    exposure:             f32,  // EV stops, 1.0 = no change. Multiply before tonemapping.
    vignette_intensity:   f32,  // 0.0 = no vignette, 1.0 = full dark edges
    vignette_roundness:   f32,  // 0.0 = square, 1.0 = circular
    chromatic_aberration: f32,  // Pixel offset of R/B channels at edges. 0.0 = off.
    film_grain:           f32,  // Grain intensity. 0.0 = off, 0.05 = subtle.
    time:                 f32,  // Engine time in seconds (animates film grain)
    _pad0:                f32,
    _pad1:                f32,
}

@group(0) @binding(0) var<uniform> params:   TonemapParams;
@group(0) @binding(1) var          hdr_tex:  texture_2d<f32>;
@group(0) @binding(2) var          tex_samp: sampler;

// ── Vertex input ──────────────────────────────────────────────────────────────

struct FragInput {
    @builtin(position) frag_coord: vec4<f32>,
    @location(0)       uv:         vec2<f32>,
}

// ── ACES filmic tonemapping ───────────────────────────────────────────────────
//
// Approximation by Krzysztof Narkowicz:
//   https://knarkowicz.wordpress.com/2016/01/06/aces-filmic-tone-mapping-curve/
// Provides a pleasing S-curve that lifts shadows and compresses highlights,
// matching the look of ACES film stock.

fn aces_narkowicz(x: vec3<f32>) -> vec3<f32> {
    let a: f32 = 2.51;
    let b: f32 = 0.03;
    let c: f32 = 2.43;
    let d: f32 = 0.59;
    let e: f32 = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

// ── Film grain (pseudo-random noise) ─────────────────────────────────────────
//
// Simple hash-based grain that animates with time.
// Using a hash instead of a texture avoids a sampler bind.

fn hash(p: vec2<f32>) -> f32 {
    var p2 = fract(p * vec2<f32>(234.34, 435.345));
    p2 += dot(p2, p2 + 34.23);
    return fract(p2.x * p2.y);
}

fn film_grain(uv: vec2<f32>, time: f32, intensity: f32) -> f32 {
    let noise = hash(uv + fract(time * 0.1));
    return (noise - 0.5) * intensity;
}

// ── Vignette ──────────────────────────────────────────────────────────────────
//
// Darkens the screen corners. The roundness parameter blends between
// a square vignette (0.0) and a circular one (1.0).

fn vignette(uv: vec2<f32>, intensity: f32, roundness: f32) -> f32 {
    // Distance from center [0..~0.707]
    let d = mix(
        max(abs(uv.x - 0.5), abs(uv.y - 0.5)),   // square (L∞ norm)
        length(uv - vec2<f32>(0.5)),               // circular (L2 norm)
        roundness,
    );
    // Smooth falloff from edge inward
    let vignette_radius = 0.5 - intensity * 0.25;
    let edge = smoothstep(0.5, vignette_radius, d);
    return mix(1.0, edge, intensity);
}

// ── Chromatic aberration ──────────────────────────────────────────────────────
//
// Offsets the R and B channels towards the screen edges, simulating the
// lens fringing found in real cameras.

fn sample_with_aberration(uv: vec2<f32>, offset: f32) -> vec3<f32> {
    if offset <= 0.0 { return textureSample(hdr_tex, tex_samp, uv).rgb; }

    let center = uv - 0.5;
    let dist   = length(center);
    let dir    = normalize(center + vec2<f32>(0.0001)); // avoid div by zero at center

    // Scale offset by distance from center (stronger at edges)
    let pixel_offset = dir * dist * offset;

    let r = textureSample(hdr_tex, tex_samp, uv + pixel_offset).r;
    let g = textureSample(hdr_tex, tex_samp, uv).g;
    let b = textureSample(hdr_tex, tex_samp, uv - pixel_offset).b;
    return vec3<f32>(r, g, b);
}

// ── Fragment shader ───────────────────────────────────────────────────────────

@fragment
fn fs_main(in: FragInput) -> @location(0) vec4<f32> {
    let uv = in.uv;

    // Get screen dimensions for pixel-space aberration offset
    let screen_size = vec2<f32>(textureDimensions(hdr_tex));
    let pixel_offset = params.chromatic_aberration / screen_size.x;

    // Sample HDR scene (with optional chromatic aberration)
    var hdr_color = sample_with_aberration(uv, pixel_offset);

    // Apply exposure (exposure > 1.0 = brighter, < 1.0 = darker)
    hdr_color *= params.exposure;

    // ACES filmic tonemapping: HDR → LDR
    var ldr_color = aces_narkowicz(hdr_color);

    // Gamma correction: linear → sRGB
    // sRGB exact transfer function would use pow(x, 1/2.4) for x > 0.0031308,
    // but pow(x, 1/2.2) is a good and fast approximation.
    ldr_color = pow(max(ldr_color, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));

    // Vignette
    let vign = vignette(uv, params.vignette_intensity, params.vignette_roundness);
    ldr_color *= vign;

    // Film grain (additive noise)
    let grain = film_grain(uv, params.time, params.film_grain);
    ldr_color += vec3<f32>(grain);

    return vec4<f32>(clamp(ldr_color, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
