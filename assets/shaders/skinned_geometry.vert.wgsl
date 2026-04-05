// ============================================================
// skinned_geometry.vert.wgsl
//
// Vertex shader for skinned (skeletal) meshes.
// Identical to geometry.vert.wgsl except it blends the vertex
// position/normal/tangent through up to 4 joint matrices before
// applying the model transform.
//
// Joint matrices are packed as mat4x4 in a storage buffer
// (up to MAX_JOINTS = 128 matrices per draw call).
// ============================================================

struct ObjectUniforms {
    model:        mat4x4<f32>,
    normal_mat:   mat4x4<f32>,
}

struct CameraUniforms {
    view_proj:    mat4x4<f32>,
    inv_view_proj:mat4x4<f32>,
    camera_pos:   vec3<f32>,
    _pad:         f32,
}

struct JointMatrices {
    joints: array<mat4x4<f32>, 128>,
}

@group(0) @binding(0) var<uniform> object: ObjectUniforms;
@group(1) @binding(0) var<uniform> camera: CameraUniforms;
@group(4) @binding(0) var<uniform> joint_buf: JointMatrices;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal:   vec3<f32>,
    @location(2) tangent:  vec4<f32>,
    @location(3) uv:       vec2<f32>,
    @location(4) joints:   vec4<u32>,
    @location(5) weights:  vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_pos:    vec4<f32>,
    @location(0)       world_pos:   vec3<f32>,
    @location(1)       world_norm:  vec3<f32>,
    @location(2)       world_tan:   vec4<f32>,
    @location(3)       uv:          vec2<f32>,
}

@vertex
fn vs_main(v: VertexInput) -> VertexOutput {
    // Blend joint matrices by weights.
    let j0 = joint_buf.joints[v.joints.x];
    let j1 = joint_buf.joints[v.joints.y];
    let j2 = joint_buf.joints[v.joints.z];
    let j3 = joint_buf.joints[v.joints.w];
    let skin_mat = j0 * v.weights.x
                 + j1 * v.weights.y
                 + j2 * v.weights.z
                 + j3 * v.weights.w;

    let skinned_pos = (skin_mat * vec4<f32>(v.position, 1.0)).xyz;
    let skinned_nor = normalize((skin_mat * vec4<f32>(v.normal, 0.0)).xyz);
    let skinned_tan = normalize((skin_mat * vec4<f32>(v.tangent.xyz, 0.0)).xyz);

    let world_pos = (object.model * vec4<f32>(skinned_pos, 1.0)).xyz;
    let world_nor = normalize((object.normal_mat * vec4<f32>(skinned_nor, 0.0)).xyz);
    let world_tan = normalize((object.normal_mat * vec4<f32>(skinned_tan, 0.0)).xyz);

    var out: VertexOutput;
    out.clip_pos   = camera.view_proj * vec4<f32>(world_pos, 1.0);
    out.world_pos  = world_pos;
    out.world_norm = world_nor;
    out.world_tan  = vec4<f32>(world_tan, v.tangent.w);
    out.uv         = v.uv;
    return out;
}
