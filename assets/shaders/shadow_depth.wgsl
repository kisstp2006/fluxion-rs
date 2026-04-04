// ============================================================
// shadow_depth.wgsl — Shadow map depth-only pass
//
// Renders scene geometry from the light's point of view to
// produce a depth map used for shadow testing in pbr_lighting.wgsl.
//
// Only outputs depth — no color attachment.
//
// Bind groups:
//   group(0) binding(0) — ShadowCameraUniforms (light_view_proj)
//   group(1) binding(0) — ModelUniforms (world_matrix)
// ============================================================

struct ShadowCamera {
    light_view_proj: mat4x4<f32>,
}

struct ModelUniforms {
    world_matrix:  mat4x4<f32>,
    normal_matrix: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> shadow_cam: ShadowCamera;
@group(1) @binding(0) var<uniform> model: ModelUniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    // normal, tangent, uv exist in the buffer but are unused here
    @location(1) _normal:  vec3<f32>,
    @location(2) _tangent: vec4<f32>,
    @location(3) _uv:      vec2<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> @builtin(position) vec4<f32> {
    let world_pos = model.world_matrix * vec4<f32>(in.position, 1.0);
    return shadow_cam.light_view_proj * world_pos;
}
