// ============================================================
// skybox.wgsl — Procedural gradient sky
//
// Renders a simple sky gradient behind all scene geometry.
// Uses the depth buffer to skip pixels where geometry was drawn.
//
// For a full skybox, replace the gradient with a cubemap sample.
// The binding is left as a placeholder for easy extension.
// ============================================================

struct SkyParams {
    horizon_color: vec3<f32>,
    _pad0:         f32,
    zenith_color:  vec3<f32>,
    _pad1:         f32,
    sun_direction: vec3<f32>,  // normalized, points toward sun
    sun_intensity: f32,
    sun_size:      f32,        // angular size (radians)
    _pad2:         f32,
    _pad3:         f32,
    _pad4:         f32,
}

struct CameraUniforms {
    view_proj:     mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos:    vec3<f32>,
    _pad:          f32,
}

@group(0) @binding(0) var<uniform> sky_params: SkyParams;
@group(0) @binding(1) var<uniform> camera:     CameraUniforms;
@group(0) @binding(2) var          depth_tex:  texture_depth_2d;

struct FragInput {
    @builtin(position) frag_coord: vec4<f32>,
    @location(0)       uv:         vec2<f32>,
}

@fragment
fn fs_main(in: FragInput) -> @location(0) vec4<f32> {
    let pixel = vec2<u32>(u32(in.frag_coord.x), u32(in.frag_coord.y));
    let depth  = textureLoad(depth_tex, pixel, 0);

    // Only render sky where no geometry was drawn (depth = far plane)
    if depth < 0.9999 { discard; }

    // Reconstruct view direction from UV
    let ndc = vec4<f32>(in.uv * 2.0 - 1.0, 0.5, 1.0);
    let ndc_f = vec4<f32>(ndc.x, -ndc.y, ndc.z, 1.0);
    let world = camera.inv_view_proj * ndc_f;
    let view_dir = normalize(world.xyz / world.w - camera.camera_pos);

    // Gradient: horizon (y≈0) → zenith (y=1)
    let t       = clamp(view_dir.y * 0.5 + 0.5, 0.0, 1.0);
    var sky     = mix(sky_params.horizon_color, sky_params.zenith_color, t * t);

    // Simple sun disc
    let sun_dot   = dot(view_dir, sky_params.sun_direction);
    let sun_angle = acos(clamp(sun_dot, -1.0, 1.0));
    if sun_angle < sky_params.sun_size {
        let sun_falloff = 1.0 - sun_angle / sky_params.sun_size;
        sky += vec3<f32>(1.0, 0.95, 0.8) * sky_params.sun_intensity
             * sun_falloff * sun_falloff;
    }

    return vec4<f32>(sky, 1.0);
}
