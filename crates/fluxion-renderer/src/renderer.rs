// ============================================================
// fluxion-renderer — FluxionRenderer
//
// Top-level renderer. Owns the wgpu device + queue + surface,
// manages the RenderGraph, and orchestrates each frame.
//
// Frame flow:
//   1. acquire surface texture
//   2. extract_frame_data()  — read ECS, build FrameData
//   3. render_graph.execute() — run all passes
//   4. submit + present
//
// WASM / native platform handling:
//   - wgpu handles the backend selection automatically
//   - Surface creation differs (winit window on native, HTMLCanvas on web)
//   - async init is driven by pollster on native, wasm-bindgen-futures on web
// ============================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use glam::Mat4;
use winit::window::Window;
use wgpu::SurfaceError;

use fluxion_core::{
    ECSWorld, EntityId,
    components::{Camera, Light, MeshRenderer},
    components::light::LightType,
    transform::Transform,
    time::Time,
};

use crate::{
    render_graph::{RenderGraph, PassSlot, RenderContext, RenderResources},
    render_graph::context::{FrameData, CameraData, MeshDrawCall},
    passes::{GeometryPass, LightingPass, SkyboxPass, BloomPass, SsaoPass, TonemapPass},
    lighting::{LightBuffer, LightBufferData, LightUniform, LIGHT_DIRECTIONAL, LIGHT_POINT, LIGHT_SPOT},
    material::MaterialRegistry,
    mesh::{GpuMesh, MeshRegistry},
    texture::TextureCache,
    shader::ShaderCache,
};

/// The main renderer.
///
/// # Initialization
/// ```rust
/// // Native:
/// let renderer = pollster::block_on(FluxionRenderer::new(Arc::new(window)))?;
/// // WASM:
/// let renderer = FluxionRenderer::new(Arc::new(window)).await?;
/// ```
pub struct FluxionRenderer {
    pub device:  wgpu::Device,
    pub queue:   wgpu::Queue,
    surface:     wgpu::Surface<'static>,
    config:      wgpu::SurfaceConfiguration,

    pub render_graph: RenderGraph,
    pub resources:    RenderResources,

    pub materials: MaterialRegistry,
    pub meshes:    MeshRegistry,
    pub textures:  TextureCache,
    pub shaders:   ShaderCache,

    /// The BindGroupLayout used by all PBR materials (group 2 in geometry pass).
    /// Stored here so `add_material()` can create new materials after init.
    pub mat_bgl: wgpu::BindGroupLayout,

    light_buffer: LightBuffer,

    pub width:  u32,
    pub height: u32,
}

impl FluxionRenderer {
    /// Create the renderer from a winit window.
    ///
    /// This is `async` because wgpu device creation is asynchronous.
    /// On native: wrap with `pollster::block_on(...)`.
    /// On WASM:   `await` inside an `async fn`.
    pub async fn new(window: Arc<Window>) -> anyhow::Result<Self> {
        let size = window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));

        // ── wgpu instance ─────────────────────────────────────────────────────
        // Backends::all() picks the best available backend for the platform:
        //   Windows: Vulkan > DX12 > DX11
        //   macOS:   Metal
        //   Web:     WebGPU > WebGL2
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // Safety: the surface must not outlive the window.
        // We use Arc<Window> so the window lives as long as the renderer.
        let surface = instance
            .create_surface(window.clone())
            .context("Failed to create wgpu surface")?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference:       wgpu::PowerPreference::HighPerformance,
                compatible_surface:     Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .context("No compatible GPU adapter found")?;

        log::info!("GPU: {:?} ({:?})", adapter.get_info().name, adapter.get_info().backend);

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label:             Some("fluxion_device"),
                    required_features: wgpu::Features::empty(),
                    required_limits:   wgpu::Limits::default(),
                    memory_hints:      Default::default(),
                },
                None,
            )
            .await
            .context("Failed to create wgpu device")?;

        // ── Surface configuration ─────────────────────────────────────────────
        let surface_caps   = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage:         wgpu::TextureUsages::RENDER_ATTACHMENT,
            format:        surface_format,
            width:         w,
            height:        h,
            present_mode:  wgpu::PresentMode::AutoVsync,
            alpha_mode:    surface_caps.alpha_modes[0],
            view_formats:  vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // ── GPU resources ─────────────────────────────────────────────────────
        let resources = RenderResources::new(&device, w, h);

        // Material registry needs a bind group layout matching geometry.frag.wgsl group(2).
        // We create a temporary material bgl here; the geometry pass also creates one.
        // In a full implementation, the bgl would be shared via a Rc/Arc.
        // For Phase 1 we create the default material directly.
        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding, visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let samp_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding, visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None,
        };
        let mat_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mat_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                tex_entry(1), samp_entry(2), tex_entry(3), samp_entry(4),
                tex_entry(5), samp_entry(6), tex_entry(7), samp_entry(8),
            ],
        });

        let mut textures  = TextureCache::new();
        let materials     = MaterialRegistry::new(&device, &queue, &mat_bgl);
        let meshes        = MeshRegistry::new(&device);
        let light_buffer  = LightBuffer::new(&device);

        // ── Render graph ──────────────────────────────────────────────────────
        let mut bloom = BloomPass::new();
        bloom.config.threshold  = 1.2;  // only very bright pixels (sun disc, emissives)
        bloom.config.strength   = 0.25;
        bloom.config.blur_passes = 4;

        let mut tonemap = TonemapPass::new(surface_format);
        tonemap.config.exposure           = 0.7;   // reduce from 1.0 — scene is very bright
        tonemap.config.vignette_intensity = 0.25;
        tonemap.config.film_grain         = 0.01;
        tonemap.config.chromatic_aberration = 0.3;

        let mut render_graph = RenderGraph::new();
        render_graph.add_pass("geometry",  PassSlot::Geometry,  Box::new(GeometryPass::new()));
        render_graph.add_pass("lighting",  PassSlot::Lighting,  Box::new(LightingPass::new()));
        render_graph.add_pass("skybox",    PassSlot::Skybox,    Box::new(SkyboxPass::new()));
        render_graph.add_pass("ssao",      PassSlot::Ssao,      Box::new(SsaoPass::new()));
        render_graph.add_pass("bloom",     PassSlot::Bloom,     Box::new(bloom));
        render_graph.add_pass("tonemap",   PassSlot::Tonemap,   Box::new(tonemap));
        render_graph.prepare(&device, &resources);

        Ok(Self {
            device, queue, surface, config,
            render_graph, resources,
            materials, meshes, textures, shaders: ShaderCache::new(),
            mat_bgl,
            light_buffer,
            width: w, height: h,
        })
    }

    // ── Public API ─────────────────────────────────────────────────────────────

    /// Create a material from a descriptor and register it. Returns the handle.
    pub fn add_material(&mut self, asset: &crate::material::MaterialAsset) -> anyhow::Result<u32> {
        let mat = crate::material::PbrMaterial::from_asset(
            &self.device, &self.queue, asset, &self.mat_bgl, &mut self.textures,
        )?;
        Ok(self.materials.add(mat))
    }

    /// Assign a material handle to a MeshRenderer entity in the world.
    pub fn set_entity_material(
        &self,
        world: &mut fluxion_core::ECSWorld,
        entity: fluxion_core::EntityId,
        handle: u32,
    ) {
        if let Some(mut mr) = world.get_component_mut::<fluxion_core::components::MeshRenderer>(entity) {
            mr.material_handle = Some(handle);
        }
    }

    /// Resolve `scene_inline_material` and on-disk `.fluxmat` paths into GPU materials.
    pub fn hydrate_scene_materials(
        &mut self,
        world: &mut ECSWorld,
        project_root: Option<&Path>,
    ) -> anyhow::Result<()> {
        let entities: Vec<EntityId> = world.all_entities().collect();
        for id in entities {
            let Some(mut mr) = world.get_component_mut::<MeshRenderer>(id) else {
                continue;
            };
            if mr.material_handle.is_some() {
                continue;
            }
            if let Some(v) = mr.scene_inline_material.take() {
                let name = format!("scene_inline_{:x}", id.to_bits());
                let asset =
                    crate::material::MaterialAsset::from_fluxionjs_mesh_material(&v, name);
                let h = self.add_material(&asset)?;
                mr.material_handle = Some(h);
                continue;
            }
            if let Some(rel) = mr.material_path.clone() {
                let p: PathBuf = project_root.map(|r| r.join(&rel)).unwrap_or_else(|| PathBuf::from(&rel));
                if p.is_file() {
                    let p_str = p.to_str().context("material path is not valid UTF-8")?;
                    let asset = crate::material::MaterialAsset::load_from_file(p_str)?;
                    let h = self.add_material(&asset)?;
                    mr.material_handle = Some(h);
                }
            }
        }
        Ok(())
    }

    /// Upload `.glb` / `.gltf` meshes referenced by `MeshRenderer.mesh_path` (relative to `base`).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn hydrate_mesh_paths(
        &mut self,
        world: &mut ECSWorld,
        base: Option<&Path>,
    ) -> anyhow::Result<()> {
        use fluxion_core::components::MeshRenderer;

        let entities: Vec<EntityId> = world.all_entities().collect();
        for id in entities {
            let Some(mut mr) = world.get_component_mut::<MeshRenderer>(id) else {
                continue;
            };
            if mr.mesh_handle.is_some() {
                continue;
            }
            let Some(ref rel) = mr.mesh_path else {
                continue;
            };
            let ext = Path::new(rel)
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_ascii_lowercase());
            if !matches!(ext.as_deref(), Some("glb") | Some("gltf")) {
                continue;
            }

            let path: PathBuf = base.map(|b| b.join(rel)).unwrap_or_else(|| PathBuf::from(rel));
            if !path.is_file() {
                log::warn!("[gltf] file not found: {}", path.display());
                continue;
            }

            let (vertices, indices) = crate::mesh::gltf_loader::load_gltf_path(&path)?;
            let n_verts = vertices.len();
            let label = path.file_name().and_then(|s| s.to_str()).unwrap_or("gltf");
            let gpu = GpuMesh::upload(&self.device, label, &vertices, &indices);
            let handle = self.meshes.add(gpu);
            mr.mesh_handle = Some(handle);
            mr.primitive = None;
            log::info!("[gltf] loaded {} ({n_verts} vertices)", path.display());
        }
        Ok(())
    }

    /// WASM: call after fetching bytes (e.g. `fetch` + `.bytes()`), same semantics as [`hydrate_mesh_paths`](Self::hydrate_mesh_paths).
    #[cfg(target_arch = "wasm32")]
    pub fn hydrate_mesh_paths_from_memory(
        &mut self,
        world: &mut ECSWorld,
        resolve: impl Fn(&str) -> Option<Vec<u8>>,
    ) -> anyhow::Result<()> {
        use fluxion_core::components::MeshRenderer;

        let entities: Vec<EntityId> = world.all_entities().collect();
        for id in entities {
            let Some(mut mr) = world.get_component_mut::<MeshRenderer>(id) else {
                continue;
            };
            if mr.mesh_handle.is_some() {
                continue;
            }
            let Some(ref rel) = mr.mesh_path else {
                continue;
            };
            let ext = Path::new(rel)
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_ascii_lowercase());
            if !matches!(ext.as_deref(), Some("glb") | Some("gltf")) {
                continue;
            }
            let Some(bytes) = resolve(rel) else {
                log::warn!("[gltf] no bytes for {rel}");
                continue;
            };
            let (vertices, indices) = crate::mesh::gltf_loader::load_gltf_slice(&bytes)?;
            let label = Path::new(rel).file_name().and_then(|s| s.to_str()).unwrap_or("gltf");
            let gpu = GpuMesh::upload(&self.device, label, &vertices, &indices);
            let handle = self.meshes.add(gpu);
            mr.mesh_handle = Some(handle);
            mr.primitive = None;
        }
        Ok(())
    }

    /// Call when the window is resized.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 { return; }
        self.width  = width;
        self.height = height;
        self.config.width  = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.resources.resize(&self.device, width, height);
        self.render_graph.resize(&self.device, width, height);
    }

    /// Render one frame from the current ECS world state.
    ///
    /// Returns `Err(SurfaceError::Outdated)` if the window was resized between
    /// this call and the last `resize()` — just call `resize()` and retry.
    pub fn render(&mut self, world: &ECSWorld, time: &Time) -> Result<(), SurfaceError> {
        // ── Acquire surface texture ───────────────────────────────────────────
        let surface_texture = self.surface.get_current_texture()?;
        let surface_view    = surface_texture.texture.create_view(&Default::default());

        // ── Extract scene data from ECS ───────────────────────────────────────
        let frame = self.extract_frame_data(world, time);

        // ── Upload light buffer to GPU ────────────────────────────────────────
        let mut light_data = LightBufferData::new();
        for light in &frame.lights {
            light_data.push(*light);
        }
        self.light_buffer.upload(&self.queue, &light_data);

        // ── Record commands ───────────────────────────────────────────────────
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame_encoder"),
        });

        let mut ctx = RenderContext {
            device:       &self.device,
            queue:        &self.queue,
            encoder:      &mut encoder,
            resources:    &self.resources,
            frame:        &frame,
            surface_view: &surface_view,
            light_buffer: &self.light_buffer.gpu_buffer,
            meshes:       &self.meshes,
            materials:    &self.materials,
        };

        self.render_graph.execute(&mut ctx);

        // ── Submit ────────────────────────────────────────────────────────────
        self.queue.submit(std::iter::once(encoder.finish()));
        surface_texture.present();
        Ok(())
    }

    // ── Private: ECS → FrameData ──────────────────────────────────────────────

    fn extract_frame_data(&mut self, world: &ECSWorld, time: &Time) -> FrameData {
        // ── Camera ────────────────────────────────────────────────────────────
        let camera = self.extract_camera(world);

        // ── Mesh draw calls ───────────────────────────────────────────────────
        let mut draw_calls: Vec<MeshDrawCall> = Vec::new();
        let default_mat = self.materials.default_handle();
        let meshes      = &self.meshes;
        world.query_active::<(&Transform, &MeshRenderer), _>(|_id, (transform, mesh_renderer)| {
            let mesh_handle = match mesh_renderer.mesh_handle {
                Some(h) => h,
                None => {
                    if let Some(prim) = mesh_renderer.primitive {
                        meshes.primitive_handle(prim)
                    } else {
                        return; // no mesh, skip
                    }
                }
            };

            let mat_handle  = mesh_renderer.material_handle.unwrap_or(default_mat);
            let world_mat   = transform.world_matrix;
            let normal_mat  = world_mat.inverse().transpose();

            draw_calls.push(MeshDrawCall {
                mesh:          mesh_handle,
                material:      mat_handle,
                world_matrix:  world_mat,
                normal_matrix: normal_mat,
                cast_shadow:   mesh_renderer.cast_shadow,
                layer:         mesh_renderer.layer,
            });
        });

        // ── Lights ────────────────────────────────────────────────────────────
        let mut lights: Vec<LightUniform> = Vec::new();
        world.query_active::<(&Transform, &Light), _>(|_id, (transform, light)| {
            let light_type = match light.light_type {
                LightType::Directional => LIGHT_DIRECTIONAL,
                LightType::Point       => LIGHT_POINT,
                LightType::Spot        => LIGHT_SPOT,
            };

            let outer_cos = (light.spot_angle.to_radians() * 0.5).cos();
            let inner_cos = ((light.spot_angle * (1.0 - light.spot_penumbra)).to_radians() * 0.5).cos();

            lights.push(LightUniform {
                position:   transform.world_position.to_array(),
                light_type,
                direction:  transform.world_forward().to_array(),
                range:      light.range,
                color:      light.color,
                intensity:  light.intensity,
                spot_angle: outer_cos,
                spot_inner: inner_cos,
                _pad0:      0.0, _pad1: 0.0,
            });
        });

        FrameData {
            camera,
            draw_calls,
            lights,
            viewport: (self.width, self.height),
            time: time.elapsed,
        }
    }

    fn extract_camera(&self, world: &ECSWorld) -> CameraData {
        let mut result: Option<CameraData> = None;
        let (w, h) = (self.width, self.height);

        world.query_active::<(&Transform, &Camera), _>(|_id, (transform, camera)| {
            if result.is_some() || !camera.is_active { return; }

            let view = Mat4::look_at_rh(
                transform.world_position,
                transform.world_position + transform.world_forward(),
                transform.world_up(),
            );
            let proj        = camera.projection_matrix(w, h);
            let view_proj   = proj * view;
            let inv_vp      = view_proj.inverse();
            let inv_proj    = proj.inverse();

            result = Some(CameraData {
                view,
                projection:    proj,
                view_proj,
                inv_view_proj: inv_vp,
                inv_proj,
                position:      transform.world_position,
                near:          camera.near,
                far:           camera.far,
            });
        });

        result.unwrap_or_else(|| {
            log::warn!("No active camera found in the scene");
            CameraData::identity()
        })
    }
}
