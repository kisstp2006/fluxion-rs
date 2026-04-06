// ============================================================
// fluxion-renderer — Texture management
//
// GpuTexture: a wgpu texture + view + optional sampler.
// TextureCache: Arc-based dedup so the same path is only uploaded once.
//
// Design mirrors the TypeScript engine's TextureCache (WeakRef-based),
// except we use Arc<GpuTexture> instead of WeakRef. The renderer holds
// a strong reference in the cache; when the last outside reference drops,
// the cache entry is cleaned up on the next frame (or on explicit purge).
// ============================================================

use std::collections::HashMap;
use std::sync::Arc;

use fluxion_core::assets::AssetSource;
use wgpu::{Device, Queue, TextureFormat};

/// A GPU-resident texture with its default view and a reusable sampler.
pub struct GpuTexture {
    pub texture: wgpu::Texture,
    pub view:    wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub width:   u32,
    pub height:  u32,
    pub format:  TextureFormat,
}

impl GpuTexture {
    /// Upload raw RGBA8 pixel data to the GPU.
    pub fn from_rgba8(
        device: &Device,
        queue:  &Queue,
        label:  &str,
        width:  u32,
        height: u32,
        data:   &[u8],
    ) -> Self {
        assert_eq!(data.len(), (width * height * 4) as usize,
            "RGBA8 data length mismatch");

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some(label),
            size:            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          TextureFormat::Rgba8UnormSrgb,
            usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats:    &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture:   &texture,
                mip_level: 0,
                origin:    wgpu::Origin3d::ZERO,
                aspect:    wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset:         0,
                bytes_per_row:  Some(4 * width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );

        let view = texture.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:            Some(&format!("{label}_sampler")),
            address_mode_u:   wgpu::AddressMode::Repeat,
            address_mode_v:   wgpu::AddressMode::Repeat,
            address_mode_w:   wgpu::AddressMode::Repeat,
            mag_filter:       wgpu::FilterMode::Linear,
            min_filter:       wgpu::FilterMode::Linear,
            mipmap_filter:    wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });

        Self { texture, view, sampler, width, height, format: TextureFormat::Rgba8UnormSrgb }
    }

    /// Create a GPU render target texture (not uploaded from CPU).
    pub fn render_target(
        device: &Device,
        label:  &str,
        width:  u32,
        height: u32,
        format: TextureFormat,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some(label),
            size:            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format,
            usage:           wgpu::TextureUsages::TEXTURE_BINDING
                           | wgpu::TextureUsages::RENDER_ATTACHMENT
                           | wgpu::TextureUsages::COPY_SRC
                           | wgpu::TextureUsages::COPY_DST,
            view_formats:    &[],
        });
        let view    = texture.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:         Some(&format!("{label}_sampler")),
            mag_filter:    wgpu::FilterMode::Linear,
            min_filter:    wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        Self { texture, view, sampler, width, height, format }
    }

    /// Create a depth texture.
    pub fn depth(device: &Device, label: &str, width: u32, height: u32) -> Self {
        let format  = TextureFormat::Depth32Float;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some(label),
            size:            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format,
            usage:           wgpu::TextureUsages::TEXTURE_BINDING
                           | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats:    &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            aspect: wgpu::TextureAspect::DepthOnly,
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        Self { texture, view, sampler, width, height, format }
    }
}

// ── TextureCache ───────────────────────────────────────────────────────────────

/// Reference-counted texture cache.
///
/// Maps asset path → Arc<GpuTexture>. Callers hold an Arc clone for as
/// long as they need the texture. When the last external Arc drops, the
/// cache still holds one reference; call `purge_unused()` to clean up.
pub struct TextureCache {
    entries: HashMap<String, Arc<GpuTexture>>,
}

impl TextureCache {
    pub fn new() -> Self { Self { entries: HashMap::new() } }

    /// Get or load a texture from a file path.
    /// If already loaded, returns a clone of the Arc (O(1), no GPU work).
    pub fn get_or_load(
        &mut self,
        device: &Device,
        queue:  &Queue,
        path:   &str,
    ) -> anyhow::Result<Arc<GpuTexture>> {
        if let Some(cached) = self.entries.get(path) {
            return Ok(cached.clone());
        }

        let texture = load_texture_file(device, queue, path)?;
        let arc     = Arc::new(texture);
        self.entries.insert(path.to_string(), arc.clone());
        Ok(arc)
    }

    /// Load a texture via [`AssetSource`] (FluxionJS-style path keys in `.fluxmat`).
    pub fn get_or_load_source(
        &mut self,
        device: &Device,
        queue:  &Queue,
        logical_path: &str,
        source: &dyn AssetSource,
    ) -> anyhow::Result<Arc<GpuTexture>> {
        if let Some(cached) = self.entries.get(logical_path) {
            return Ok(cached.clone());
        }

        let bytes = source
            .read(logical_path)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let label = std::path::Path::new(logical_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(logical_path);
        let texture = load_texture_memory(device, queue, label, &bytes)?;
        let arc = Arc::new(texture);
        self.entries.insert(logical_path.to_string(), arc.clone());
        Ok(arc)
    }

    /// Insert an already-created texture into the cache.
    pub fn insert(&mut self, path: &str, texture: GpuTexture) -> Arc<GpuTexture> {
        let arc = Arc::new(texture);
        self.entries.insert(path.to_string(), arc.clone());
        arc
    }

    /// Remove entries where only the cache itself holds a reference.
    /// Call this once per second or on scene unload.
    pub fn purge_unused(&mut self) {
        self.entries.retain(|_, arc| Arc::strong_count(arc) > 1);
    }

    pub fn get(&self, path: &str) -> Option<Arc<GpuTexture>> {
        self.entries.get(path).cloned()
    }
}

impl Default for TextureCache { fn default() -> Self { Self::new() } }

// ── File / memory loading ─────────────────────────────────────────────────────

/// Decode image bytes (PNG, JPEG, …) and upload as RGBA8 sRGB.
pub fn load_texture_memory(
    device: &Device,
    queue:  &Queue,
    label:  &str,
    bytes:  &[u8],
) -> anyhow::Result<GpuTexture> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| anyhow::anyhow!("Failed to decode texture '{label}': {e}"))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok(GpuTexture::from_rgba8(device, queue, label, w, h, &rgba))
}

fn load_texture_file(device: &Device, queue: &Queue, path: &str) -> anyhow::Result<GpuTexture> {
    let file_data = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("Failed to read texture '{}': {}", path, e))?;
    let label = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path);
    load_texture_memory(device, queue, label, &file_data)
}
