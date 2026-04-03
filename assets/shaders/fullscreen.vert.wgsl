// ============================================================
// fullscreen.vert.wgsl
//
// Reusable fullscreen triangle vertex shader.
// Generates a giant triangle that covers the entire viewport
// without needing a vertex buffer — the triangle is computed
// entirely from the vertex index (0, 1, 2).
//
// This technique is standard for post-processing passes:
//   - No vertex buffer setup needed
//   - The GPU clips the oversized triangle to the viewport
//   - One draw call with 3 vertices, no index buffer
//
// UV (0,0) = top-left, (1,1) = bottom-right (wgpu/Vulkan convention).
// ============================================================

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0)       uv:            vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VertexOutput {
    // Standard fullscreen triangle trick:
    // vertex 0: (-1, -1)  uv (0, 1)  bottom-left
    // vertex 1: ( 3, -1)  uv (2, 1)  bottom-right (off screen)
    // vertex 2: (-1,  3)  uv (0,-1)  top-left     (off screen)
    let x = f32(i32(vid) * 2 - 1);   // 0→-1, 1→3, 2→-1
    let y = f32(1 - i32(vid) * 2);   // produces -1, -1, 3 with offset trick

    // Simpler and correct formulation:
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(2.0, 1.0),
        vec2<f32>(0.0,-1.0),
    );

    var out: VertexOutput;
    out.clip_position = vec4<f32>(positions[vid], 0.0, 1.0);
    out.uv            = uvs[vid];
    return out;
}
