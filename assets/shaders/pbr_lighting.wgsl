// ============================================================
// pbr_lighting.wgsl — Deferred PBR lighting pass
//
// Full-screen pass that reads the GBuffer and computes physically-
// based lighting for all lights in the scene.
//
// PBR model:
//   - Diffuse:   Lambertian (albedo / π)
//   - Specular:  Cook-Torrance BRDF
//     - D  = GGX/Trowbridge-Reitz normal distribution function
//     - G  = Smith's shadowing-masking function
//     - F  = Schlick's Fresnel approximation
//
// Supports up to 64 lights per frame (directional + point + spot).
//
// Bind groups:
//   group(0) binding(0) — CameraUniforms
//   group(1) binding(0) — LightBuffer (array of LightData)
//   group(2) binding(0..4) — GBuffer textures (albedo_ao, normal, orm, emission, depth)
// ============================================================

const PI:      f32 = 3.14159265358979;
const MAX_LIGHTS: u32 = 64u;

// ── Camera ────────────────────────────────────────────────────────────────────

struct CameraUniforms {
    view_proj:       mat4x4<f32>,
    inv_view_proj:   mat4x4<f32>,  // for reconstructing world pos from depth
    camera_position: vec3<f32>,
    debug_view:      u32,          // 0=Lit 1=Albedo 2=Normal 3=Roughness 4=Metalness 5=AO 6=Emissive 7=Unlit
}

@group(0) @binding(0) var<uniform> camera: CameraUniforms;

// ── Light data ────────────────────────────────────────────────────────────────

// Light type constants (match LightType enum in Rust)
const LIGHT_DIRECTIONAL: u32 = 0u;
const LIGHT_POINT:       u32 = 1u;
const LIGHT_SPOT:        u32 = 2u;

struct LightData {
    position:    vec3<f32>,
    light_type:  u32,
    direction:   vec3<f32>,     // Normalized. Only directional + spot.
    range:       f32,
    color:       vec3<f32>,
    intensity:   f32,
    spot_angle:  f32,           // cos(outer half-angle)
    spot_inner:  f32,           // cos(inner half-angle) — penumbra boundary
    _pad0:       f32,
    _pad1:       f32,
}

struct LightBuffer {
    count:             u32,
    _pad0:             u32,
    _pad1:             u32,
    _pad2:             u32,
    ambient_color:     vec3<f32>,
    ambient_intensity: f32,
    lights:            array<LightData, 64>,
    fog_color:         vec3<f32>,
    fog_density:       f32,
    fog_enabled:       u32,
    fog_mode:          u32,   // 0 = Exponential, 1 = Linear
    fog_near:          f32,
    fog_far:           f32,
}

@group(1) @binding(0) var<uniform> light_buf: LightBuffer;

// ── GBuffer samplers ──────────────────────────────────────────────────────────

@group(2) @binding(0) var gbuf_albedo_ao: texture_2d<f32>;
@group(2) @binding(1) var gbuf_normal:    texture_2d<f32>;
@group(2) @binding(2) var gbuf_orm:       texture_2d<f32>;
@group(2) @binding(3) var gbuf_emission:  texture_2d<f32>;
@group(2) @binding(4) var gbuf_depth:     texture_depth_2d;
@group(2) @binding(5) var gbuf_sampler:   sampler;

// ── Shadow map ────────────────────────────────────────────────────────────────

struct ShadowUniforms {
    light_view_proj: mat4x4<f32>,
    has_shadow:      u32,
    _pad0:           u32,
    _pad1:           u32,
    _pad2:           u32,
}

@group(3) @binding(0) var<uniform> shadow_uni:     ShadowUniforms;
@group(3) @binding(1) var          shadow_map:     texture_depth_2d;
@group(3) @binding(2) var          shadow_sampler: sampler_comparison;

// PCF shadow test — 3×3 kernel, returns [0..1] where 1 = fully lit.
fn shadow_pcf(world_pos: vec3<f32>) -> f32 {
    if shadow_uni.has_shadow == 0u { return 1.0; }

    let light_clip = shadow_uni.light_view_proj * vec4<f32>(world_pos, 1.0);
    // Perspective divide → NDC
    var proj = light_clip.xyz / light_clip.w;
    // Map NDC [-1,1] → UV [0,1]; flip Y for wgpu convention
    let shadow_uv = vec2<f32>(proj.x * 0.5 + 0.5, -proj.y * 0.5 + 0.5);
    let depth     = proj.z;

    // Outside the shadow frustum → fully lit
    if shadow_uv.x < 0.0 || shadow_uv.x > 1.0 ||
       shadow_uv.y < 0.0 || shadow_uv.y > 1.0 ||
       depth < 0.0 || depth > 1.0 {
        return 1.0;
    }

    let tex_size = vec2<f32>(textureDimensions(shadow_map));
    let texel    = 1.0 / tex_size;
    var shadow   = 0.0;

    // 3×3 PCF
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let offset = vec2<f32>(f32(dx), f32(dy)) * texel;
            shadow += textureSampleCompare(
                shadow_map, shadow_sampler,
                shadow_uv + offset,
                depth - 0.005,  // small bias to prevent acne
            );
        }
    }
    return shadow / 9.0;
}

// ── Vertex input (from fullscreen.vert.wgsl) ──────────────────────────────────

struct FragInput {
    @builtin(position) frag_coord: vec4<f32>,
    @location(0)       uv:         vec2<f32>,
}

// ── PBR helper functions ──────────────────────────────────────────────────────

// GGX normal distribution function.
// D(h, n, α) = α² / (π · ((n·h)² · (α²-1) + 1)²)
fn distribution_ggx(n_dot_h: f32, roughness: f32) -> f32 {
    let a  = roughness * roughness;
    let a2 = a * a;
    let d  = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

// Smith's geometry function (Schlick approximation).
// G(n, v, l, α) = G_sub(n,v,α) · G_sub(n,l,α)
fn geometry_schlick(n_dot_v: f32, roughness: f32) -> f32 {
    let k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    return n_dot_v / (n_dot_v * (1.0 - k) + k);
}

fn geometry_smith(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    return geometry_schlick(n_dot_v, roughness) * geometry_schlick(n_dot_l, roughness);
}

// Fresnel-Schlick approximation.
// F(v, h) = F0 + (1 - F0) · (1 - v·h)^5
fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (1.0 - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

// Fresnel-Schlick with roughness correction for ambient/IBL.
// Roughness attenuates the specular peak so rough surfaces don't get
// overly bright ambient specular highlights.
fn fresnel_schlick_roughness(cos_theta: f32, f0: vec3<f32>, roughness: f32) -> vec3<f32> {
    let f_max = vec3<f32>(max(1.0 - roughness, f0.x), max(1.0 - roughness, f0.y), max(1.0 - roughness, f0.z));
    return f0 + (f_max - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

// Cook-Torrance BRDF specular contribution for a single light.
// Returns: (diffuse_brdf, specular_brdf) contributions
fn cook_torrance(
    n: vec3<f32>, v: vec3<f32>, l: vec3<f32>,
    albedo:    vec3<f32>,
    roughness: f32,
    metalness: f32,
) -> vec3<f32> {
    let h = normalize(v + l);
    let n_dot_v = max(dot(n, v), 0.0001);
    let n_dot_l = max(dot(n, l), 0.0);
    let n_dot_h = max(dot(n, h), 0.0);
    let v_dot_h = max(dot(v, h), 0.0);

    if n_dot_l <= 0.0 { return vec3<f32>(0.0); }

    // Dielectric F0 = 0.04, metals use albedo as F0
    let f0 = mix(vec3<f32>(0.04), albedo, metalness);

    // Specular BRDF
    let D = distribution_ggx(n_dot_h, roughness);
    let G = geometry_smith(n_dot_v, n_dot_l, roughness);
    let F = fresnel_schlick(v_dot_h, f0);

    let specular = (D * G * F) / max(4.0 * n_dot_v * n_dot_l, 0.0001);

    // Diffuse: metals have no diffuse contribution
    let k_d = (vec3<f32>(1.0) - F) * (1.0 - metalness);
    let diffuse = k_d * albedo / PI;

    return (diffuse + specular) * n_dot_l;
}

// Smooth attenuation for point and spot lights.
// Returns 0 at range, 1 at distance 0. Physically-based inverse-square falloff
// with a smooth cutoff to avoid hard edges.
fn distance_attenuation(dist: f32, range: f32) -> f32 {
    let normalized = clamp(dist / range, 0.0, 1.0);
    // Inverse-square with smooth window function
    let window = pow(max(0.0, 1.0 - normalized * normalized * normalized * normalized), 2.0);
    return window / max(dist * dist, 0.0001);
}

// Reconstruct world-space position from depth buffer + screen UV.
fn reconstruct_world_pos(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    // NDC: x,y in [-1,1], z = depth (0 = near, 1 = far in wgpu/Vulkan)
    let ndc = vec4<f32>(uv * 2.0 - 1.0, depth, 1.0);
    // wgpu uses Y-down NDC, UV (0,0) is top-left → flip Y
    let ndc_flipped = vec4<f32>(ndc.x, -ndc.y, ndc.z, 1.0);
    let world = camera.inv_view_proj * ndc_flipped;
    return world.xyz / world.w;
}

// ── Fragment shader ───────────────────────────────────────────────────────────

@fragment
fn fs_main(in: FragInput) -> @location(0) vec4<f32> {
    let tex_size  = textureDimensions(gbuf_albedo_ao);
    let pixel_uv  = in.uv;

    // ── Read GBuffer ──────────────────────────────────────────────────────────
    let albedo_ao_samp = textureSample(gbuf_albedo_ao, gbuf_sampler, pixel_uv);
    let normal_samp    = textureSample(gbuf_normal,    gbuf_sampler, pixel_uv);
    let orm_samp       = textureSample(gbuf_orm,       gbuf_sampler, pixel_uv);
    let emission_samp  = textureSample(gbuf_emission,  gbuf_sampler, pixel_uv);

    // Load depth (non-sampler load for precision)
    let pixel_coord = vec2<u32>(u32(in.frag_coord.x), u32(in.frag_coord.y));
    let depth       = textureLoad(gbuf_depth, pixel_coord, 0);

    // Sky pixels have depth = 0.0 (far plane in reverse-Z) or 1.0 in standard Z.
    // We skip lighting for sky pixels — they'll be drawn by the skybox pass.
    if depth >= 0.9999 { return vec4<f32>(0.0); }

    // Decode GBuffer
    let albedo    = albedo_ao_samp.rgb;
    let ao        = albedo_ao_samp.a;
    // Unpack normal from [0,1] to [-1,1]
    let n         = normalize(normal_samp.rgb * 2.0 - 1.0);
    let roughness = orm_samp.g;
    let metalness = orm_samp.b;
    let emission  = emission_samp.rgb;

    // Reconstruct world position from depth
    let world_pos = reconstruct_world_pos(pixel_uv, depth);

    // View direction (fragment → camera)
    let v = normalize(camera.camera_position - world_pos);

    // ── Accumulate light contributions ────────────────────────────────────────
    var total_radiance = vec3<f32>(0.0);

    for (var i = 0u; i < light_buf.count; i++) {
        let light = light_buf.lights[i];
        var l: vec3<f32>;       // direction from fragment to light
        var attenuation: f32;

        if light.light_type == LIGHT_DIRECTIONAL {
            // Directional: light direction is constant across the scene
            l           = -light.direction;
            // First directional light uses the shadow map; others are unshadowed.
            attenuation = select(1.0, shadow_pcf(world_pos), i == 0u);

        } else if light.light_type == LIGHT_POINT {
            let to_light = light.position - world_pos;
            let dist     = length(to_light);
            l           = to_light / max(dist, 0.0001);
            attenuation = distance_attenuation(dist, light.range);

        } else { // SPOT
            let to_light    = light.position - world_pos;
            let dist        = length(to_light);
            l               = to_light / max(dist, 0.0001);
            let spot_cos    = dot(-l, light.direction);
            let cone_atten  = clamp(
                (spot_cos - light.spot_angle) / max(light.spot_inner - light.spot_angle, 0.0001),
                0.0, 1.0,
            );
            attenuation = distance_attenuation(dist, light.range) * cone_atten * cone_atten;
        }

        let radiance   = light.color * light.intensity * attenuation;
        let brdf       = cook_torrance(n, v, l, albedo, roughness, metalness);
        total_radiance += brdf * radiance;
    }

    // ── Ambient (configurable flat ambient from LightBuffer) ─────────────────
    // Tinted by the sky color set from Rust. SSAO multiplies this by occlusion.
    // Energy-conserving split: diffuse and specular ambient terms.
    let n_dot_v_amb = max(dot(n, v), 0.0);
    let f0_amb      = mix(vec3<f32>(0.04), albedo, metalness);
    let f_amb       = fresnel_schlick_roughness(n_dot_v_amb, f0_amb, roughness);
    let k_d_amb     = (vec3<f32>(1.0) - f_amb) * (1.0 - metalness);
    let amb_base    = light_buf.ambient_color * light_buf.ambient_intensity * ao;
    let ambient     = amb_base * (k_d_amb * albedo + f_amb * (1.0 - roughness) * albedo);

    // ── Debug view override ───────────────────────────────────────────────────
    // Non-zero debug_view bypasses normal lighting and outputs a single GBuffer channel.
    if camera.debug_view != 0u {
        var dbg = vec3<f32>(0.0);
        if camera.debug_view == 1u {
            // Albedo
            dbg = pow(albedo, vec3<f32>(1.0 / 2.2)); // sRGB for display
        } else if camera.debug_view == 2u {
            // World-space normal → [0,1]
            dbg = n * 0.5 + 0.5;
        } else if camera.debug_view == 3u {
            // Roughness (greyscale)
            dbg = vec3<f32>(roughness);
        } else if camera.debug_view == 4u {
            // Metalness (greyscale)
            dbg = vec3<f32>(metalness);
        } else if camera.debug_view == 5u {
            // Ambient Occlusion (greyscale)
            dbg = vec3<f32>(ao);
        } else if camera.debug_view == 6u {
            // Emissive
            dbg = emission;
        } else if camera.debug_view == 7u {
            // Unlit — albedo without any lighting
            dbg = albedo;
        }
        return vec4<f32>(dbg, 1.0);
    }

    // ── Final composition ─────────────────────────────────────────────────────
    var color = ambient + total_radiance + emission;

    if (light_buf.fog_enabled != 0u) {
        let dist = length(camera.camera_position - world_pos);
        var fog_t: f32;
        if (light_buf.fog_mode == 1u) {
            // Linear fog
            let range = max(light_buf.fog_far - light_buf.fog_near, 0.0001);
            fog_t = clamp((dist - light_buf.fog_near) / range, 0.0, 1.0);
        } else {
            // Exponential fog (default)
            fog_t = 1.0 - exp(-light_buf.fog_density * dist);
        }
        color = mix(color, light_buf.fog_color, fog_t);
    }

    return vec4<f32>(color, 1.0);
}
