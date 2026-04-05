// ============================================================
// fluxion-renderer — CPU-side primitive mesh builders
//
// Each function returns (Vec<Vertex>, Vec<u32>) ready to upload
// to the GPU via GpuMesh::upload().
//
// Conventions:
//   - All primitives are centered at the origin
//   - Scale: 1.0 unit = 1 meter (before applying Transform scale)
//   - Winding: counter-clockwise (standard for right-hand coordinate system)
//   - UVs: (0,0) = bottom-left, (1,1) = top-right
// ============================================================

use super::Vertex;
use std::f32::consts::PI;

// ── Cube ──────────────────────────────────────────────────────────────────────

/// Unit cube centered at origin. Each face has its own 4 vertices
/// (no vertex sharing between faces, so normals are sharp).
pub fn cube() -> (Vec<Vertex>, Vec<u32>) {
    // 6 faces × 4 vertices = 24 vertices
    // Face order: +X, -X, +Y, -Y, +Z, -Z
    let faces: &[([f32; 3], [f32; 3], [f32; 3], f32)] = &[
        // (center_offset, normal, tangent, bitangent_sign)
        ([1.0, 0.0, 0.0],  [1.0,0.0,0.0],  [0.0,0.0,-1.0], 1.0),  // +X
        ([-1.0, 0.0, 0.0], [-1.0,0.0,0.0], [0.0,0.0, 1.0], 1.0),  // -X
        ([0.0, 1.0, 0.0],  [0.0,1.0,0.0],  [1.0,0.0, 0.0], 1.0),  // +Y
        ([0.0,-1.0, 0.0],  [0.0,-1.0,0.0], [1.0,0.0, 0.0],-1.0),  // -Y
        ([0.0, 0.0, 1.0],  [0.0,0.0,1.0],  [1.0,0.0, 0.0], 1.0),  // +Z
        ([0.0, 0.0,-1.0],  [0.0,0.0,-1.0], [-1.0,0.0,0.0], 1.0),  // -Z
    ];

    let mut vertices = Vec::with_capacity(24);
    let mut indices  = Vec::with_capacity(36);
    let h = 0.5_f32;

    for (i, &(_n_off, normal, tangent, bitan_sign)) in faces.iter().enumerate() {
        let n = glam::Vec3::from(normal);
        let t = glam::Vec3::from(tangent);
        let b = n.cross(t) * bitan_sign;

        // Four corners of the face quad
        let corners = [
            (t * -h + b * -h + n * h),
            (t *  h + b * -h + n * h),
            (t *  h + b *  h + n * h),
            (t * -h + b *  h + n * h),
        ];
        let uvs = [[0.0, 0.0_f32], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

        let base = (i * 4) as u32;
        for (_j, (pos, uv)) in corners.iter().zip(uvs.iter()).enumerate() {
            vertices.push(Vertex {
                position: pos.to_array(),
                normal,
                tangent:  [tangent[0], tangent[1], tangent[2], bitan_sign],
                uv:       *uv,
            });
        }
        // Two triangles: 0-1-2 and 0-2-3
        indices.extend_from_slice(&[base, base+1, base+2, base, base+2, base+3]);
    }

    (vertices, indices)
}

// ── Sphere ────────────────────────────────────────────────────────────────────

/// UV sphere with `stacks` latitude rings and `slices` longitude segments.
/// Radius = 0.5 (unit diameter).
pub fn sphere(stacks: u32, slices: u32) -> (Vec<Vertex>, Vec<u32>) {
    let r = 0.5_f32;
    let mut vertices = Vec::new();
    let mut indices  = Vec::new();

    for s in 0..=stacks {
        let phi   = PI * (s as f32) / (stacks as f32); // [0, π]
        let sin_p = phi.sin();
        let cos_p = phi.cos();

        for l in 0..=slices {
            let theta = 2.0 * PI * (l as f32) / (slices as f32);
            let sin_t = theta.sin();
            let cos_t = theta.cos();

            let x = sin_p * cos_t;
            let y = cos_p;
            let z = sin_p * sin_t;

            let normal  = [x, y, z];
            let tangent = [-sin_t, 0.0, cos_t, 1.0];
            let uv      = [(l as f32) / (slices as f32), (s as f32) / (stacks as f32)];

            vertices.push(Vertex {
                position: [x * r, y * r, z * r],
                normal,
                tangent,
                uv,
            });
        }
    }

    for s in 0..stacks {
        for l in 0..slices {
            let a = s * (slices + 1) + l;
            let b = a + slices + 1;
            indices.extend_from_slice(&[a, b, a+1, b, b+1, a+1]);
        }
    }

    (vertices, indices)
}

// ── Plane ────────────────────────────────────────────────────────────────────

/// Unit plane on the XZ plane (Y = 0), 1×1 meter, centered at origin.
pub fn plane() -> (Vec<Vertex>, Vec<u32>) {
    let h = 0.5_f32;
    let normal  = [0.0_f32, 1.0, 0.0];
    let tangent = [1.0_f32, 0.0, 0.0, 1.0];

    let vertices = vec![
        Vertex { position: [-h, 0.0, -h], normal, tangent, uv: [0.0, 0.0] },
        Vertex { position: [ h, 0.0, -h], normal, tangent, uv: [1.0, 0.0] },
        Vertex { position: [ h, 0.0,  h], normal, tangent, uv: [1.0, 1.0] },
        Vertex { position: [-h, 0.0,  h], normal, tangent, uv: [0.0, 1.0] },
    ];
    let indices = vec![0, 2, 1, 0, 3, 2];
    (vertices, indices)
}

// ── Cylinder ──────────────────────────────────────────────────────────────────

/// Cylinder with `slices` segments around the circumference.
/// Radius = 0.5, height = 1.0, centered at origin.
pub fn cylinder(slices: u32) -> (Vec<Vertex>, Vec<u32>) {
    let r = 0.5_f32;
    let h = 0.5_f32;
    let mut vertices = Vec::new();
    let mut indices  = Vec::new();

    // Side surface
    for s in 0..=slices {
        let theta  = 2.0 * PI * (s as f32) / (slices as f32);
        let sin_t  = theta.sin();
        let cos_t  = theta.cos();
        let normal = [cos_t, 0.0, sin_t];
        let tangent = [-sin_t, 0.0, cos_t, 1.0];
        let uv_u   = (s as f32) / (slices as f32);

        vertices.push(Vertex { position: [cos_t * r, -h, sin_t * r], normal, tangent, uv: [uv_u, 0.0] });
        vertices.push(Vertex { position: [cos_t * r,  h, sin_t * r], normal, tangent, uv: [uv_u, 1.0] });
    }
    let _side_verts = vertices.len() as u32;
    for s in 0..slices {
        let b = s * 2;
        indices.extend_from_slice(&[b, b+1, b+2, b+1, b+3, b+2]);
    }

    // Top cap
    let top_center = vertices.len() as u32;
    vertices.push(Vertex { position: [0.0, h, 0.0], normal: [0.0,1.0,0.0], tangent: [1.0,0.0,0.0,1.0], uv: [0.5,0.5] });
    for s in 0..slices {
        let theta = 2.0 * PI * (s as f32) / (slices as f32);
        let (sin_t, cos_t) = (theta.sin(), theta.cos());
        vertices.push(Vertex { position: [cos_t*r, h, sin_t*r], normal: [0.0,1.0,0.0], tangent: [1.0,0.0,0.0,1.0], uv: [cos_t*0.5+0.5, sin_t*0.5+0.5] });
    }
    for s in 0..slices {
        let v0 = top_center;
        let v1 = top_center + 1 + s;
        let v2 = top_center + 1 + (s + 1) % slices;
        indices.extend_from_slice(&[v0, v1, v2]);
    }

    // Bottom cap
    let bot_center = vertices.len() as u32;
    vertices.push(Vertex { position: [0.0, -h, 0.0], normal: [0.0,-1.0,0.0], tangent: [1.0,0.0,0.0,1.0], uv: [0.5,0.5] });
    for s in 0..slices {
        let theta = 2.0 * PI * (s as f32) / (slices as f32);
        let (sin_t, cos_t) = (theta.sin(), theta.cos());
        vertices.push(Vertex { position: [cos_t*r, -h, sin_t*r], normal: [0.0,-1.0,0.0], tangent: [1.0,0.0,0.0,1.0], uv: [cos_t*0.5+0.5, sin_t*0.5+0.5] });
    }
    for s in 0..slices {
        let v0 = bot_center;
        let v1 = bot_center + 1 + (s + 1) % slices;
        let v2 = bot_center + 1 + s;
        indices.extend_from_slice(&[v0, v1, v2]);
    }

    (vertices, indices)
}

// ── Capsule ───────────────────────────────────────────────────────────────────

/// Capsule: cylinder + hemispherical end-caps.
/// Radius = 0.5, total height = 1.0 (cylinder body height = 0.5, cap height = 0.25 each).
pub fn capsule(slices: u32, stacks_per_cap: u32) -> (Vec<Vertex>, Vec<u32>) {
    let r       = 0.5_f32;
    let half_h  = 0.25_f32; // half-height of cylindrical body
    let mut vertices = Vec::new();
    let mut indices  = Vec::new();

    // Helper: add a hemisphere (top=true for +Y cap, top=false for -Y cap)
    let mut add_hemisphere = |y_offset: f32, flip: bool| {
        let y_sign = if flip { -1.0_f32 } else { 1.0 };
        let base   = vertices.len() as u32;
        for s in 0..=stacks_per_cap {
            let phi   = (PI * 0.5) * (s as f32) / (stacks_per_cap as f32);
            let phi   = if flip { PI - phi } else { phi }; // flip for bottom cap
            let sin_p = phi.sin();
            let cos_p = phi.cos() * y_sign;
            for l in 0..=slices {
                let theta  = 2.0 * PI * (l as f32) / (slices as f32);
                let (sin_t, cos_t) = (theta.sin(), theta.cos());
                let nx = sin_p * cos_t;
                let ny = cos_p;
                let nz = sin_p * sin_t;
                vertices.push(Vertex {
                    position: [nx * r, ny * r + y_offset, nz * r],
                    normal:   [nx, ny, nz],
                    tangent:  [-sin_t, 0.0, cos_t, 1.0],
                    uv:       [(l as f32)/(slices as f32), (s as f32)/(stacks_per_cap as f32)],
                });
            }
        }
        for s in 0..stacks_per_cap {
            for l in 0..slices {
                let a = base + s * (slices + 1) + l;
                let b = a + slices + 1;
                indices.extend_from_slice(&[a, b, a+1, b, b+1, a+1]);
            }
        }
    };

    add_hemisphere( half_h, false); // top cap
    add_hemisphere(-half_h, true);  // bottom cap

    // Cylindrical body (just the side surface, no caps)
    let body_base = vertices.len() as u32;
    for s in 0..=slices {
        let theta  = 2.0 * PI * (s as f32) / (slices as f32);
        let (sin_t, cos_t) = (theta.sin(), theta.cos());
        let normal  = [cos_t, 0.0, sin_t];
        let tangent = [-sin_t, 0.0, cos_t, 1.0];
        let uv_u   = (s as f32) / (slices as f32);
        vertices.push(Vertex { position: [cos_t*r, -half_h, sin_t*r], normal, tangent, uv: [uv_u, 0.0] });
        vertices.push(Vertex { position: [cos_t*r,  half_h, sin_t*r], normal, tangent, uv: [uv_u, 1.0] });
    }
    for s in 0..slices {
        let b = body_base + s * 2;
        indices.extend_from_slice(&[b, b+1, b+2, b+1, b+3, b+2]);
    }

    (vertices, indices)
}
