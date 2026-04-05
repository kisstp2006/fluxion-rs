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

// Reinhard global tonemapping.
fn reinhard(x: vec3<f32>) -> vec3<f32> {
    return x / (x + vec3<f32>(1.0));
}

// AgX tonemapping — perceptual, neutral highlights (Blender-style).
// Based on Troy Sobotka's AgX transform, simplified approximation.
fn agx(x: vec3<f32>) -> vec3<f32> {
    // Inset matrix (sRGB → AgX log space)
    let m = mat3x3<f32>(
        vec3<f32>(0.842479062253094,  0.0423282422610123, 0.0423756549057051),
        vec3<f32>(0.0784335999999992, 0.878468636469772,  0.0784336),
        vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104),
    );
    let min_ev = -12.47393;
    let max_ev =  4.026069;
    var v = m * max(x, vec3<f32>(0.0));
    v = clamp(log2(v + vec3<f32>(0.0000001)), vec3<f32>(min_ev), vec3<f32>(max_ev));
    v = (v - min_ev) / (max_ev - min_ev);
    // Sigmoid contrast curve
    let c1 = -0.202116101926749;
    let c2 =  0.501825783573368;
    let c3 =  1.01007777459823;
    let c4 = -0.0275826090694;
    let c5 =  0.228086381506378;
    let t  = v;
    v = t * (t * (t * (t * c4 + c3) + c2) + c1) + c5;
    // Outset matrix (AgX log → sRGB)
    let mi = mat3x3<f32>(
        vec3<f32>( 1.19687900512661,  -0.0528968517574562, -0.0529716355144492),
        vec3<f32>(-0.0980208811401368, 1.15190312990417,   -0.0980434501171241),
        vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116),
    );
    v = mi * v;
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
    if params.tone_mode == TONE_NONE {
        ldr_color = clamp(hdr_color, vec3<f32>(0.0), vec3<f32>(1.0));
    } else if params.tone_mode == TONE_LINEAR {
        ldr_color = clamp(hdr_color, vec3<f32>(0.0), vec3<f32>(1.0));
    } else if params.tone_mode == TONE_REINHARD {
        ldr_color = reinhard(hdr_color);
    } else if params.tone_mode == TONE_AGX {
        ldr_color = agx(hdr_color);
    } else {
        ldr_color = aces_narkowicz(hdr_color);
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
