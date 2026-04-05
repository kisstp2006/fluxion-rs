// ============================================================
// skybox.wgsl — Multi-mode sky renderer
//
// Modes (sky_params.sky_mode):
//   0 = Gradient  — horizon/zenith color gradient + simple sun disc
//   1 = Preetham  — analytical atmospheric scattering (Preetham 1999)
//   2 = SolidColor — flat clear color, no sky
//   3 = Panorama  — equirectangular texture sampled by view direction
//
// Renders only sky pixels (depth == far plane).
// ============================================================

struct SkyParams {
    horizon_color:     vec3<f32>,
    sky_mode:          u32,         // 0=gradient 1=preetham 2=solid 3=panorama
    zenith_color:      vec3<f32>,
    _pad1:             f32,
    sun_direction:     vec3<f32>,   // normalised, toward sun
    sun_intensity:     f32,
    sun_size:          f32,         // angular radius (radians)
    _pad2a:            f32,         // padding to align solid_color to 16-byte boundary
    _pad2b:            f32,
    _pad2c:            f32,
    solid_color:       vec3<f32>,   // used in SolidColor mode (offset 64)
    turbidity:         f32,
    rayleigh:          f32,
    mie_coefficient:   f32,
    mie_directional_g: f32,
    _pad3:             f32,
}

struct CameraUniforms {
    view_proj:     mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos:    vec3<f32>,
    _pad:          f32,
}

@group(0) @binding(0) var<uniform> sky_params:   SkyParams;
@group(0) @binding(1) var<uniform> camera:        CameraUniforms;
@group(0) @binding(2) var          depth_tex:     texture_depth_2d;
@group(0) @binding(3) var          panorama_tex:  texture_2d<f32>;
@group(0) @binding(4) var          panorama_samp: sampler;

struct FragInput {
    @builtin(position) frag_coord: vec4<f32>,
    @location(0)       uv:         vec2<f32>,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn reconstruct_view_dir(uv: vec2<f32>) -> vec3<f32> {
    let ndc   = vec4<f32>(uv * 2.0 - 1.0, 0.5, 1.0);
    let ndc_f = vec4<f32>(ndc.x, -ndc.y, ndc.z, 1.0);
    let world = camera.inv_view_proj * ndc_f;
    return normalize(world.xyz / world.w - camera.camera_pos);
}

fn sun_disc(view_dir: vec3<f32>) -> vec3<f32> {
    let cos_a = dot(view_dir, sky_params.sun_direction);
    let angle = acos(clamp(cos_a, -1.0, 1.0));
    if angle < sky_params.sun_size {
        let f = 1.0 - angle / sky_params.sun_size;
        return vec3<f32>(1.0, 0.95, 0.8) * sky_params.sun_intensity * f * f;
    }
    return vec3<f32>(0.0);
}

// ── Gradient sky ─────────────────────────────────────────────────────────────

fn sky_gradient(view_dir: vec3<f32>) -> vec3<f32> {
    let t = clamp(view_dir.y * 0.5 + 0.5, 0.0, 1.0);
    var col = mix(sky_params.horizon_color, sky_params.zenith_color, t * t);
    col += sun_disc(view_dir);
    return col;
}

// ── Preetham analytical atmosphere (Preetham 1999) ───────────────────────────
// Based on the original paper and the widely-used Three.js Sky shader port.

fn preetham_A(turbidity: f32) -> vec3<f32> {
    return vec3<f32>(-0.0193, -0.0167, 0.1787) * turbidity
         + vec3<f32>(-0.2592, -0.2608, -1.4630);
}
fn preetham_B(turbidity: f32) -> vec3<f32> {
    return vec3<f32>(-0.0665, -0.0950, -0.0272) * turbidity
         + vec3<f32>(0.0008, 0.0092, 0.2102);
}
fn preetham_C(turbidity: f32) -> vec3<f32> {
    return vec3<f32>(-0.0004, -0.0079, -0.0619) * turbidity
         + vec3<f32>(0.2125,  0.2102,  0.2256);
}
fn preetham_D(turbidity: f32) -> vec3<f32> {
    return vec3<f32>(-0.0641, -0.0441,  0.0450) * turbidity
         + vec3<f32>(0.8989,  0.8810, -0.0137);
}
fn preetham_E(turbidity: f32) -> vec3<f32> {
    return vec3<f32>(-0.0033, -0.0109, -0.0146) * turbidity
         + vec3<f32>(0.0452,  0.0529,  0.0667);
}

fn zenith_luminance(turbidity: f32, sun_theta: f32) -> f32 {
    let chi = (4.0 / 9.0 - turbidity / 120.0) * (3.14159265 - 2.0 * sun_theta);
    return (4.0453 * turbidity - 4.9710) * tan(chi) - 0.2155 * turbidity + 2.4192;
}

fn zenith_chroma_x(turbidity: f32, sun_theta: f32) -> f32 {
    let t2 = sun_theta * sun_theta;
    let t3 = t2 * sun_theta;
    return turbidity * turbidity * ( 0.00166 * t3 - 0.00375 * t2 + 0.00209 * sun_theta)
         + turbidity * (-0.02903 * t3 + 0.06377 * t2 - 0.03202 * sun_theta + 0.00394)
         + ( 0.11693 * t3 - 0.21196 * t2 + 0.06052 * sun_theta + 0.25886);
}

fn zenith_chroma_y(turbidity: f32, sun_theta: f32) -> f32 {
    let t2 = sun_theta * sun_theta;
    let t3 = t2 * sun_theta;
    return turbidity * turbidity * ( 0.00275 * t3 - 0.00610 * t2 + 0.00317 * sun_theta)
         + turbidity * (-0.04214 * t3 + 0.08970 * t2 - 0.04153 * sun_theta + 0.00516)
         + ( 0.15346 * t3 - 0.26756 * t2 + 0.06670 * sun_theta + 0.26688);
}

fn perez(cos_theta: f32, gamma: f32, cos_gamma: f32, A: f32, B: f32, C: f32, D: f32, E: f32) -> f32 {
    return (1.0 + A * exp(B / (cos_theta + 0.01)))
         * (1.0 + C * exp(D * gamma) + E * cos_gamma * cos_gamma);
}

fn xyY_to_XYZ(x: f32, y: f32, Y: f32) -> vec3<f32> {
    let X = (x / y) * Y;
    let Z = ((1.0 - x - y) / y) * Y;
    return vec3<f32>(X, Y, Z);
}

fn XYZ_to_linear_rgb(xyz: vec3<f32>) -> vec3<f32> {
    // sRGB D65 matrix
    let r =  3.2404542 * xyz.x - 1.5371385 * xyz.y - 0.4985314 * xyz.z;
    let g = -0.9692660 * xyz.x + 1.8760108 * xyz.y + 0.0415560 * xyz.z;
    let b =  0.0556434 * xyz.x - 0.2040259 * xyz.y + 1.0572252 * xyz.z;
    return max(vec3<f32>(r, g, b), vec3<f32>(0.0));
}

fn sky_preetham(view_dir: vec3<f32>) -> vec3<f32> {
    let turb   = sky_params.turbidity;
    let sun_d  = sky_params.sun_direction;
    // Sun zenith angle
    let sun_theta = acos(clamp(sun_d.y, 0.0, 1.0));

    // Zenith reference values
    let Yz  = zenith_luminance(turb, sun_theta);
    let xz  = zenith_chroma_x(turb, sun_theta);
    let yz2 = zenith_chroma_y(turb, sun_theta);

    let Av = preetham_A(turb); let Bv = preetham_B(turb);
    let Cv = preetham_C(turb); let Dv = preetham_D(turb);
    let Ev = preetham_E(turb);

    // View angle above horizon
    let cos_theta_v = clamp(view_dir.y, 0.001, 1.0);
    let cos_gamma   = dot(view_dir, sun_d);
    let gamma       = acos(clamp(cos_gamma, -1.0, 1.0));

    // Perez distribution — normalised against zenith
    let f_v   = vec3<f32>(
        perez(cos_theta_v, gamma, cos_gamma, Av.x, Bv.x, Cv.x, Dv.x, Ev.x),
        perez(cos_theta_v, gamma, cos_gamma, Av.y, Bv.y, Cv.y, Dv.y, Ev.y),
        perez(cos_theta_v, gamma, cos_gamma, Av.z, Bv.z, Cv.z, Dv.z, Ev.z)
    );
    let f_0 = vec3<f32>(
        perez(1.0, sun_theta, sun_d.y, Av.x, Bv.x, Cv.x, Dv.x, Ev.x),
        perez(1.0, sun_theta, sun_d.y, Av.y, Bv.y, Cv.y, Dv.y, Ev.y),
        perez(1.0, sun_theta, sun_d.y, Av.z, Bv.z, Cv.z, Dv.z, Ev.z)
    );

    let Y = Yz * f_v.x / f_0.x;
    let x = xz  * f_v.y / f_0.y;
    let y = yz2 * f_v.z / f_0.z;

    let xyz = xyY_to_XYZ(x, y, Y * 0.01); // scale to ~0..1 range
    var col = XYZ_to_linear_rgb(xyz);

    // Add sun disc on top
    col += sun_disc(view_dir);
    return col;
}

// ── Panorama (equirectangular) ────────────────────────────────────────────────

fn sky_panorama(view_dir: vec3<f32>) -> vec3<f32> {
    let pi  = 3.14159265;
    let u   = 0.5 + atan2(view_dir.z, view_dir.x) / (2.0 * pi);
    let v   = 0.5 - asin(clamp(view_dir.y, -1.0, 1.0)) / pi;
    return textureSample(panorama_tex, panorama_samp, vec2<f32>(u, v)).rgb;
}

// ── Main ─────────────────────────────────────────────────────────────────────

@fragment
fn fs_main(in: FragInput) -> @location(0) vec4<f32> {
    let pixel = vec2<u32>(u32(in.frag_coord.x), u32(in.frag_coord.y));
    let depth = textureLoad(depth_tex, pixel, 0);

    // Only render sky where no geometry was drawn (depth == far plane)
    if depth < 0.9999 { discard; }

    let view_dir = reconstruct_view_dir(in.uv);

    var sky: vec3<f32>;
    switch sky_params.sky_mode {
        case 1u: { sky = sky_preetham(view_dir); }
        case 2u: { sky = sky_params.solid_color; }
        case 3u: { sky = sky_panorama(view_dir); }
        default: { sky = sky_gradient(view_dir); }
    }

    return vec4<f32>(sky, 1.0);
}
