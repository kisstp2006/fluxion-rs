// ============================================================
// ssao.wgsl — Screen-Space Ambient Occlusion
//
// Estimates how much ambient light reaches each pixel by sampling
// a hemisphere of points around each surface normal and testing
// whether they are occluded by nearby geometry (using the depth buffer).
//
// Output: single-channel occlusion texture [0,1].
//   0 = fully occluded (dark crevice)
//   1 = fully open (receives full ambient light)
//
// This is then blurred by ssao_blur.wgsl and multiplied into the
// ambient term in the lighting pass.
//
// Ported from the SSAO implementation in the TypeScript engine's
// PostProcessing.ts (ssao.frag.glsl).
// ============================================================

const SAMPLE_COUNT: u32  = 32u;
const PI:           f32  = 3.14159265;

struct SsaoParams {
    /// Sampling hemisphere radius in world units. Larger = coarser occlusion.
    radius:   f32,
    /// Depth bias to prevent self-occlusion ("shadow acne").
    bias:     f32,
    /// Occlusion intensity multiplier.
    intensity: f32,
    _pad:     f32,
    /// Precomputed hemisphere sample directions (in tangent space).
    /// Packed as vec4 (xyz = direction, w = unused).
    samples:  array<vec4<f32>, 32>,
}

@group(0) @binding(0) var<uniform> params:      SsaoParams;
@group(0) @binding(1) var          gbuf_normal: texture_2d<f32>;
@group(0) @binding(2) var          gbuf_depth:  texture_depth_2d;
@group(0) @binding(3) var          noise_tex:   texture_2d<f32>;  // 4x4 random rotation texture
@group(0) @binding(4) var          tex_samp:    sampler;

struct CameraUniforms {
    view_proj:     mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    proj:          mat4x4<f32>,
    inv_proj:      mat4x4<f32>,
    camera_pos:    vec3<f32>,
    _pad:          f32,
}
@group(1) @binding(0) var<uniform> camera: CameraUniforms;

struct FragInput {
    @builtin(position) frag_coord: vec4<f32>,
    @location(0)       uv:         vec2<f32>,
}

fn reconstruct_view_pos(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec4<f32>(uv * 2.0 - 1.0, depth, 1.0);
    let ndc_f = vec4<f32>(ndc.x, -ndc.y, ndc.z, 1.0);
    let view = camera.inv_proj * ndc_f;
    return view.xyz / view.w;
}

fn sample_depth_at_view_pos(view_pos: vec3<f32>) -> f32 {
    // Project view_pos back to UV
    let clip = camera.proj * vec4<f32>(view_pos, 1.0);
    let ndc  = clip.xy / clip.w;
    let uv   = ndc * vec2<f32>(0.5, -0.5) + 0.5;
    // Clamp to prevent sampling outside the texture
    let uv_c = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let pix  = vec2<u32>(
        u32(uv_c.x * f32(textureDimensions(gbuf_depth).x)),
        u32(uv_c.y * f32(textureDimensions(gbuf_depth).y)),
    );
    return textureLoad(gbuf_depth, pix, 0);
}

@fragment
fn fs_main(in: FragInput) -> @location(0) vec4<f32> {
    let tex_size    = vec2<f32>(textureDimensions(gbuf_depth));
    let pixel_coord = vec2<u32>(u32(in.frag_coord.x), u32(in.frag_coord.y));

    // Read depth — skip sky pixels
    let depth = textureLoad(gbuf_depth, pixel_coord, 0);
    if depth >= 0.9999 { return vec4<f32>(1.0); }

    // Reconstruct view-space position
    let view_pos = reconstruct_view_pos(in.uv, depth);

    // Read world-space normal → convert to view space
    let normal_sample  = textureSample(gbuf_normal, tex_samp, in.uv).rgb;
    let world_normal   = normalize(normal_sample * 2.0 - 1.0);
    // Use only the rotation part of the view transform (upper-left 3x3 of view matrix).
    // view_proj = proj * view, so we need just view's rotation. We extract it from
    // inv_view_proj: the transpose of its upper-left 3x3 gives the view rotation.
    // Simpler: transform the normal with view_proj as a direction (w=0) and then
    // un-apply projection distortion by using inv_proj on the result.
    let view_pos_h     = camera.inv_proj * (camera.view_proj * vec4<f32>(world_normal, 0.0));
    let view_normal    = normalize(view_pos_h.xyz);

    // Random rotation from noise texture (tile 4x4 over screen)
    let noise_uv    = in.uv * (tex_size / 4.0);
    let random_vec  = textureSample(noise_tex, tex_samp, noise_uv).xyz * 2.0 - 1.0;

    // Build TBN basis in view space (Gram-Schmidt)
    let tangent   = normalize(random_vec - view_normal * dot(random_vec, view_normal));
    let bitangent = cross(view_normal, tangent);
    let tbn       = mat3x3<f32>(tangent, bitangent, view_normal);

    // Accumulate occlusion
    var occlusion = 0.0;
    for (var i = 0u; i < SAMPLE_COUNT; i++) {
        // Transform hemisphere sample to view space
        let sample_dir  = tbn * params.samples[i].xyz;
        let sample_pos  = view_pos + sample_dir * params.radius;

        // Get the depth at the sample position
        let sample_depth = sample_depth_at_view_pos(sample_pos);

        // Reconstruct Z of the geometry at the sample UV
        let sample_uv = clamp(
            (camera.proj * vec4<f32>(sample_pos, 1.0)).xy
            / (camera.proj * vec4<f32>(sample_pos, 1.0)).w
            * vec2<f32>(0.5, -0.5) + 0.5,
            vec2<f32>(0.0), vec2<f32>(1.0),
        );
        let geometry_view_pos = reconstruct_view_pos(sample_uv, sample_depth);

        // Range check: don't occlude surfaces that are far away
        let range_check = smoothstep(0.0, 1.0, params.radius / abs(view_pos.z - geometry_view_pos.z));

        // A sample occludes if geometry is closer to the camera than the sample
        if geometry_view_pos.z >= sample_pos.z + params.bias {
            occlusion += range_check;
        }
    }

    occlusion = 1.0 - (occlusion / f32(SAMPLE_COUNT)) * params.intensity;
    return vec4<f32>(occlusion, occlusion, occlusion, 1.0);
}
