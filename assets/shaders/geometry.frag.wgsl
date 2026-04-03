// ============================================================
// geometry.frag.wgsl — GBuffer fill fragment shader
//
// Writes material data to multiple render targets (MRT):
//   attachment 0 — albedo_ao:   RGB = base color (linear), A = ambient occlusion
//   attachment 1 — normal:      RGB = world-space normal (encoded to [0,1])
//   attachment 2 — orm:         R = occlusion, G = roughness, B = metalness
//   attachment 3 — emission:    RGB = emissive color * intensity
//
// Depth is written to the depth attachment automatically by the GPU.
//
// Bind groups:
//   group(0) binding(0) — CameraUniforms (same as vertex shader)
//   group(2) binding(0) — PbrParams uniform buffer
//   group(2) binding(1) — albedo texture + sampler
//   group(2) binding(2) — normal map texture + sampler
//   group(2) binding(3) — ORM (occlusion-roughness-metalness) texture + sampler
//   group(2) binding(4) — emissive texture + sampler
// ============================================================

// ── Material uniform ──────────────────────────────────────────────────────────

struct PbrParams {
    color:              vec4<f32>,   // base color RGBA (linear)
    emissive:           vec3<f32>,
    emissive_intensity: f32,
    roughness:          f32,
    metalness:          f32,
    normal_scale:       f32,         // normal map strength (1.0 = full, 0.0 = flat)
    ao_intensity:       f32,
    uv_scale:           vec2<f32>,
    uv_offset:          vec2<f32>,
    // Bitfield: which texture slots are bound.
    // bit 0 = albedo, 1 = normal, 2 = orm, 3 = emissive
    texture_flags:      u32,
    _pad:               vec3<f32>,
}

@group(2) @binding(0) var<uniform> material: PbrParams;
@group(2) @binding(1) var albedo_tex:   texture_2d<f32>;
@group(2) @binding(2) var albedo_samp:  sampler;
@group(2) @binding(3) var normal_tex:   texture_2d<f32>;
@group(2) @binding(4) var normal_samp:  sampler;
@group(2) @binding(5) var orm_tex:      texture_2d<f32>;
@group(2) @binding(6) var orm_samp:     sampler;
@group(2) @binding(7) var emissive_tex: texture_2d<f32>;
@group(2) @binding(8) var emissive_samp:sampler;

// ── Vertex output → fragment input ────────────────────────────────────────────

struct FragInput {
    @location(0) world_position:  vec3<f32>,
    @location(1) world_normal:    vec3<f32>,
    @location(2) world_tangent:   vec3<f32>,
    @location(3) world_bitangent: vec3<f32>,
    @location(4) uv:              vec2<f32>,
}

// ── GBuffer output (4 render targets) ────────────────────────────────────────

struct GBufferOutput {
    @location(0) albedo_ao: vec4<f32>,  // RGB=albedo, A=ao
    @location(1) normal:    vec4<f32>,  // RGB=world normal encoded, A=unused
    @location(2) orm:       vec4<f32>,  // R=ao, G=roughness, B=metalness, A=unused
    @location(3) emission:  vec4<f32>,  // RGB=emission, A=unused
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn has_texture(flags: u32, bit: u32) -> bool {
    return (flags & (1u << bit)) != 0u;
}

// Pack world normal from [-1,1] to [0,1] for storage in an Rgba8Unorm texture.
fn pack_normal(n: vec3<f32>) -> vec3<f32> {
    return n * 0.5 + 0.5;
}

// ── Fragment shader ───────────────────────────────────────────────────────────

@fragment
fn fs_main(in: FragInput) -> GBufferOutput {
    let uv = in.uv * material.uv_scale + material.uv_offset;

    // ── Base color ────────────────────────────────────────────────────────────
    var base_color = material.color;
    if has_texture(material.texture_flags, 0u) {
        let tex_color = textureSample(albedo_tex, albedo_samp, uv);
        base_color *= tex_color;
    }
    // Alpha test: discard fully transparent fragments
    if base_color.a < 0.01 { discard; }

    // ── Normal mapping ────────────────────────────────────────────────────────
    var world_normal = normalize(in.world_normal);
    if has_texture(material.texture_flags, 1u) {
        // Sample normal map, remap from [0,1] to [-1,1]
        var normal_sample = textureSample(normal_tex, normal_samp, uv).xyz * 2.0 - 1.0;
        normal_sample.x *= material.normal_scale;
        normal_sample.y *= material.normal_scale;

        // Build TBN matrix (tangent space → world space)
        let T = normalize(in.world_tangent);
        let B = normalize(in.world_bitangent);
        let N = normalize(in.world_normal);
        let tbn = mat3x3<f32>(T, B, N);
        world_normal = normalize(tbn * normal_sample);
    }

    // ── ORM (occlusion, roughness, metalness) ─────────────────────────────────
    var occlusion = material.ao_intensity;
    var roughness = material.roughness;
    var metalness = material.metalness;
    if has_texture(material.texture_flags, 2u) {
        // Standard ORM packing: R=occlusion, G=roughness, B=metalness
        let orm_sample = textureSample(orm_tex, orm_samp, uv).rgb;
        occlusion *= orm_sample.r;
        roughness *= orm_sample.g;
        metalness *= orm_sample.b;
    }

    // ── Emission ──────────────────────────────────────────────────────────────
    var emission = material.emissive * material.emissive_intensity;
    if has_texture(material.texture_flags, 3u) {
        let emissive_sample = textureSample(emissive_tex, emissive_samp, uv).rgb;
        emission *= emissive_sample;
    }

    // ── Write GBuffer ─────────────────────────────────────────────────────────
    var out: GBufferOutput;
    out.albedo_ao = vec4<f32>(base_color.rgb, occlusion);
    out.normal    = vec4<f32>(pack_normal(world_normal), 0.0);
    out.orm       = vec4<f32>(occlusion, roughness, metalness, 0.0);
    out.emission  = vec4<f32>(emission, 0.0);
    return out;
}
