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

use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use glam::{Mat4, Vec3};
use winit::window::Window;
use wgpu::SurfaceError;

use fluxion_core::{
    ECSWorld, EntityId,
    Color,
    assets,
    components::{Camera, Light, MeshRenderer, ParticleEmitter, RigidBody, PhysicsShape},
    components::light::LightType,
    components::camera::ProjectionMode,
    scene::SceneSettings,
    transform::Transform,
    time::Time,
    debug_draw,
};

use crate::{
    config::RendererConfig,
    render_graph::{RenderGraph, PassSlot, RenderContext, RenderResources},
    render_graph::context::{FrameData, CameraData, MeshDrawCall, SkyParams, ParticleInstance},
    passes::{GeometryPass, LightingPass, SkyboxPass, BloomPass, SsaoPass, TonemapPass, ParticleOverlayPass, DebugLinePass, ShadowPass},
    lighting::{LightBuffer, LightBufferData, LightUniform, LIGHT_DIRECTIONAL, LIGHT_POINT, LIGHT_SPOT, MAX_LIGHTS},
    material::MaterialRegistry,
    mesh::{GpuMesh, MeshRegistry},
    texture::{GpuTexture, TextureCache},
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
    surface:        wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,

    pub render_graph: RenderGraph,
    pub resources:    RenderResources,

    pub materials: MaterialRegistry,
    pub meshes:    MeshRegistry,
    pub textures:  TextureCache,
    pub shaders:   ShaderCache,

    /// When set, [`Self::add_material`] resolves texture paths through this source (FluxionJS-style relative paths).
    pub asset_source: Option<Arc<dyn assets::AssetSource>>,

    /// From loaded `.scene` ([`SceneSettings`]); drives ambient, fog, sky tint, and [`Self::physics_gravity`].
    pub scene_settings: SceneSettings,

    /// Reserved: cubemap / equirect path from `scene_settings.skybox` once texture sky is implemented.
    pub skybox_asset_path: Option<String>,

    /// The BindGroupLayout used by all PBR materials (group 2 in geometry pass).
    /// Stored here so `add_material()` can create new materials after init.
    pub mat_bgl: wgpu::BindGroupLayout,

    light_buffer: LightBuffer,

    /// Active renderer configuration. Can be mutated at runtime.
    /// Call `apply_config()` after changing to push updates to passes.
    pub config: RendererConfig,

    /// Runtime max lights (clamped to MAX_LIGHTS).
    max_lights: usize,

    pub width:  u32,
    pub height: u32,

    /// When `true`, component gizmos (camera frustum, collider wireframe, light, particles)
    /// and the editor grid are drawn as a debug overlay every frame.
    pub gizmos_enabled: bool,

    /// Offscreen render target for the editor viewport panel.
    /// Recreated automatically when the window is resized.
    pub viewport_texture: Option<GpuTexture>,

    /// Last frame's camera matrices — cached for use by editor gizmos.
    pub last_view_matrix: glam::Mat4,
    pub last_proj_matrix: glam::Mat4,
}

impl FluxionRenderer {
    /// Create the renderer from a winit window.
    ///
    /// This is `async` because wgpu device creation is asynchronous.
    /// On native: wrap with `pollster::block_on(...)`.
    /// On WASM:   `await` inside an `async fn`.
    pub async fn new(window: Arc<Window>, config: RendererConfig) -> anyhow::Result<Self> {
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

        let surface_config = wgpu::SurfaceConfiguration {
            usage:         wgpu::TextureUsages::RENDER_ATTACHMENT,
            format:        surface_format,
            width:         w,
            height:        h,
            present_mode:  wgpu::PresentMode::AutoVsync,
            alpha_mode:    surface_caps.alpha_modes[0],
            view_formats:  vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

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

        // ── Render graph — built from config ─────────────────────────────────
        let mut bloom = BloomPass::new();
        bloom.config.enabled     = config.bloom.enabled;
        bloom.config.threshold   = config.bloom.threshold;
        bloom.config.soft_knee   = config.bloom.soft_knee;
        bloom.config.strength    = config.bloom.strength;
        bloom.config.blur_passes = config.bloom.blur_passes;

        let mut tonemap = TonemapPass::new(surface_format);
        tonemap.config.exposure             = config.tonemap.exposure;
        tonemap.config.vignette_intensity   = config.tonemap.vignette_intensity;
        tonemap.config.vignette_roundness   = config.tonemap.vignette_roundness;
        tonemap.config.chromatic_aberration = config.tonemap.chromatic_aberration;
        tonemap.config.film_grain           = config.tonemap.film_grain;

        let mut ssao = SsaoPass::new();
        ssao.enabled   = config.ssao.enabled;
        ssao.radius    = config.ssao.radius;
        ssao.bias      = config.ssao.bias;
        ssao.intensity = config.ssao.intensity;

        let max_lights = config.max_lights.min(MAX_LIGHTS);

        let mut render_graph = RenderGraph::new();
        render_graph.add_pass("shadow",    PassSlot::Shadow,   Box::new(ShadowPass::new()));
        render_graph.add_pass("geometry",  PassSlot::Geometry, Box::new(GeometryPass::new()));
        render_graph.add_pass("lighting",  PassSlot::Lighting, Box::new(LightingPass::new()));
        render_graph.add_pass("skybox",    PassSlot::Skybox,   Box::new(SkyboxPass::new()));
        render_graph.add_pass("ssao",      PassSlot::Ssao,     Box::new(ssao));
        render_graph.add_pass("bloom",     PassSlot::Bloom,    Box::new(bloom));
        render_graph.add_pass("tonemap",   PassSlot::Tonemap,  Box::new(tonemap));
        render_graph.add_pass("particles",   PassSlot::Overlay,  Box::new(ParticleOverlayPass::new(surface_format)));
        render_graph.add_pass("debug_lines", PassSlot::Overlay,  Box::new(DebugLinePass::new(surface_format)));

        // Apply per-pass enable flags from config.
        render_graph.set_enabled("skybox",    config.passes.skybox);
        render_graph.set_enabled("ssao",      config.passes.ssao);
        render_graph.set_enabled("bloom",     config.passes.bloom);
        render_graph.set_enabled("particles", config.passes.particles);

        render_graph.prepare(&device, &resources);

        Ok(Self {
            device, queue, surface,
            surface_config,
            render_graph, resources,
            materials, meshes, textures, shaders: ShaderCache::new(),
            mat_bgl,
            light_buffer,
            asset_source: None,
            scene_settings: SceneSettings::default(),
            skybox_asset_path: None,
            config,
            max_lights,
            width: w, height: h,
            gizmos_enabled: false,
            viewport_texture: None,
            last_view_matrix: glam::Mat4::IDENTITY,
            last_proj_matrix: glam::Mat4::IDENTITY,
        })
    }

    // ── Public API ─────────────────────────────────────────────────────────────

    /// Set the asset root for texture loads in [`Self::add_material`] (and optional WASM scene materials).
    pub fn set_asset_source(&mut self, src: Option<Arc<dyn assets::AssetSource>>) {
        self.asset_source = src;
    }

    /// Apply a new [`RendererConfig`] at runtime.
    ///
    /// Updates all pass configs and enable flags without recreating GPU resources.
    /// Call this after loading or hot-reloading `renderer.config.json`.
    pub fn apply_config(&mut self, config: RendererConfig) {
        use crate::passes::{BloomPass, TonemapPass, SsaoPass};

        self.max_lights = config.max_lights.min(MAX_LIGHTS);

        if let Some(bloom) = self.render_graph.get_pass_mut::<BloomPass>("bloom") {
            bloom.config.enabled     = config.bloom.enabled;
            bloom.config.threshold   = config.bloom.threshold;
            bloom.config.soft_knee   = config.bloom.soft_knee;
            bloom.config.strength    = config.bloom.strength;
            bloom.config.blur_passes = config.bloom.blur_passes;
        }
        if let Some(tonemap) = self.render_graph.get_pass_mut::<TonemapPass>("tonemap") {
            tonemap.config.exposure             = config.tonemap.exposure;
            tonemap.config.vignette_intensity   = config.tonemap.vignette_intensity;
            tonemap.config.vignette_roundness   = config.tonemap.vignette_roundness;
            tonemap.config.chromatic_aberration = config.tonemap.chromatic_aberration;
            tonemap.config.film_grain           = config.tonemap.film_grain;
        }
        if let Some(ssao) = self.render_graph.get_pass_mut::<SsaoPass>("ssao") {
            ssao.enabled   = config.ssao.enabled;
            ssao.radius    = config.ssao.radius;
            ssao.bias      = config.ssao.bias;
            ssao.intensity = config.ssao.intensity;
        }

        self.render_graph.set_enabled("skybox",    config.passes.skybox);
        self.render_graph.set_enabled("ssao",      config.passes.ssao);
        self.render_graph.set_enabled("bloom",     config.passes.bloom);
        self.render_graph.set_enabled("particles", config.passes.particles);

        self.config = config;
    }

    /// Apply global scene file settings (ambient, fog, gravity vector, skybox path placeholder).
    pub fn apply_scene_settings(&mut self, settings: SceneSettings) {
        self.skybox_asset_path = settings.skybox.clone();
        self.scene_settings = settings;
    }

    /// Scene gravity from the last applied [`SceneSettings`] (for physics integration).
    pub fn physics_gravity(&self) -> Vec3 {
        Vec3::from_array(self.scene_settings.physics_gravity)
    }

    /// Compile external `.wgsl` from an [`assets::AssetSource`] (custom materials / hot reload).
    pub fn load_shader_module_from_source(
        &mut self,
        source: &dyn assets::AssetSource,
        wgsl_path: &str,
        module_name: &str,
    ) -> anyhow::Result<&wgpu::ShaderModule> {
        let text = assets::read_text(source, wgsl_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(self.shaders.get_or_compile(&self.device, module_name, &text))
    }

    /// Create a material from a descriptor and register it. Returns the handle.
    pub fn add_material(&mut self, asset: &crate::material::MaterialAsset) -> anyhow::Result<u32> {
        let src = self.asset_source.as_deref();
        let mat = crate::material::PbrMaterial::from_asset(
            &self.device,
            &self.queue,
            asset,
            &self.mat_bgl,
            &mut self.textures,
            src,
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

    /// Resolve `scene_inline_material` and `.fluxmat` paths into GPU materials (FluxionJS parity).
    ///
    /// Native: reads under `project_root` (or current directory). WASM: uses [`Self::asset_source`]
    /// if set; otherwise skips file-backed materials.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn hydrate_scene_materials(
        &mut self,
        world: &mut ECSWorld,
        project_root: Option<&Path>,
    ) -> anyhow::Result<()> {
        let root = project_root
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let disk = assets::DiskAssetSource::new(root);
        self.hydrate_scene_materials_from_source(world, &disk, None)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn hydrate_scene_materials(
        &mut self,
        world: &mut ECSWorld,
        _project_root: Option<&Path>,
    ) -> anyhow::Result<()> {
        let Some(src) = self.asset_source.clone() else {
            return Ok(());
        };
        self.hydrate_scene_materials_from_source(world, src.as_ref(), None)
    }

    /// Same as [`Self::hydrate_scene_materials`] but uses any [`assets::AssetSource`] (disk, memory, fetch).
    pub fn hydrate_scene_materials_from_source(
        &mut self,
        world: &mut ECSWorld,
        source: &dyn assets::AssetSource,
        base: Option<&str>,
    ) -> anyhow::Result<()> {
        let entities: Vec<EntityId> = world.all_entities().collect();
        for id in entities {
            let Some(mut mr) = world.get_component_mut::<MeshRenderer>(id) else {
                continue;
            };
            if let Some(v) = mr.scene_inline_material.take() {
                let name = format!("scene_inline_{:x}", id.to_bits());
                let asset =
                    crate::material::MaterialAsset::from_fluxionjs_mesh_material(&v, name);
                let h = self.add_material(&asset)?;
                mr.material_handle = Some(h);
                drop(mr);
                Self::propagate_scene_material_to_gltf_children(world, id, h);
                continue;
            }
            if let Some(rel) = mr.material_path.clone() {
                let logical = assets::join_logical(base, &rel);
                if !source.exists(&logical) && !source.exists(&rel) {
                    continue;
                }
                let bytes = source
                    .read(&logical)
                    .or_else(|_| source.read(&rel))
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                let asset = crate::material::MaterialAsset::from_json_bytes(&bytes, &logical)?;
                let h = self.add_material(&asset)?;
                mr.material_handle = Some(h);
                drop(mr);
                Self::propagate_scene_material_to_gltf_children(world, id, h);
            }
        }
        Ok(())
    }

    /// Child entities created for extra glTF primitives: mesh handle set, no asset path.
    fn propagate_scene_material_to_gltf_children(world: &mut ECSWorld, parent: EntityId, handle: u32) {
        for child in world.get_children(parent) {
            let Some(mut mr) = world.get_component_mut::<MeshRenderer>(child) else {
                continue;
            };
            if mr.mesh_path.is_none() && mr.mesh_handle.is_some() {
                mr.material_handle = Some(handle);
            }
        }
    }

    /// `true` if scene inline / resolvable `.fluxmat` should replace per-primitive glTF materials.
    fn scene_material_overrides_gltf(
        mr: &MeshRenderer,
        source: &dyn assets::AssetSource,
        base: Option<&str>,
    ) -> bool {
        if mr.scene_inline_material.is_some() {
            return true;
        }
        let Some(rel) = mr.material_path.as_ref() else {
            return false;
        };
        let full = assets::join_logical(base, rel);
        source.exists(&full) || source.exists(rel)
    }

    fn upload_gltf_textures(&mut self, uploads: &[crate::mesh::gltf_loader::GltfTextureUpload]) {
        for u in uploads {
            if self.textures.get(&u.key).is_some() {
                continue;
            }
            let (w, h) = u.rgba.dimensions();
            let tex = GpuTexture::from_rgba8(
                &self.device,
                &self.queue,
                &u.key,
                w,
                h,
                u.rgba.as_raw(),
            );
            self.textures.insert(&u.key, tex);
        }
    }

    fn apply_gltf_load_output(
        &mut self,
        world: &mut ECSWorld,
        root: EntityId,
        out: crate::mesh::gltf_loader::GltfLoadOutput,
        label: &str,
        skip_gltf_materials: bool,
    ) -> anyhow::Result<()> {
        self.upload_gltf_textures(&out.textures);

        let mut mat_handles: Vec<u32> = Vec::with_capacity(out.materials.len());
        if !skip_gltf_materials {
            for asset in &out.materials {
                mat_handles.push(self.add_material(asset)?);
            }
        }

        let Some(mr) = world.get_component_mut::<MeshRenderer>(root) else {
            anyhow::bail!("[gltf] entity has no MeshRenderer");
        };
        let cast_shadow = mr.cast_shadow;
        let receive_shadow = mr.receive_shadow;
        let layer = mr.layer;
        drop(mr);

        let n_mat = mat_handles.len();
        let mat_idx = |i: usize| -> Option<u32> {
            if skip_gltf_materials || n_mat == 0 {
                return None;
            }
            Some(mat_handles[i.min(n_mat.saturating_sub(1))])
        };

        // Root entity keeps scene placement; geometry lives under a glTF node hierarchy.
        {
            let mut mr = world.get_component_mut::<MeshRenderer>(root).unwrap();
            mr.mesh_handle = None;
            mr.material_handle = None;
            mr.primitive = None;
        }

        let mut entity_by_idx: Vec<EntityId> = Vec::with_capacity(out.nodes.len());
        let mut prim_count = 0usize;
        let mut vert_count = 0usize;

        for (ni, node) in out.nodes.iter().enumerate() {
            let parent_entity = node
                .parent_idx
                .and_then(|p| entity_by_idx.get(p).copied())
                .unwrap_or(root);

            let e = world.spawn(Some(node.name.as_str()));
            let mut t = Transform::new();
            t.position = node.position;
            t.rotation = node.rotation;
            t.scale = node.scale;
            t.dirty = true;
            t.world_dirty = true;
            world.add_component(e, t);
            world.set_parent(e, Some(parent_entity), false);
            entity_by_idx.push(e);

            for (pi, prim) in node.mesh_primitives.iter().enumerate() {
                let child_name = format!("{}_p{pi}_{label}", node.name);
                let child = world.spawn(Some(child_name.as_str()));
                world.add_component(child, Transform::new());
                world.set_parent(child, Some(e), false);
                let sub_label = format!("{label}_n{ni}_p{pi}");
                let gpu = GpuMesh::upload(&self.device, &sub_label, &prim.vertices, &prim.indices);
                let hm = self.meshes.add(gpu);
                world.add_component(
                    child,
                    MeshRenderer {
                        mesh_path: None,
                        material_path: None,
                        primitive: None,
                        cast_shadow,
                        receive_shadow,
                        layer,
                        mesh_handle: Some(hm),
                        material_handle: mat_idx(prim.material_index),
                        scene_inline_material: None,
                    },
                );
                prim_count += 1;
                vert_count += prim.vertices.len();
            }
        }

        log::info!(
            "[gltf] {label}: {} nodes, {prim_count} primitives, {vert_count} vertices, {} materials",
            out.nodes.len(),
            out.materials.len()
        );
        Ok(())
    }

    /// Native: load glTF paths relative to `base` (or current working directory).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn hydrate_mesh_paths(
        &mut self,
        world: &mut ECSWorld,
        base: Option<&Path>,
    ) -> anyhow::Result<()> {
        let root = base
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let disk = assets::DiskAssetSource::new(root);
        self.hydrate_mesh_paths_from_source(world, &disk, None)
    }

    /// Upload `.glb` / `.gltf` meshes using a FluxionJS-style logical path and any [`assets::AssetSource`].
    pub fn hydrate_mesh_paths_from_source(
        &mut self,
        world: &mut ECSWorld,
        source: &dyn assets::AssetSource,
        base: Option<&str>,
    ) -> anyhow::Result<()> {
        use fluxion_core::components::MeshRenderer;

        let entities: Vec<EntityId> = world.all_entities().collect();
        for id in entities {
            let Some(mr) = world.get_component_mut::<MeshRenderer>(id) else {
                continue;
            };
            if mr.mesh_handle.is_some() {
                continue;
            }
            let Some(rel_owned) = mr.mesh_path.clone() else {
                continue;
            };
            let ext = Path::new(&rel_owned)
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_ascii_lowercase());
            if !matches!(ext.as_deref(), Some("glb") | Some("gltf")) {
                continue;
            }

            let logical = assets::join_logical(base, &rel_owned);
            let rel_lower = rel_owned.to_ascii_lowercase();

            let skip_mat = Self::scene_material_overrides_gltf(&mr, source, base);
            drop(mr);

            let out = if rel_lower.ends_with(".gltf") {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    if let Some(root) = source.native_project_root() {
                        let path = assets::resolve_under_root(root, &logical)
                            .map_err(|e| anyhow::anyhow!("{e}"))?;
                        crate::mesh::gltf_loader::load_gltf_path_full(&path)?
                    } else {
                        Self::load_gltf_bytes_with_uri_resolver(source, &logical, &rel_owned)?
                    }
                }
                #[cfg(target_arch = "wasm32")]
                {
                    Self::load_gltf_bytes_with_uri_resolver(source, &logical, &rel_owned)?
                }
            } else {
                let bytes = source
                    .read(&logical)
                    .or_else(|_| source.read(&rel_owned))
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                crate::mesh::gltf_loader::load_gltf_slice_full(&bytes)?
            };

            let label = Path::new(&rel_owned)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("gltf");
            self.apply_gltf_load_output(world, id, out, label, skip_mat)?;
        }
        Ok(())
    }

    /// WASM helper: `resolve` returns bytes per logical path (same as [`assets::FnAssetSource`]).
    #[cfg(target_arch = "wasm32")]
    pub fn hydrate_mesh_paths_from_memory(
        &mut self,
        world: &mut ECSWorld,
        resolve: impl Fn(&str) -> Option<Vec<u8>> + Send + Sync + 'static,
    ) -> anyhow::Result<()> {
        let src = assets::FnAssetSource::new(move |p| {
            resolve(p).ok_or_else(|| assets::AssetError::NotFound(p.to_string()))
        });
        self.hydrate_mesh_paths_from_source(world, &src, None)
    }

    fn load_gltf_bytes_with_uri_resolver(
        source: &dyn assets::AssetSource,
        logical: &str,
        rel_owned: &str,
    ) -> anyhow::Result<crate::mesh::gltf_loader::GltfLoadOutput> {
        let bytes = source
            .read(logical)
            .or_else(|_| source.read(rel_owned))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let dir_prefix = Path::new(logical)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("");
        let mut resolve_uri = |uri: &str| -> anyhow::Result<Vec<u8>> {
            let u = uri.replace('\\', "/");
            let key = if dir_prefix.is_empty() {
                u.clone()
            } else {
                format!("{dir_prefix}/{u}")
            };
            source
                .read(&key)
                .or_else(|_| source.read(&u))
                .map_err(|e| anyhow::anyhow!("{e}"))
        };
        crate::mesh::gltf_loader::load_gltf_slice_full_with_resolver(&bytes, &mut resolve_uri)
    }

    /// Call when the window is resized.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 { return; }
        self.width  = width;
        self.height = height;
        self.surface_config.width  = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
        self.resources.resize(&self.device, width, height);
        self.render_graph.resize(&self.device, width, height);
    }

    /// Swapchain format (sRGB), for UI backends (`egui-wgpu`, etc.).
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_config.format
    }

    /// Render the 3-D scene to an offscreen viewport texture (same size as the window).
    ///
    /// The result is stored in `self.viewport_texture` and can be retrieved via
    /// [`Self::viewport_view`].  Call this before [`Self::render_ui_only`] when
    /// using the editor viewport panel.
    pub fn render_to_viewport(&mut self, world: &ECSWorld, time: &Time) -> anyhow::Result<()> {
        if self.width == 0 || self.height == 0 { return Ok(()); }
        let w      = self.width;
        let h      = self.height;
        let format = self.surface_config.format;

        let needs_recreate = self.viewport_texture.as_ref()
            .map(|t| t.width != w || t.height != h || t.format != format)
            .unwrap_or(true);
        if needs_recreate {
            self.viewport_texture = Some(GpuTexture::render_target(
                &self.device, "viewport_color", w, h, format,
            ));
        }

        let frame = self.extract_frame_data(world, time);

        let mut light_data = LightBufferData::new();
        for light in &frame.lights { light_data.push(*light); }
        let s = &self.scene_settings;
        light_data.ambient_color     = s.ambient_color;
        light_data.ambient_intensity = s.ambient_intensity;
        light_data.fog_color         = s.fog_color;
        light_data.fog_density       = s.fog_density;
        light_data.fog_enabled       = u32::from(s.fog_enabled);
        light_data._fog_pad          = [0; 3];
        self.light_buffer.upload(&self.queue, &light_data);

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewport_encoder"),
        });

        // Safety: viewport_texture is not dropped or reallocated while `encoder` is
        // being recorded.  render_graph.execute() only accesses the fields of ctx,
        // none of which include `viewport_texture` itself.
        let vp_view: *const wgpu::TextureView =
            &self.viewport_texture.as_ref().unwrap().view;
        let mut ctx = RenderContext {
            device:       &self.device,
            queue:        &self.queue,
            encoder:      &mut encoder,
            resources:    &self.resources,
            frame:        &frame,
            surface_view: unsafe { &*vp_view },
            light_buffer: &self.light_buffer.gpu_buffer,
            meshes:       &self.meshes,
            materials:    &self.materials,
        };
        self.render_graph.execute(&mut ctx);

        self.queue.submit(std::iter::once(encoder.finish()));
        Ok(())
    }

    /// View into the last viewport render target, or `None` if `render_to_viewport`
    /// has not been called yet.
    pub fn viewport_view(&self) -> Option<&wgpu::TextureView> {
        self.viewport_texture.as_ref().map(|t| &t.view)
    }

    /// Acquire the swap-chain surface, clear it to a neutral colour, then call
    /// `after` for UI-only work (egui).  Does **not** run the 3-D render graph.
    /// Use after [`Self::render_to_viewport`] in editor mode.
    pub fn render_ui_only(
        &mut self,
        after: impl FnOnce(&wgpu::Device, &wgpu::Queue, &mut wgpu::CommandEncoder, &wgpu::TextureView) -> Vec<wgpu::CommandBuffer>,
    ) -> Result<(), SurfaceError> {
        let surface_texture = self.surface.get_current_texture()?;
        let surface_view    = surface_texture.texture.create_view(&Default::default());

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ui_encoder"),
        });
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui_clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           &surface_view,
                    resolve_target: None,
                    ops:            wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(wgpu::Color { r: 0.07, g: 0.07, b: 0.07, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes:         None,
                occlusion_query_set:      None,
            });
        }

        let user_bufs = after(&self.device, &self.queue, &mut encoder, &surface_view);
        self.queue.submit(user_bufs.into_iter().chain(std::iter::once(encoder.finish())));
        surface_texture.present();
        Ok(())
    }

    /// Render one frame from the current ECS world state.
    ///
    /// Returns `Err(SurfaceError::Outdated)` if the window was resized between
    /// this call and the last `resize()` — just call `resize()` and retry.
    pub fn render(&mut self, world: &ECSWorld, time: &Time) -> Result<(), SurfaceError> {
        self.render_with(world, time, |_, _, _, _| Vec::new())
    }

    /// Like [`Self::render`], then run `after` on the same encoder before submit (e.g. egui paint).
    /// Return extra command buffers to submit before the main frame encoder (egui-wgpu callbacks).
    pub fn render_with(
        &mut self,
        world: &ECSWorld,
        time: &Time,
        after: impl FnOnce(&wgpu::Device, &wgpu::Queue, &mut wgpu::CommandEncoder, &wgpu::TextureView) -> Vec<wgpu::CommandBuffer>,
    ) -> Result<(), SurfaceError> {
        let surface_texture = self.surface.get_current_texture()?;
        let surface_view    = surface_texture.texture.create_view(&Default::default());

        let frame = self.extract_frame_data(world, time);

        let mut light_data = LightBufferData::new();
        for light in &frame.lights {
            light_data.push(*light);
        }
        let s = &self.scene_settings;
        light_data.ambient_color = s.ambient_color;
        light_data.ambient_intensity = s.ambient_intensity;
        light_data.fog_color = s.fog_color;
        light_data.fog_density = s.fog_density;
        light_data.fog_enabled = u32::from(s.fog_enabled);
        light_data._fog_pad = [0; 3];
        self.light_buffer.upload(&self.queue, &light_data);

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

        let user_bufs = after(&self.device, &self.queue, &mut encoder, &surface_view);

        self.queue
            .submit(user_bufs.into_iter().chain(std::iter::once(encoder.finish())));
        surface_texture.present();
        Ok(())
    }

    // ── Private: ECS → FrameData ──────────────────────────────────────────────

    fn extract_frame_data(&mut self, world: &ECSWorld, time: &Time) -> FrameData {
        // ── Camera ────────────────────────────────────────────────────────────
        let camera = self.extract_camera(world);
        // Cache for editor gizmo overlay.
        self.last_view_matrix = camera.view;
        self.last_proj_matrix = camera.projection;

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

        let sky = Self::compute_sky_params(&self.scene_settings, &lights);

        let mut particles: Vec<ParticleInstance> = Vec::new();
        const MAX_DRAW: usize = 4096;
        world.query_active::<&ParticleEmitter, _>(|_, emitter| {
            for p in &emitter.particles {
                if particles.len() >= MAX_DRAW {
                    return;
                }
                particles.push(ParticleInstance {
                    position: p.position.to_array(),
                    size:     p.size,
                    color:    p.color,
                });
            }
        });

        if self.gizmos_enabled {
            self.populate_gizmos(world);
        }
        let debug_lines = debug_draw::drain_debug_lines();

        // ── Shadow view-projection (first directional cast_shadow light) ──────
        let mut shadow_view_proj  = Mat4::IDENTITY;
        let mut has_shadow_caster = false;
        'shadow: for light in &lights {
            if light.light_type == LIGHT_DIRECTIONAL {
                // Only compute for the first directional light (shadow index 0).
                let dir = glam::Vec3::from_array(light.direction);
                // Orthographic frustum: 50m half-size, depth range 0..500m.
                let half  = 50.0f32;
                let depth = 500.0f32;
                // Light view: look from above along the light direction.
                let eye    = -dir * (depth * 0.5);
                let target = glam::Vec3::ZERO;
                let up     = if dir.y.abs() > 0.99 { glam::Vec3::X } else { glam::Vec3::Y };
                let light_view = Mat4::look_at_rh(eye, target, up);
                let light_proj = Mat4::orthographic_rh(-half, half, -half, half, 0.0, depth);
                shadow_view_proj  = light_proj * light_view;
                has_shadow_caster = true;
                break 'shadow;
            }
        }

        FrameData {
            camera,
            draw_calls,
            lights,
            viewport: (self.width, self.height),
            time: time.elapsed,
            sky,
            particles,
            debug_lines,
            shadow_view_proj,
            has_shadow_caster,
        }
    }

    /// Draw component gizmos for every entity in the world that has a recognisable component.
    /// Pushes into `fluxion_core::debug_draw` global (drained immediately after).
    fn populate_gizmos(&self, world: &ECSWorld) {
        let (w, h) = (self.width as f32, self.height as f32);
        let aspect = if h > 0.0 { w / h } else { 1.0 };

        // Editor grid (XZ plane)
        debug_draw::draw_grid(100.0, 100, Color::Custom(0.18, 0.20, 0.22, 1.0));

        // ── Camera frustum ────────────────────────────────────────────────────
        world.query_active::<(&Transform, &Camera), _>(|_id, (t, cam)| {
            if !cam.is_active { return; }
            let fwd   = t.world_forward();
            let up    = t.world_up();
            let right = fwd.cross(up).normalize_or_zero();
            let up2   = right.cross(fwd).normalize_or_zero();
            let vis_far = (cam.far * 0.08).clamp(1.0, 20.0);
            match cam.projection_mode {
                ProjectionMode::Perspective => {
                    debug_draw::draw_frustum(
                        t.world_position, fwd, up2, right,
                        cam.fov, aspect, cam.near, vis_far,
                        Color::Yellow,
                    );
                }
                ProjectionMode::Orthographic => {
                    // Draw a simple box for ortho cameras
                    let hw = cam.ortho_size * aspect;
                    let hh = cam.ortho_size;
                    let c  = Color::Custom(0.9, 0.8, 0.1, 1.0);
                    debug_draw::draw_aabb(
                        t.world_position - glam::Vec3::new(hw, hh, 0.1),
                        t.world_position + glam::Vec3::new(hw, hh, vis_far),
                        c,
                    );
                }
            }
        });

        // ── Light gizmos ──────────────────────────────────────────────────────
        world.query_active::<(&Transform, &Light), _>(|_id, (t, light)| {
            let pos = t.world_position;
            let c = Color::Custom(light.color[0], light.color[1], light.color[2], 1.0);
            match light.light_type {
                LightType::Point => {
                    debug_draw::draw_sphere(pos, light.range, c);
                }
                LightType::Spot => {
                    let fwd = t.world_forward();
                    let half_angle = (light.spot_angle * 0.5).to_radians();
                    debug_draw::draw_cone(pos, fwd, half_angle, light.range, 8, c);
                }
                LightType::Directional => {
                    let fwd   = t.world_forward();
                    let right = t.world_up().cross(fwd).normalize_or_zero();
                    let up    = fwd.cross(right).normalize_or_zero();
                    let len   = 1.5_f32;
                    let offsets = [
                        glam::Vec3::ZERO,
                        right * 0.5, -right * 0.5,
                        up * 0.5,    -up * 0.5,
                    ];
                    for off in offsets {
                        debug_draw::draw_line(pos + off, pos + off + fwd * len, c);
                    }
                }
            }
        });

        // ── Particle emitter cones ────────────────────────────────────────────
        world.query_active::<(&Transform, &ParticleEmitter), _>(|_id, (t, pe)| {
            let half_angle = (pe.spread_degrees * 0.5).to_radians();
            let emit_dir   = t.world_up();
            debug_draw::draw_cone(
                t.world_position, emit_dir, half_angle, 1.5,
                8, Color::Orange,
            );
        });

        // ── RigidBody collider wireframes ─────────────────────────────────────
        world.query_active::<(&Transform, &RigidBody), _>(|_id, (t, rb)| {
            let pos = t.world_position;
            let rot = t.world_rotation;
            let c   = Color::Lime;
            match rb.shape {
                PhysicsShape::Box { half_extents } => {
                    debug_draw::draw_box_rotated(
                        pos,
                        glam::Vec3::from(half_extents),
                        rot, c,
                    );
                }
                PhysicsShape::Sphere { radius } => {
                    debug_draw::draw_sphere(pos, radius, c);
                }
                PhysicsShape::Capsule { half_height, radius } => {
                    debug_draw::draw_capsule(pos, half_height, radius, rot, c);
                }
                PhysicsShape::HalfSpace => {
                    debug_draw::draw_line(
                        pos - glam::Vec3::X * 10.0,
                        pos + glam::Vec3::X * 10.0,
                        c,
                    );
                    debug_draw::draw_line(
                        pos - glam::Vec3::Z * 10.0,
                        pos + glam::Vec3::Z * 10.0,
                        c,
                    );
                }
            }
        });
    }

    fn compute_sky_params(settings: &SceneSettings, lights: &[LightUniform]) -> SkyParams {
        let mut sky = SkyParams::default();
        let t = settings.ambient_intensity.max(0.0);
        let a = settings.ambient_color;
        sky.horizon_color = [
            (0.55_f32 + a[0] * t * 1.5).clamp(0.08, 1.0),
            (0.65_f32 + a[1] * t * 1.5).clamp(0.08, 1.0),
            (0.95_f32 + a[2] * t * 1.2).clamp(0.15, 1.0),
        ];
        sky.zenith_color = [
            (0.08_f32 + a[0] * t).clamp(0.02, 1.0),
            (0.22_f32 + a[1] * t).clamp(0.05, 1.0),
            (0.55_f32 + a[2] * t).clamp(0.08, 1.0),
        ];
        for lu in lights {
            if lu.light_type == LIGHT_DIRECTIONAL {
                sky.sun_direction = lu.direction;
                break;
            }
        }
        let d = Vec3::from_array(sky.sun_direction);
        if d.length_squared() > 1e-8 {
            sky.sun_direction = d.normalize().to_array();
        }
        sky
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
