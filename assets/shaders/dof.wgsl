// ============================================================
// dof.wgsl — Depth of Field (Bokeh DoF, single-pass approximation)
//
// Reads the HDR scene + linear depth and outputs a CoC-blurred
// version of the scene. Blurs out-of-focus regions using a
// disc-shaped kernel weighted by circle-of-confusion radius.
//
// Bindings:
//   group(0) binding(0) — DofParams uniform
//   group(0) binding(1) — hdr_tex      (HDR scene, Rgba16Float)
//   group(0) binding(2) — depth_tex    (Depth32Float)
//   group(0) binding(3) — tex_sampler
// ============================================================

struct DofParams {
    focus_distance: f32,   // distance to in-focus plane (world units)
    aperture:       f32,   // lens aperture — controls CoC scale (0 = off)
    max_blur:       f32,   // max blur radius in pixels
    near_plane:     f32,
    far_plane:      f32,
    _pad0:          f32,
    _pad1:          f32,
    _pad2:          f32,
}

@group(0) @binding(0) var<uniform> params:     DofParams;
@group(0) @binding(1) var          hdr_tex:    texture_2d<f32>;
@group(0) @binding(2) var          depth_tex:  texture_depth_2d;
@group(0) @binding(3) var          tex_samp:   sampler;

struct FragInput {
    @builtin(position) frag_coord: vec4<f32>,
    @location(0)       uv:         vec2<f32>,
}

// Convert NDC depth to linear view-space depth [near, far]
fn linearize_depth(depth: f32, near: f32, far: f32) -> f32 {
    return (near * far) / (far - depth * (far - near));
}

// Circle of Confusion radius in normalised UV space.
// Positive = behind focus, negative = in front.
fn coc_radius(linear_depth: f32) -> f32 {
    let dist = linear_depth - params.focus_distance;
    return clamp(params.aperture * dist / max(linear_depth, 0.0001), -1.0, 1.0);
}

// ── Disc kernel (Poisson disc, 12 samples) ────────────────────────────────────
// Poisson disc offsets in unit circle
const KERNEL_COUNT: i32 = 12;
fn kernel_offset(i: i32) -> vec2<f32> {
    switch i {
        case  0: { return vec2<f32>( 0.000000,  0.500000); }
        case  1: { return vec2<f32>( 0.433013,  0.250000); }
        case  2: { return vec2<f32>( 0.433013, -0.250000); }
        case  3: { return vec2<f32>( 0.000000, -0.500000); }
        case  4: { return vec2<f32>(-0.433013, -0.250000); }
        case  5: { return vec2<f32>(-0.433013,  0.250000); }
        case  6: { return vec2<f32>( 0.250000,  0.433013); }
        case  7: { return vec2<f32>(-0.250000,  0.433013); }
        case  8: { return vec2<f32>(-0.250000, -0.433013); }
        case  9: { return vec2<f32>( 0.250000, -0.433013); }
        case 10: { return vec2<f32>( 0.000000,  0.000000); }
        case 11: { return vec2<f32>( 0.100000,  0.200000); }
        default: { return vec2<f32>(0.0, 0.0); }
    }
}

@fragment
fn fs_main(in: FragInput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let screen_size = vec2<f32>(textureDimensions(hdr_tex));
    let texel_size  = 1.0 / screen_size;

    // Sample depth at this pixel
    let raw_depth    = textureSample(depth_tex, tex_samp, uv);
    let linear_d     = linearize_depth(raw_depth, params.near_plane, params.far_plane);

    // Circle of Confusion for this pixel
    let coc   = coc_radius(linear_d);
    let blur_r = abs(coc) * params.max_blur;

    // If blur radius is negligible, return sharp sample
    if blur_r < 0.5 {
        return textureSample(hdr_tex, tex_samp, uv);
    }

    // Accumulate blurred colour using disc kernel
    var col_sum   = vec3<f32>(0.0);
    var weight_sum = 0.0;

    for (var i: i32 = 0; i < KERNEL_COUNT; i++) {
        let offset    = kernel_offset(i) * blur_r * texel_size;
        let sample_uv = uv + offset;

        // Sample neighbour depth to avoid bleeding sharp foreground into background
        let nd = textureSample(depth_tex, tex_samp, sample_uv);
        let nl = linearize_depth(nd, params.near_plane, params.far_plane);
        let nc = coc_radius(nl);

        // Weight: accept neighbour if its CoC overlaps center pixel
        let w = select(0.0, 1.0, abs(nc) >= abs(coc) * 0.75);

        col_sum    += textureSample(hdr_tex, tex_samp, sample_uv).rgb * w;
        weight_sum += w;
    }

    if weight_sum < 0.0001 {
        return textureSample(hdr_tex, tex_samp, uv);
    }
    return vec4<f32>(col_sum / weight_sum, 1.0);
}
