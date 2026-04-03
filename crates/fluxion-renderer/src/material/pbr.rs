// ============================================================
// fluxion-renderer — PbrMaterial (GPU-resident)
//
// PbrMaterial is the runtime material — textures are loaded,
// uniform buffer is created, bind group is assembled.
// Created from MaterialAsset by the renderer.
//
// PbrParams: the uniform buffer layout. Must match PbrParams in
// geometry.frag.wgsl (field order, types, padding).
// ============================================================

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use wgpu::{Device, Queue, BindGroupLayout};

use super::material_asset::MaterialAsset;
use crate::texture::{GpuTexture, TextureCache};

// ── PbrParams (UBO layout) ─────────────────────────────────────────────────────

/// Must exactly match the `PbrParams` struct in geometry.frag.wgsl.
/// Field order and padding are critical — Rust and WGSL must agree.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct PbrParams {
    pub color:              [f32; 4],  // base color RGBA linear
    pub emissive:           [f32; 3],
    pub emissive_intensity: f32,
    pub roughness:          f32,
    pub metalness:          f32,
    pub normal_scale:       f32,
    pub ao_intensity:       f32,
    pub uv_scale:           [f32; 2],
    pub uv_offset:          [f32; 2],
    /// Bitfield: bit 0=albedo tex, 1=normal tex, 2=orm tex, 3=emissive tex.
    pub texture_flags:      u32,
    pub _pad:               [f32; 3],
}

// ── PbrMaterial ────────────────────────────────────────────────────────────────

/// GPU-resident PBR material.
///
/// Contains the GPU uniform buffer and the bind group that references
/// all textures + the uniform. The renderer binds this at group(2) before
/// each draw call.
pub struct PbrMaterial {
    pub name:          String,
    pub params_buffer: wgpu::Buffer,
    pub bind_group:    wgpu::BindGroup,
    /// Cached Arc handles so textures stay alive as long as the material does.
    _textures: Vec<Arc<GpuTexture>>,
    /// Current params (kept for runtime modification by scripts).
    pub params: PbrParams,
    /// Whether this material requires alpha blending (draw order matters).
    pub is_transparent: bool,
    pub double_sided:   bool,
}

impl PbrMaterial {
    /// Build a 1×1 white fallback texture for unbound slots.
    fn white_texture(device: &Device, queue: &Queue) -> Arc<GpuTexture> {
        let tex = GpuTexture::from_rgba8(device, queue, "fallback_white", 1, 1,
            &[255, 255, 255, 255]);
        Arc::new(tex)
    }

    fn flat_normal_texture(device: &Device, queue: &Queue) -> Arc<GpuTexture> {
        // Flat normal map: RGB = (0.5, 0.5, 1.0) → decoded as (0,0,1) = straight up
        let tex = GpuTexture::from_rgba8(device, queue, "fallback_normal", 1, 1,
            &[128, 128, 255, 255]);
        Arc::new(tex)
    }

    /// Create a GPU material from a MaterialAsset descriptor.
    pub fn from_asset(
        device:   &Device,
        queue:    &Queue,
        asset:    &MaterialAsset,
        layout:   &BindGroupLayout,
        textures: &mut TextureCache,
    ) -> anyhow::Result<Self> {
        use wgpu::util::DeviceExt;

        // Load textures (or fallbacks)
        let fallback_white  = Self::white_texture(device, queue);
        let fallback_normal = Self::flat_normal_texture(device, queue);

        let mut load_or = |path: &Option<String>, fallback: &Arc<GpuTexture>| -> Arc<GpuTexture> {
            match path {
                Some(p) => textures.get_or_load(device, queue, p)
                    .unwrap_or_else(|e| { log::warn!("Texture load failed: {e}"); fallback.clone() }),
                None    => fallback.clone(),
            }
        };

        let albedo_tex   = load_or(&asset.albedo_map,    &fallback_white);
        let normal_tex   = load_or(&asset.normal_map,    &fallback_normal);
        let orm_tex      = load_or(&asset.roughness_map, &fallback_white); // use roughness slot for ORM
        let emissive_tex = load_or(&asset.emissive_map,  &fallback_white);

        // Build texture_flags bitfield
        let mut texture_flags = 0u32;
        if asset.albedo_map.is_some()    { texture_flags |= 1 << 0; }
        if asset.normal_map.is_some()    { texture_flags |= 1 << 1; }
        if asset.roughness_map.is_some() { texture_flags |= 1 << 2; }
        if asset.emissive_map.is_some()  { texture_flags |= 1 << 3; }

        // Get UV transform for the primary slot
        let uv = asset.uv_transforms.get("albedo")
            .cloned()
            .unwrap_or_default();

        let params = PbrParams {
            color:              asset.color,
            emissive:           asset.emissive,
            emissive_intensity: asset.emissive_intensity,
            roughness:          asset.roughness,
            metalness:          asset.metalness,
            normal_scale:       asset.normal_scale,
            ao_intensity:       asset.ao_intensity,
            uv_scale:           uv.scale,
            uv_offset:          uv.offset,
            texture_flags,
            _pad:               [0.0; 3],
        };

        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some(&format!("{}_params", asset.name)),
            contents: bytemuck::bytes_of(&params),
            usage:    wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some(&format!("{}_bind_group", asset.name)),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&albedo_tex.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&albedo_tex.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&normal_tex.view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&normal_tex.sampler) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&orm_tex.view) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&orm_tex.sampler) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&emissive_tex.view) },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::Sampler(&emissive_tex.sampler) },
            ],
        });

        Ok(PbrMaterial {
            name:           asset.name.clone(),
            params_buffer,
            bind_group,
            _textures:      vec![albedo_tex, normal_tex, orm_tex, emissive_tex],
            params,
            is_transparent: matches!(asset.alpha_mode, super::material_asset::AlphaMode::Blend),
            double_sided:   asset.double_sided,
        })
    }

    /// Update the GPU uniform buffer after `params` has been modified.
    pub fn upload_params(&self, queue: &Queue) {
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&self.params));
    }
}

// ── MaterialRegistry ──────────────────────────────────────────────────────────

/// Stores all GPU materials by handle (u32).
pub struct MaterialRegistry {
    materials: Vec<Option<PbrMaterial>>,
    default_handle: u32,
}

impl MaterialRegistry {
    pub fn new(device: &Device, queue: &Queue, layout: &BindGroupLayout) -> Self {
        let mut reg = Self { materials: Vec::new(), default_handle: 0 };
        // Slot 0: built-in default grey PBR material
        let default_asset = MaterialAsset::default();
        let mut tex_cache = TextureCache::new();
        let default_mat = PbrMaterial::from_asset(device, queue, &default_asset, layout, &mut tex_cache)
            .expect("default material creation should never fail");
        reg.materials.push(Some(default_mat));
        reg
    }

    pub fn default_handle(&self) -> u32 { self.default_handle }

    pub fn add(&mut self, mat: PbrMaterial) -> u32 {
        if let Some(slot) = self.materials.iter().position(|s| s.is_none()) {
            self.materials[slot] = Some(mat);
            return slot as u32;
        }
        let h = self.materials.len() as u32;
        self.materials.push(Some(mat));
        h
    }

    pub fn get(&self, handle: u32) -> Option<&PbrMaterial> {
        self.materials.get(handle as usize)?.as_ref()
    }

    pub fn get_mut(&mut self, handle: u32) -> Option<&mut PbrMaterial> {
        self.materials.get_mut(handle as usize)?.as_mut()
    }

    pub fn remove(&mut self, handle: u32) {
        if let Some(slot) = self.materials.get_mut(handle as usize) { *slot = None; }
    }
}
