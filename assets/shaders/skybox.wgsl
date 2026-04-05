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

// ── Rayleigh + Mie atmospheric scattering ────────────────────────────────────
// Based on the Three.js Sky shader (A. J. Preetham et al. 1999, Bruneton 2008).
// Works directly in linear RGB — no XYZ conversion needed.
// betaR is wavelength-dependent → gives the characteristic blue-sky tinting.

fn mie_phase(cos_a: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (3.0 / (8.0 * 3.14159265))
         * ((1.0 - g2) * (1.0 + cos_a * cos_a))
         / ((2.0 + g2) * pow(max(1.0 + g2 - 2.0 * g * cos_a, 1e-6), 1.5));
}

fn sky_preetham(view_dir: vec3<f32>) -> vec3<f32> {
    let PI    = 3.14159265;
    let sun   = sky_params.sun_direction;
    let ray   = sky_params.rayleigh;
    let turb  = sky_params.turbidity;
    let mie_c = sky_params.mie_coefficient;
    let mie_g = sky_params.mie_directional_g;

    // Rayleigh scattering — wavelength-dependent (R<G<B → blue sky)
    let betaR = vec3<f32>(5.5e-6, 13.0e-6, 22.4e-6) * ray;
    // Mie scattering — wavelength-independent, driven by turbidity + coefficient
    let betaM = vec3<f32>(2.1e-5) * turb * mie_c;

    // Chapman-approximated optical depth along view ray
    let vH  = max(view_dir.y, 0.001);
    let inv = 1.0 / (vH + 0.025 * exp(-22.26 * vH));
    let sR  = 8400.0 * inv;   // Rayleigh scale height ~8.4 km
    let sM  = 1200.0 * inv;   // Mie    scale height ~1.2 km

    // Extinction (atmospheric absorption)
    let Fex = exp(-(betaR * sR + betaM * sM));

    // Phase functions
    let cos_a  = clamp(dot(view_dir, sun), -1.0, 1.0);
    let rPhase = (3.0 / (16.0 * PI)) * (1.0 + cos_a * cos_a);
    let mPhase = mie_phase(cos_a, mie_g);

    let betaRTheta = betaR * rPhase;
    let betaMTheta = betaM * mPhase;
    let betaTotal  = betaR + betaM;

    // Sun irradiance increases as sun rises above horizon
    let sunE = 20.0 * max(sun.y, 0.0) + 2.0;

    // Primary in-scattering
    var Lin = pow(
        sunE * ((betaRTheta + betaMTheta) / betaTotal) * (1.0 - Fex),
        vec3<f32>(1.5)
    );
    // Horizon blend at sunset/sunrise
    Lin *= mix(
        vec3<f32>(1.0),
        pow(sunE * ((betaRTheta + betaMTheta) / betaTotal) * Fex, vec3<f32>(0.5)),
        clamp(pow(1.0 - max(sun.y, 0.0), 5.0), 0.0, 1.0)
    );

    // Ambient sky light + sun disc
    var L0 = vec3<f32>(0.1) * Fex;
    L0 += sun_disc(view_dir);

    // Scale to HDR range (~0..1 for average sky brightness); tonemap handles the rest
    var col = (Lin + L0) * 0.04 + vec3<f32>(0.0, 0.0003, 0.0006);

    // Tint with user-set horizon/zenith colors (same blend as gradient mode)
    let t    = clamp(view_dir.y * 0.5 + 0.5, 0.0, 1.0);
    let tint = mix(sky_params.horizon_color, sky_params.zenith_color, t * t);
    col *= tint * 2.0;

    return max(col, vec3<f32>(0.0));
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
