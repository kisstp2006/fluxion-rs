// ============================================================
// fluxion-renderer — SkinnedVertex + SkinnedGpuMesh
//
// Extends the standard Vertex with bone weights so the skinned
// geometry pass can blend joint transforms in the vertex shader.
//
// Layout (matches skinned_geometry.vert.wgsl):
//   location(0) = position   vec3
//   location(1) = normal     vec3
//   location(2) = tangent    vec4
//   location(3) = uv         vec2
//   location(4) = joints     vec4<u32>  (up to 4 joint indices)
//   location(5) = weights    vec4<f32>  (corresponding blend weights, sum ≈ 1)
// ============================================================

use bytemuck::{Pod, Zeroable};
use wgpu::Device;

/// One vertex in the skinned mesh layout.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct SkinnedVertex {
    pub position: [f32; 3],
    pub normal:   [f32; 3],
    pub tangent:  [f32; 4],
    pub uv:       [f32; 2],
    /// Up to 4 joint indices (0-padded if fewer influences).
    pub joints:   [u32; 4],
    /// Blend weights (0.0-1.0, sum ≈ 1.0).
    pub weights:  [f32; 4],
}

impl SkinnedVertex {
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        use std::mem::size_of;
        wgpu::VertexBufferLayout {
            array_stride: size_of::<SkinnedVertex>() as wgpu::BufferAddress,
            step_mode:    wgpu::VertexStepMode::Vertex,
            attributes:   &[
                wgpu::VertexAttribute { offset: 0,  shader_location: 0, format: wgpu::VertexFormat::Float32x3 },
                wgpu::VertexAttribute { offset: 12, shader_location: 1, format: wgpu::VertexFormat::Float32x3 },
                wgpu::VertexAttribute { offset: 24, shader_location: 2, format: wgpu::VertexFormat::Float32x4 },
                wgpu::VertexAttribute { offset: 40, shader_location: 3, format: wgpu::VertexFormat::Float32x2 },
                wgpu::VertexAttribute { offset: 48, shader_location: 4, format: wgpu::VertexFormat::Uint32x4  },
                wgpu::VertexAttribute { offset: 64, shader_location: 5, format: wgpu::VertexFormat::Float32x4 },
            ],
        }
    }
}

// ── SkinnedGpuMesh ────────────────────────────────────────────────────────────

/// GPU-resident skinned mesh.  Owns vertex + index buffers.
pub struct SkinnedGpuMesh {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer:  wgpu::Buffer,
    pub index_count:   u32,
    pub label:         String,
}

impl SkinnedGpuMesh {
    pub fn upload(device: &Device, label: &str, vertices: &[SkinnedVertex], indices: &[u32]) -> Self {
        use wgpu::util::DeviceExt;

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some(&format!("{label}_skv")),
            contents: bytemuck::cast_slice(vertices),
            usage:    wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some(&format!("{label}_ski")),
            contents: bytemuck::cast_slice(indices),
            usage:    wgpu::BufferUsages::INDEX,
        });
        Self { vertex_buffer, index_buffer, index_count: indices.len() as u32, label: label.to_string() }
    }
}

// ── SkinnedMeshRegistry ───────────────────────────────────────────────────────

/// Stores skinned GPU meshes by handle (separate from `MeshRegistry`).
pub struct SkinnedMeshRegistry {
    meshes: Vec<Option<SkinnedGpuMesh>>,
}

impl SkinnedMeshRegistry {
    pub fn new() -> Self { Self { meshes: Vec::new() } }

    pub fn add(&mut self, mesh: SkinnedGpuMesh) -> u32 {
        if let Some(slot) = self.meshes.iter().position(|s| s.is_none()) {
            self.meshes[slot] = Some(mesh);
            return slot as u32;
        }
        let h = self.meshes.len() as u32;
        self.meshes.push(Some(mesh));
        h
    }

    pub fn get(&self, handle: u32) -> Option<&SkinnedGpuMesh> {
        self.meshes.get(handle as usize)?.as_ref()
    }

    pub fn remove(&mut self, handle: u32) {
        if let Some(s) = self.meshes.get_mut(handle as usize) { *s = None; }
    }
}

impl Default for SkinnedMeshRegistry { fn default() -> Self { Self::new() } }
