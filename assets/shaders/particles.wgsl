// ============================================================
// Instanced billboard particles — overlay pass (after tonemap)
// ============================================================

struct CameraUniform {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> u_camera: CameraUniform;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) color: vec4<f32>,
}

fn quad_corner(vi: u32) -> vec2<f32> {
    if vi == 0u { return vec2(-1.0, -1.0); }
    if vi == 1u { return vec2(1.0, -1.0); }
    if vi == 2u { return vec2(1.0, 1.0); }
    if vi == 3u { return vec2(-1.0, -1.0); }
    if vi == 4u { return vec2(1.0, 1.0); }
    if vi == 5u { return vec2(-1.0, 1.0); }
    return vec2(0.0);
}

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    @location(0) p_pos: vec3<f32>,
    @location(1) p_size: f32,
    @location(2) p_color: vec4<f32>,
) -> VsOut {
    let uv = quad_corner(vi);
    let cam = u_camera.camera_pos.xyz;
    let to_cam = normalize(cam - p_pos);
    var world_up = vec3<f32>(0.0, 1.0, 0.0);
    var right = normalize(cross(world_up, to_cam));
    if (length(right) < 0.01) {
        right = vec3<f32>(1.0, 0.0, 0.0);
    }
    let up = normalize(cross(to_cam, right));
    let offset = right * (uv.x * p_size) + up * (uv.y * p_size);
    let world = p_pos + offset;
    var o: VsOut;
    o.clip_pos = u_camera.view_proj * vec4<f32>(world, 1.0);
    let edge = 1.0 - smoothstep(0.82, 1.0, length(uv));
    o.color = vec4<f32>(p_color.rgb, p_color.a * edge);
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}
