// ============================================================
// fluxion-renderer — glTF / glB mesh import
//
// Loads the first mesh in the file (all triangle primitives merged)
// into our deferred PBR vertex layout (position, normal, tangent, uv).
//
// See also: [learn-wgpu — models](https://sotrh.github.io/learn-wgpu/beginner/tutorial9-models/),
// [renderling](https://github.com/schell/renderling) for larger-scale glTF + wgpu patterns.
// ============================================================

use std::path::Path;

use anyhow::Context;
use glam::{Vec2, Vec3};

use super::Vertex;

/// Load all **Triangles** primitives of the **first** mesh in `path` (.gltf / .glb).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_gltf_path(path: &Path) -> anyhow::Result<(Vec<Vertex>, Vec<u32>)> {
    let path_str = path.to_str().context("path must be UTF-8")?;
    let (doc, buffers, _images) = gltf::import(path_str).with_context(|| format!("gltf::import({path_str})"))?;
    load_first_mesh(&doc, &buffers)
}

/// Load from an in-memory `.glb` / `.gltf` slice (e.g. after `fetch` on WASM).
pub fn load_gltf_slice(data: &[u8]) -> anyhow::Result<(Vec<Vertex>, Vec<u32>)> {
    let (doc, buffers, _images) = gltf::import_slice(data).context("gltf::import_slice")?;
    load_first_mesh(&doc, &buffers)
}

fn load_first_mesh(doc: &gltf::Document, buffers: &[gltf::buffer::Data]) -> anyhow::Result<(Vec<Vertex>, Vec<u32>)> {
    let mesh = doc
        .meshes()
        .next()
        .context("glTF contains no meshes")?;

    let mut all_vertices: Vec<Vertex> = Vec::new();
    let mut all_indices: Vec<u32> = Vec::new();

    for prim in mesh.primitives() {
        if prim.mode() != gltf::mesh::Mode::Triangles {
            log::warn!(
                "[gltf] Skipping primitive with mode {:?} (only Triangles supported)",
                prim.mode()
            );
            continue;
        }

        let reader = prim.reader(|buffer| buffers.get(buffer.index()).map(|d| d.as_ref()));

        let positions: Vec<[f32; 3]> = reader
            .read_positions()
            .context("primitive missing POSITION")?
            .collect();

        if positions.is_empty() {
            continue;
        }

        let count = positions.len();

        let indices: Vec<u32> = if let Some(iter) = reader.read_indices() {
            iter.into_u32().collect()
        } else {
            (0..count as u32).collect()
        };

        let normals: Vec<[f32; 3]> = if let Some(iter) = reader.read_normals() {
            let v: Vec<_> = iter.collect();
            if v.len() != count {
                anyhow::bail!("NORMAL count {} != POSITION count {}", v.len(), count);
            }
            v
        } else {
            compute_vertex_normals(&positions, &indices)
        };

        let uvs: Vec<[f32; 2]> = if let Some(tc) = reader.read_tex_coords(0) {
            let v: Vec<_> = tc.into_f32().map(|[u, v]| [u, v]).collect();
            if v.len() != count {
                anyhow::bail!("TEXCOORD_0 count {} != POSITION count {}", v.len(), count);
            }
            v
        } else {
            vec![[0.0, 0.0]; count]
        };

        let tangents: Vec<[f32; 4]> = if let Some(iter) = reader.read_tangents() {
            let v: Vec<_> = iter.collect();
            if v.len() != count {
                anyhow::bail!("TANGENT count {} != POSITION count {}", v.len(), count);
            }
            v
        } else {
            compute_tangents(&positions, &normals, &uvs, &indices)
        };

        let base = all_vertices.len() as u32;
        for i in 0..count {
            all_vertices.push(Vertex {
                position: positions[i],
                normal:   normals[i],
                tangent:  tangents[i],
                uv:       uvs[i],
            });
        }

        for idx in &indices {
            all_indices.push(base + idx);
        }
    }

    anyhow::ensure!(
        !all_vertices.is_empty() && !all_indices.is_empty(),
        "glTF mesh produced no triangle geometry"
    );

    Ok((all_vertices, all_indices))
}

/// Face normals averaged per vertex when glTF omits NORMAL.
fn compute_vertex_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut acc: Vec<Vec3> = vec![Vec3::ZERO; positions.len()];
    for tri in indices.chunks(3) {
        if tri.len() < 3 {
            continue;
        }
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() {
            continue;
        }
        let p0 = Vec3::from(positions[i0]);
        let p1 = Vec3::from(positions[i1]);
        let p2 = Vec3::from(positions[i2]);
        let n = (p1 - p0).cross(p2 - p0);
        if n.length_squared() > 1e-20 {
            let n = n.normalize();
            acc[i0] += n;
            acc[i1] += n;
            acc[i2] += n;
        }
    }
    acc.into_iter()
        .map(|v| {
            if v.length_squared() > 1e-20 {
                v.normalize().to_array()
            } else {
                [0.0, 1.0, 0.0]
            }
        })
        .collect()
}

/// Tangent (xyz) + handedness (w) for normal mapping.
fn compute_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Vec<[f32; 4]> {
    let mut tan1 = vec![Vec3::ZERO; positions.len()];
    let mut tan2 = vec![Vec3::ZERO; positions.len()];

    for tri in indices.chunks(3) {
        if tri.len() < 3 {
            continue;
        }
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() {
            continue;
        }

        let p0 = Vec3::from(positions[i0]);
        let p1 = Vec3::from(positions[i1]);
        let p2 = Vec3::from(positions[i2]);
        let uv0 = Vec2::new(uvs[i0][0], uvs[i0][1]);
        let uv1 = Vec2::new(uvs[i1][0], uvs[i1][1]);
        let uv2 = Vec2::new(uvs[i2][0], uvs[i2][1]);

        let q1 = p1 - p0;
        let q2 = p2 - p0;
        let duv1 = uv1 - uv0;
        let duv2 = uv2 - uv0;
        let det = duv1.x * duv2.y - duv2.x * duv1.y;
        if det.abs() < 1e-12 {
            continue;
        }
        let r = 1.0 / det;
        let tangent = (q1 * duv2.y - q2 * duv1.y) * r;
        let bitangent = (q2 * duv1.x - q1 * duv2.x) * r;

        tan1[i0] += tangent;
        tan1[i1] += tangent;
        tan1[i2] += tangent;
        tan2[i0] += bitangent;
        tan2[i1] += bitangent;
        tan2[i2] += bitangent;
    }

    let mut out = Vec::with_capacity(positions.len());
    for i in 0..positions.len() {
        let n = Vec3::from(normals[i]);
        let mut t = tan1[i];
        t = t - n * n.dot(t);
        if t.length_squared() < 1e-20 {
            t = n.cross(if n.y.abs() < 0.9 { Vec3::Y } else { Vec3::X });
        }
        t = t.normalize();
        let b = n.cross(t);
        let w = if tan2[i].dot(b) < 0.0 { -1.0 } else { 1.0 };
        out.push([t.x, t.y, t.z, w]);
    }
    out
}
