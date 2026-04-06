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
//
// Layout (112 bytes, 7×16):
//   [  0] color              vec4<f32>   16 B
//   [ 16] emissive           vec3<f32>   12 B
//   [ 28] emissive_intensity f32          4 B
//   [ 32] roughness          f32          4 B
//   [ 36] metalness          f32          4 B
//   [ 40] normal_scale       f32          4 B
//   [ 44] ao_intensity       f32          4 B
//   [ 48] uv_scale           vec2<f32>    8 B
//   [ 56] uv_offset          vec2<f32>    8 B
//   [ 64] texture_flags      u32          4 B
//   [ 68] clearcoat          f32          4 B
//   [ 72] clearcoat_roughness f32         4 B
//   [ 76] anisotropy         f32          4 B
//   [ 80] sheen_color        vec3<f32>   12 B  (16-byte aligned ✓)
//   [ 92] sheen_roughness    f32          4 B
//   [ 96] subsurface_color   vec3<f32>   12 B  (16-byte aligned ✓)
//   [108] subsurface         f32          4 B
//   total = 112 B

struct PbrParams {
    color:               vec4<f32>,
    emissive:            vec3<f32>,
    emissive_intensity:  f32,
    roughness:           f32,
    metalness:           f32,
    normal_scale:        f32,
    ao_intensity:        f32,
    uv_scale:            vec2<f32>,
    uv_offset:           vec2<f32>,
    // Bitfield: bit 0=albedo, 1=normal, 2=orm, 3=emissive
    texture_flags:       u32,
    // Extended PBR (clearcoat, sheen, subsurface, anisotropy)
    clearcoat:           f32,
    clearcoat_roughness: f32,
    anisotropy:          f32,
    sheen_color:         vec3<f32>,
    sheen_roughness:     f32,
    subsurface_color:    vec3<f32>,
    subsurface:          f32,
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
//
//   RT0 albedo_ao:  RGB=base color (linear), A=AO
//   RT1 normal:     RGB=world normal packed [0,1], A=clearcoat strength
//   RT2 orm:        R=AO, G=roughness, B=metalness, A=pack(anisotropy,subsurface)
//   RT3 emission:   RGB=emissive radiance, A=pack(sheen+clearcoat_roughness)

struct GBufferOutput {
    @location(0) albedo_ao: vec4<f32>,
    @location(1) normal:    vec4<f32>,
    @location(2) orm:       vec4<f32>,
    @location(3) emission:  vec4<f32>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn has_texture(flags: u32, bit: u32) -> bool {
    return (flags & (1u << bit)) != 0u;
}

fn pack_normal(n: vec3<f32>) -> vec3<f32> {
    return n * 0.5 + 0.5;
}

// Pack two [0,1] floats into one f32 channel (8 bits each, top bits).
fn pack_f2(a: f32, b: f32) -> f32 {
    let ia = u32(clamp(a, 0.0, 1.0) * 255.0);
    let ib = u32(clamp(b, 0.0, 1.0) * 255.0);
    return f32((ia << 8u) | ib) / 65535.0;
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
    if base_color.a < 0.01 { discard; }

    // ── Normal mapping ────────────────────────────────────────────────────────
    var world_normal = normalize(in.world_normal);
    if has_texture(material.texture_flags, 1u) {
        var normal_sample = textureSample(normal_tex, normal_samp, uv).xyz * 2.0 - 1.0;
        normal_sample.x *= material.normal_scale;
        normal_sample.y *= material.normal_scale;
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
        let orm_sample = textureSample(orm_tex, orm_samp, uv).rgb;
        occlusion *= orm_sample.r;
        roughness *= orm_sample.g;
        metalness *= orm_sample.b;
    }

    // ── Anisotropy tangent rotation (stored in orm.a) ─────────────────────────
    // Remap anisotropy from [-1,1] → [0,1] for GBuffer storage.
    let aniso_packed = material.anisotropy * 0.5 + 0.5;

    // ── Emission ──────────────────────────────────────────────────────────────
    var emission = material.emissive * material.emissive_intensity;
    if has_texture(material.texture_flags, 3u) {
        let emissive_sample = textureSample(emissive_tex, emissive_samp, uv).rgb;
        emission *= emissive_sample;
    }

    // ── Subsurface wrap: tint base color toward subsurface color ──────────────
    // Only non-zero when subsurface > 0; blended in lighting pass via GBuffer.
    let sss_weight = material.subsurface;

    // ── Write GBuffer ─────────────────────────────────────────────────────────
    var out: GBufferOutput;
    out.albedo_ao = vec4<f32>(base_color.rgb, occlusion);
    // Pack clearcoat in normal.a (cleared to 0 when no clearcoat).
    out.normal    = vec4<f32>(pack_normal(world_normal), material.clearcoat);
    // Pack anisotropy (remapped) and subsurface into orm.a using pack_f2.
    out.orm       = vec4<f32>(occlusion, roughness, metalness, pack_f2(aniso_packed, sss_weight));
    // Pack clearcoat_roughness and sheen luminance into emission.a.
    let sheen_lum  = dot(material.sheen_color, vec3<f32>(0.2126, 0.7152, 0.0722));
    out.emission  = vec4<f32>(emission, pack_f2(material.clearcoat_roughness, sheen_lum));
    return out;
}
