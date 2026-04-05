// ============================================================
// skinned_geometry.vert.wgsl
//
// Vertex shader for skinned (skeletal) meshes.
// Identical to geometry.vert.wgsl except it blends the vertex
// position/normal/tangent through up to 4 joint matrices before
// applying the model transform.
//
// Joint matrices are packed as mat4x4 in a uniform buffer
// (up to MAX_JOINTS = 128 matrices per draw call).
//
// Bind groups:
//   group(0) binding(0) — CameraUniforms
//   group(1) binding(0) — ModelUniforms  (world_matrix, normal_matrix)
//   group(2)            — Material (fragment only)
//   group(3) binding(0) — JointMatrices
// ============================================================

struct ModelUniforms {
    world_matrix:  mat4x4<f32>,
    normal_matrix: mat4x4<f32>,
}

struct CameraUniforms {
    view_proj:     mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos:    vec3<f32>,
    _pad:          f32,
}

struct JointMatrices {
    joints: array<mat4x4<f32>, 128>,
}

@group(0) @binding(0) var<uniform> camera:    CameraUniforms;
@group(1) @binding(0) var<uniform> model:     ModelUniforms;
@group(3) @binding(0) var<uniform> joint_buf: JointMatrices;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal:   vec3<f32>,
    @location(2) tangent:  vec4<f32>,   // xyz = tangent, w = bitangent sign
    @location(3) uv:       vec2<f32>,
    @location(4) joints:   vec4<u32>,
    @location(5) weights:  vec4<f32>,
}

// Output layout matches geometry.vert.wgsl so the same frag shader works.
struct VertexOutput {
    @builtin(position) clip_position:   vec4<f32>,
    @location(0)       world_position:  vec3<f32>,
    @location(1)       world_normal:    vec3<f32>,
    @location(2)       world_tangent:   vec3<f32>,
    @location(3)       world_bitangent: vec3<f32>,
    @location(4)       uv:              vec2<f32>,
}

@vertex
fn vs_main(v: VertexInput) -> VertexOutput {
    // ── Blend joint matrices by weights ──────────────────────────────────────
    let j0 = joint_buf.joints[v.joints.x];
    let j1 = joint_buf.joints[v.joints.y];
    let j2 = joint_buf.joints[v.joints.z];
    let j3 = joint_buf.joints[v.joints.w];
    let skin_mat = j0 * v.weights.x
                 + j1 * v.weights.y
                 + j2 * v.weights.z
                 + j3 * v.weights.w;

    // Apply skinning in object space.
    let skinned_pos = (skin_mat * vec4<f32>(v.position,     1.0)).xyz;
    let skinned_nor = normalize((skin_mat * vec4<f32>(v.normal,        0.0)).xyz);
    let skinned_tan = normalize((skin_mat * vec4<f32>(v.tangent.xyz,   0.0)).xyz);

    // ── World-space transform ─────────────────────────────────────────────────
    let world_pos = (model.world_matrix  * vec4<f32>(skinned_pos, 1.0)).xyz;
    // Use normal_matrix (inverse-transpose) for directions — correct under non-uniform scale.
    let world_nor = normalize((model.normal_matrix * vec4<f32>(skinned_nor, 0.0)).xyz);
    let world_tan = normalize((model.normal_matrix * vec4<f32>(skinned_tan, 0.0)).xyz);
    // Reconstruct bitangent from cross product, preserving handedness sign.
    let world_bit = normalize(cross(world_nor, world_tan) * v.tangent.w);

    var out: VertexOutput;
    out.clip_position   = camera.view_proj * vec4<f32>(world_pos, 1.0);
    out.world_position  = world_pos;
    out.world_normal    = world_nor;
    out.world_tangent   = world_tan;
    out.world_bitangent = world_bit;
    out.uv              = v.uv;
    return out;
}
