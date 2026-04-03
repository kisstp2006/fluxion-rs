// ============================================================
// fluxion-renderer — GpuMesh + MeshRegistry + primitive builders
//
// GpuMesh: vertex + index buffers on the GPU, ready for draw calls.
// MeshRegistry: stores GpuMesh by handle (u32 key).
// primitives: builds cube, sphere, plane vertex data on the CPU.
// ============================================================

pub mod primitives;

use wgpu::Device;
use bytemuck::{Pod, Zeroable};

// ── Vertex layout ─────────────────────────────────────────────────────────────

/// One vertex in our standard mesh layout.
///
/// Layout matches the binding locations in geometry.vert.wgsl:
///   location(0) = position
///   location(1) = normal
///   location(2) = tangent (xyz + handedness sign in w)
///   location(3) = uv
///
/// Using repr(C) ensures the layout is deterministic (no padding surprises).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal:   [f32; 3],
    pub tangent:  [f32; 4],  // xyz = tangent, w = bitangent sign
    pub uv:       [f32; 2],
}

impl Vertex {
    /// Describe the vertex buffer layout for wgpu pipeline creation.
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        use std::mem::size_of;
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode:    wgpu::VertexStepMode::Vertex,
            attributes:   &[
                // position: vec3<f32>
                wgpu::VertexAttribute {
                    offset:          0,
                    shader_location: 0,
                    format:          wgpu::VertexFormat::Float32x3,
                },
                // normal: vec3<f32>
                wgpu::VertexAttribute {
                    offset:          12,
                    shader_location: 1,
                    format:          wgpu::VertexFormat::Float32x3,
                },
                // tangent: vec4<f32>
                wgpu::VertexAttribute {
                    offset:          24,
                    shader_location: 2,
                    format:          wgpu::VertexFormat::Float32x4,
                },
                // uv: vec2<f32>
                wgpu::VertexAttribute {
                    offset:          40,
                    shader_location: 3,
                    format:          wgpu::VertexFormat::Float32x2,
                },
            ],
        }
    }
}

// ── GpuMesh ────────────────────────────────────────────────────────────────────

/// GPU-resident mesh. Owns the vertex buffer and index buffer.
pub struct GpuMesh {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer:  wgpu::Buffer,
    pub index_count:   u32,
    pub label:         String,
}

impl GpuMesh {
    /// Upload CPU mesh data to the GPU.
    pub fn upload(device: &Device, label: &str, vertices: &[Vertex], indices: &[u32]) -> Self {
        use wgpu::util::DeviceExt;

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some(&format!("{label}_vb")),
            contents: bytemuck::cast_slice(vertices),
            usage:    wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some(&format!("{label}_ib")),
            contents: bytemuck::cast_slice(indices),
            usage:    wgpu::BufferUsages::INDEX,
        });

        Self {
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            label: label.to_string(),
        }
    }
}

// ── MeshRegistry ──────────────────────────────────────────────────────────────

/// Stores all GPU meshes by handle. Handles are stable u32 keys.
pub struct MeshRegistry {
    meshes:      Vec<Option<GpuMesh>>,
    /// Preloaded primitive meshes (Cube=0, Sphere=1, Plane=2, Cylinder=3, Capsule=4)
    primitive_handles: [u32; 5],
}

impl MeshRegistry {
    /// Initialize the registry and pre-upload all primitive meshes.
    pub fn new(device: &Device) -> Self {
        let mut meshes = Vec::new();

        // Slot 0: Cube
        let (cube_v, cube_i) = primitives::cube();
        meshes.push(Some(GpuMesh::upload(device, "primitive_cube", &cube_v, &cube_i)));

        // Slot 1: Sphere (UV sphere, 32 stacks × 32 slices)
        let (sph_v, sph_i) = primitives::sphere(32, 32);
        meshes.push(Some(GpuMesh::upload(device, "primitive_sphere", &sph_v, &sph_i)));

        // Slot 2: Plane (XZ, 1×1 unit, centered at origin)
        let (pln_v, pln_i) = primitives::plane();
        meshes.push(Some(GpuMesh::upload(device, "primitive_plane", &pln_v, &pln_i)));

        // Slot 3: Cylinder
        let (cyl_v, cyl_i) = primitives::cylinder(32);
        meshes.push(Some(GpuMesh::upload(device, "primitive_cylinder", &cyl_v, &cyl_i)));

        // Slot 4: Capsule
        let (cap_v, cap_i) = primitives::capsule(16, 8);
        meshes.push(Some(GpuMesh::upload(device, "primitive_capsule", &cap_v, &cap_i)));

        Self {
            primitive_handles: [0, 1, 2, 3, 4],
            meshes,
        }
    }

    pub fn primitive_handle(&self, kind: fluxion_core::components::mesh_renderer::PrimitiveType) -> u32 {
        use fluxion_core::components::mesh_renderer::PrimitiveType::*;
        match kind {
            Cube     => self.primitive_handles[0],
            Sphere   => self.primitive_handles[1],
            Plane    => self.primitive_handles[2],
            Cylinder => self.primitive_handles[3],
            Capsule  => self.primitive_handles[4],
        }
    }

    /// Add a mesh and return its handle.
    pub fn add(&mut self, mesh: GpuMesh) -> u32 {
        // Reuse a free slot if available
        if let Some(slot) = self.meshes.iter().position(|s| s.is_none()) {
            self.meshes[slot] = Some(mesh);
            return slot as u32;
        }
        let handle = self.meshes.len() as u32;
        self.meshes.push(Some(mesh));
        handle
    }

    pub fn get(&self, handle: u32) -> Option<&GpuMesh> {
        self.meshes.get(handle as usize)?.as_ref()
    }

    pub fn remove(&mut self, handle: u32) {
        if let Some(slot) = self.meshes.get_mut(handle as usize) {
            *slot = None;
        }
    }
}
