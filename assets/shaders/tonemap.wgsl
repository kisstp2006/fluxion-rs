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

// tone_mode constants (match ToneMapMode::as_u32 in Rust)
const TONE_NONE:     u32 = 0u;
const TONE_LINEAR:   u32 = 1u;
const TONE_REINHARD: u32 = 2u;
const TONE_ACES:     u32 = 3u;
const TONE_AGX:      u32 = 4u;
const TONE_UCHIMURA: u32 = 5u;

struct TonemapParams {
    exposure:             f32,  // EV stops, 1.0 = no change. Multiply before tonemapping.
    vignette_intensity:   f32,  // 0.0 = no vignette, 1.0 = full dark edges
    vignette_roundness:   f32,  // 0.0 = square, 1.0 = circular
    chromatic_aberration: f32,  // Pixel offset of R/B channels at edges. 0.0 = off.
    film_grain:           f32,  // Grain intensity. 0.0 = off, 0.05 = subtle.
    time:                 f32,  // Engine time in seconds (animates film grain)
    tone_mode:            u32,  // ToneMapMode (0=None 1=Linear 2=Reinhard 3=ACES 4=AgX)
    _pad0:                f32,
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
// Stephen Hill / MJP Baking Lab RRT+ODT fitted ACES.
// Uses proper AP1 input/output matrices + RRT+ODT rational approximation.
// Source: https://github.com/TheRealMJP/BakingLab/blob/master/BakingLab/ACES.hlsl
// This is the same implementation used by WickedEngine, Unreal Engine, etc.
//
// sRGB → XYZ → D65_2_D60 → AP1 → RRT_SAT
const ACES_INPUT_MAT: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.59719, 0.07600, 0.02840),
    vec3<f32>(0.35458, 0.90834, 0.13383),
    vec3<f32>(0.04823, 0.01566, 0.83777),
);
// ODT_SAT → XYZ → D60_2_D65 → sRGB
const ACES_OUTPUT_MAT: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>( 1.60475, -0.10208, -0.00327),
    vec3<f32>(-0.53108,  1.10813, -0.07276),
    vec3<f32>(-0.07367, -0.00605,  1.07602),
);

fn aces_rrt_odt_fit(v: vec3<f32>) -> vec3<f32> {
    let a = v * (v + vec3<f32>(0.0245786)) - vec3<f32>(0.000090537);
    let b = v * (0.983729 * v + vec3<f32>(0.4329510)) + vec3<f32>(0.238081);
    return a / b;
}

fn aces_fitted(color: vec3<f32>) -> vec3<f32> {
    var c = ACES_INPUT_MAT * color;
    // Clamp to avoid half-precision issues near very bright point light centers
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(100.0));
    c = aces_rrt_odt_fit(c);
    c = ACES_OUTPUT_MAT * c;
    return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
}

// Reinhard global (simple, fast).
fn reinhard(x: vec3<f32>) -> vec3<f32> {
    return x / (x + vec3<f32>(1.0));
}

// Reinhard Extended — luminance-based, preserves hue under saturation.
// Max white Lw can be tuned; 4.0 works well for typical HDR scenes.
fn reinhard_extended(x: vec3<f32>) -> vec3<f32> {
    let lw: f32 = 4.0;
    let lum = dot(x, vec3<f32>(0.2126, 0.7152, 0.0722));
    let lum_out = (lum * (1.0 + lum / (lw * lw))) / (1.0 + lum);
    let scale = select(lum_out / lum, 0.0, lum <= 0.0);
    return clamp(x * scale, vec3<f32>(0.0), vec3<f32>(1.0));
}

// Uchimura tone mapping (Gran Turismo 7 style).
// Reference: "Driving Toward Reality: Physically Based Tone Mapping and Perceptual
//             Fidelity in Gran Turismo 7"
// https://www.desmos.com/calculator/gslcdxvipg
fn uchimura_curve(x: f32, P: f32, a: f32, m: f32, l: f32, c: f32, b: f32) -> f32 {
    let l0  = ((P - m) * l) / a;
    let L0  = m - m / a;
    let L1  = m + (1.0 - m) / a;
    let S0  = m + l0;
    let S1  = m + a * l0;
    let C2  = (a * P) / (P - S1);
    let CP  = -C2 / P;
    let w0  = 1.0 - smoothstep(0.0, m, x);
    let w2  = step(m + l0, x);
    let w1  = 1.0 - w0 - w2;
    let T   = m * pow(max(x / m, 0.0001), c) + b;
    let S   = P - (P - S1) * exp(CP * (x - S0));
    let L   = m + a * (x - m);
    return T * w0 + L * w1 + S * w2;
}

fn uchimura(color: vec3<f32>) -> vec3<f32> {
    // Default parameters tuned for GT7 look
    let P: f32 = 1.0;   // Max brightness
    let a: f32 = 1.0;   // Contrast
    let m: f32 = 0.22;  // Linear section start
    let l: f32 = 0.4;   // Linear section length
    let c: f32 = 1.33;  // Black tightness
    let b: f32 = 0.0;   // Pedestal
    return vec3<f32>(
        uchimura_curve(color.r, P, a, m, l, c, b),
        uchimura_curve(color.g, P, a, m, l, c, b),
        uchimura_curve(color.b, P, a, m, l, c, b),
    );
}

// AgX tonemapping — perceptual, neutral highlights (Blender-style).
// Based on Troy Sobotka's AgX transform.
// Inset/outset matrices from https://iolite-engine.com/blog_posts/minimal_agx_implementation
const AGX_INSET_MAT: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.842479062253094,  0.0423282422610123, 0.0423756549057051),
    vec3<f32>(0.0784335999999992, 0.878468636469772,  0.0784336),
    vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104),
);
const AGX_OUTSET_MAT: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>( 1.19687900512661,  -0.0528968517574562, -0.0529716355144492),
    vec3<f32>(-0.0980208811401368, 1.15190312990417,   -0.0980434501171241),
    vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116),
);

fn agx_default_contrast_approx(x: vec3<f32>) -> vec3<f32> {
    // Polynomial sigmoid in Horner form — correctly ordered
    // Coefficients: c0=0.228, c1=-0.202, c2=0.502, c3=1.010, c4=-0.028
    let x2 = x * x;
    let x4 = x2 * x2;
    return 0.228086381506378 +
           x  * (-0.202116101926749 +
           x  * ( 0.501825783573368 +
           x  * ( 1.01007777459823  +
           x  * (-0.0275826090694))));
}

fn agx(color: vec3<f32>) -> vec3<f32> {
    let min_ev = -12.47393;
    let max_ev =  4.026069;
    var v = AGX_INSET_MAT * max(color, vec3<f32>(0.0));
    v = clamp(log2(v + vec3<f32>(0.0000001)), vec3<f32>(min_ev), vec3<f32>(max_ev));
    v = (v - min_ev) / (max_ev - min_ev);
    v = agx_default_contrast_approx(v);
    v = AGX_OUTSET_MAT * v;
    return clamp(v, vec3<f32>(0.0), vec3<f32>(1.0));
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

    // Tonemapping: HDR → LDR (mode selected from uniform)
    var ldr_color: vec3<f32>;
    if params.tone_mode == TONE_NONE || params.tone_mode == TONE_LINEAR {
        ldr_color = clamp(hdr_color, vec3<f32>(0.0), vec3<f32>(1.0));
    } else if params.tone_mode == TONE_REINHARD {
        ldr_color = reinhard_extended(hdr_color);
    } else if params.tone_mode == TONE_ACES {
        ldr_color = aces_fitted(hdr_color);
    } else if params.tone_mode == TONE_AGX {
        ldr_color = agx(hdr_color);
    } else if params.tone_mode == TONE_UCHIMURA {
        ldr_color = uchimura(hdr_color);
    } else {
        ldr_color = aces_fitted(hdr_color);
    }

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
