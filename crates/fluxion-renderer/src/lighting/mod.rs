// ============================================================
// fluxion-renderer — Light data GPU layout
//
// Mirrors the LightBuffer struct in pbr_lighting.wgsl.
// Must be kept in sync with the shader — any field rename or
// reorder here must match the shader.
//
// Using bytemuck::Pod/Zeroable for safe transmutation to bytes.
// ============================================================

use bytemuck::{Pod, Zeroable};
use wgpu::{Device, Queue};

pub const MAX_LIGHTS: usize = 64;

// Light type constants — match WGSL shader constants
pub const LIGHT_DIRECTIONAL: u32 = 0;
pub const LIGHT_POINT:       u32 = 1;
pub const LIGHT_SPOT:        u32 = 2;

/// Per-light GPU data. Layout must match `LightData` struct in pbr_lighting.wgsl.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct LightUniform {
    pub position:   [f32; 3],
    pub light_type: u32,
    pub direction:  [f32; 3],
    pub range:      f32,
    pub color:      [f32; 3],
    pub intensity:  f32,
    pub spot_angle: f32,  // cos(outer half-angle)
    pub spot_inner: f32,  // cos(inner half-angle)
    pub _pad0:      f32,
    pub _pad1:      f32,
}

/// Entire light buffer uploaded as one UBO. Layout matches `LightBuffer` in WGSL.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct LightBufferData {
    pub count:           u32,
    pub _pad0:           u32,
    pub _pad1:           u32,
    pub _pad2:           u32,
    /// Flat ambient light color (linear RGB). Added to all surfaces regardless of normals.
    pub ambient_color:   [f32; 3],
    pub ambient_intensity: f32,
    pub lights: [LightUniform; MAX_LIGHTS],
    /// Fog settings (matches [`fluxion_core::components::environment::FogSettings`]).
    pub fog_color:       [f32; 3],
    pub fog_density:     f32,
    pub fog_enabled:     u32,
    /// 0 = Exponential, 1 = Linear
    pub fog_mode:        u32,
    pub fog_near:        f32,
    pub fog_far:         f32,
}

impl LightBufferData {
    pub fn new() -> Self {
        Self {
            count:             0,
            _pad0:             0, _pad1: 0, _pad2: 0,
            ambient_color:     [0.5, 0.6, 0.7],  // sky blue tint
            ambient_intensity: 0.08,
            lights:            [LightUniform::zeroed(); MAX_LIGHTS],
            fog_color:         [0.5, 0.6, 0.7],
            fog_density:       0.01,
            fog_enabled:       0,
            fog_mode:          0,
            fog_near:          10.0,
            fog_far:           100.0,
        }
    }

    /// Push a light into the buffer. Silently ignores lights beyond MAX_LIGHTS.
    pub fn push(&mut self, light: LightUniform) {
        if (self.count as usize) < MAX_LIGHTS {
            self.lights[self.count as usize] = light;
            self.count += 1;
        }
    }

    pub fn clear(&mut self) {
        self.count = 0;
    }
}

impl Default for LightBufferData { fn default() -> Self { Self::new() } }

/// GPU buffer wrapper for the light list.
pub struct LightBuffer {
    pub gpu_buffer: wgpu::Buffer,
}

impl LightBuffer {
    pub fn new(device: &Device) -> Self {
        use std::mem::size_of;
        let gpu_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("light_buffer"),
            size:               size_of::<LightBufferData>() as u64,
            usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { gpu_buffer }
    }

    /// Upload the current light list to the GPU.
    pub fn upload(&self, queue: &Queue, data: &LightBufferData) {
        queue.write_buffer(&self.gpu_buffer, 0, bytemuck::bytes_of(data));
    }
}
