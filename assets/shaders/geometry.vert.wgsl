// ============================================================
// geometry.vert.wgsl — GBuffer fill vertex shader
//
// Transforms each mesh vertex from model space → world space → clip space.
// Passes world-space position, normal, tangent, and UVs to the fragment
// shader for GBuffer packing.
//
// Bind groups:
//   group(0) binding(0) — CameraUniforms (view_proj, camera_position)
//   group(1) binding(0) — ModelUniforms  (world_matrix, normal_matrix)
// ============================================================

// ── Bind groups ───────────────────────────────────────────────────────────────

struct CameraUniforms {
    view_proj:       mat4x4<f32>,
    camera_position: vec3<f32>,
    _pad:            f32,
}

struct ModelUniforms {
    /// Model → world matrix (column-major, same convention as glam).
    world_matrix:  mat4x4<f32>,
    /// Inverse-transpose of world_matrix for correct normal transformation.
    /// If the model has non-uniform scale, normals must use this instead of world_matrix.
    normal_matrix: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(1) @binding(0) var<uniform> model:  ModelUniforms;

// ── Vertex input ──────────────────────────────────────────────────────────────

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal:   vec3<f32>,
    @location(2) tangent:  vec4<f32>,  // xyz = tangent, w = bitangent sign (+1 or -1)
    @location(3) uv:       vec2<f32>,
}

// ── Vertex output (interpolated to fragment shader) ───────────────────────────

struct VertexOutput {
    @builtin(position) clip_position:  vec4<f32>,
    @location(0)       world_position: vec3<f32>,
    @location(1)       world_normal:   vec3<f32>,
    @location(2)       world_tangent:  vec3<f32>,
    @location(3)       world_bitangent:vec3<f32>,
    @location(4)       uv:             vec2<f32>,
}

// ── Vertex shader ─────────────────────────────────────────────────────────────

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    let world_pos = model.world_matrix * vec4<f32>(in.position, 1.0);

    // Normal matrix is the inverse-transpose of the upper 3×3 of world_matrix.
    // This ensures normals remain perpendicular to surfaces under non-uniform scale.
    let world_normal   = normalize((model.normal_matrix * vec4<f32>(in.normal,      0.0)).xyz);
    let world_tangent  = normalize((model.normal_matrix * vec4<f32>(in.tangent.xyz, 0.0)).xyz);

    // Bitangent: reconstruct from normal × tangent, corrected by the handedness sign
    let world_bitangent = normalize(cross(world_normal, world_tangent) * in.tangent.w);

    var out: VertexOutput;
    out.clip_position   = camera.view_proj * world_pos;
    out.world_position  = world_pos.xyz;
    out.world_normal    = world_normal;
    out.world_tangent   = world_tangent;
    out.world_bitangent = world_bitangent;
    out.uv              = in.uv;
    return out;
}
