// ============================================================
// fluxion-renderer — SkyboxPass (multi-mode sky)
//
// Supports four sky modes controlled by SkyParams.sky_mode:
//   0 = Gradient (horizon/zenith + sun disc)
//   1 = Preetham analytical atmosphere
//   2 = Solid color
//   3 = Panorama equirectangular texture
//
// Bindings: 0=SkyParams, 1=CameraUniforms, 2=depth, 3=panorama_tex, 4=panorama_samp
// ============================================================

pub use crate::render_graph::SkyParams;
use crate::render_graph::{RenderPass, RenderContext, RenderResources};
use crate::shader::library as shaders;

pub struct SkyboxPass {
    pub params: SkyParams,
    pipeline:   Option<wgpu::RenderPipeline>,
    bgl:        Option<wgpu::BindGroupLayout>,
    bind_group: Option<wgpu::BindGroup>,
    sky_buf:    Option<wgpu::Buffer>,
    camera_buf: Option<wgpu::Buffer>,
    /// Optional panorama texture (mode 3). Set via `set_panorama_texture`.
    pub panorama_texture: Option<wgpu::Texture>,
    panorama_view:        Option<wgpu::TextureView>,
    panorama_sampler:     Option<wgpu::Sampler>,
    /// 1×1 white fallback used when no panorama is loaded.
    fallback_texture:  Option<wgpu::Texture>,
    fallback_view:     Option<wgpu::TextureView>,
    fallback_sampler:  Option<wgpu::Sampler>,
}

impl SkyboxPass {
    pub fn new() -> Self {
        Self {
            params: SkyParams::default(),
            pipeline: None, bgl: None, bind_group: None,
            sky_buf: None, camera_buf: None,
            panorama_texture: None, panorama_view: None, panorama_sampler: None,
            fallback_texture: None, fallback_view: None, fallback_sampler: None,
        }
    }

    /// Upload a panorama RGBA texture (Rgba8UnormSrgb) and invalidate the bind group.
    pub fn set_panorama_texture(
        &mut self,
        device: &wgpu::Device,
        queue:  &wgpu::Queue,
        width:  u32,
        height: u32,
        data:   &[u8],      // raw RGBA8 bytes
    ) {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label:  Some("sky_panorama"),
            size:   wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          wgpu::TextureFormat::Rgba8UnormSrgb,
            usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats:    &[],
        });
        queue.write_texture(
            wgpu::ImageCopyTexture { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            data,
            wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(4 * width), rows_per_image: Some(height) },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        let view = tex.create_view(&Default::default());
        self.panorama_texture = Some(tex);
        self.panorama_view    = Some(view);
        self.bind_group = None; // force rebuild
    }
}

impl RenderPass for SkyboxPass {
    fn name(&self) -> &str { "skybox" }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn prepare(&mut self, device: &wgpu::Device, _resources: &RenderResources) {
        use wgpu::util::DeviceExt;

        let vert = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky_vert"),
            source: wgpu::ShaderSource::Wgsl(shaders::FULLSCREEN_VERT.into()),
        });
        let frag = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky_frag"),
            source: wgpu::ShaderSource::Wgsl(shaders::SKYBOX.into()),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky_bgl"),
            entries: &[
                // 0: SkyParams uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                // 1: CameraUniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                // 2: depth texture (sky mask)
                wgpu::BindGroupLayoutEntry {
                    binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                // 3: panorama texture (2D Rgba8)
                wgpu::BindGroupLayoutEntry {
                    binding: 3, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                // 4: panorama sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 4, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let sky_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sky_params_buf"),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            contents: bytemuck::bytes_of(&self.params),
        });

        let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky_camera_buf"),
            size: 144, // mat4×4 (64) + mat4×4 (64) + vec3 (12) + pad (4) = 144
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // 1×1 white fallback texture — used when no panorama is loaded
        let fallback = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky_fallback"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format:    wgpu::TextureFormat::Rgba8UnormSrgb,
            usage:     wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // leave fallback as zero — black is fine for non-panorama modes

        let fallback_view    = fallback.create_view(&Default::default());
        let fallback_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky_fallback_samp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let panorama_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky_panorama_samp"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None, bind_group_layouts: &[&bgl], push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky_pipeline"), layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &vert, entry_point: "vs_main", buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &frag, entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: None, write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive:    wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample:  wgpu::MultisampleState::default(),
            multiview:    None,
            cache:        None,
        });

        self.bgl             = Some(bgl);
        self.sky_buf         = Some(sky_buf);
        self.camera_buf      = Some(camera_buf);
        self.pipeline        = Some(pipeline);
        self.fallback_texture = Some(fallback);
        self.fallback_view    = Some(fallback_view);
        self.fallback_sampler = Some(fallback_sampler);
        self.panorama_sampler = Some(panorama_sampler);
    }

    fn resize(&mut self, _device: &wgpu::Device, _w: u32, _h: u32) {
        // depth texture was recreated — invalidate cached bind group
        self.bind_group = None;
    }

    fn execute(&mut self, ctx: &mut RenderContext) {
        // Choose panorama view/sampler or fallbacks
        let panorama_view = self.panorama_view.as_ref()
            .or(self.fallback_view.as_ref());
        let panorama_samp = self.panorama_sampler.as_ref()
            .or(self.fallback_sampler.as_ref());

        // Lazy-build bind group (depth.view + optional panorama change)
        if self.bind_group.is_none() {
            if let (Some(bgl), Some(sky_buf), Some(camera_buf), Some(pv), Some(ps)) = (
                self.bgl.as_ref(), self.sky_buf.as_ref(), self.camera_buf.as_ref(),
                panorama_view, panorama_samp,
            ) {
                self.bind_group = Some(ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("sky_bg"), layout: bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: sky_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: camera_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&ctx.resources.depth.view) },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(pv) },
                        wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(ps) },
                    ],
                }));
            }
        }

        let pipeline   = match self.pipeline.as_ref()  { Some(p) => p, None => return };
        let sky_buf    = match self.sky_buf.as_ref()    { Some(b) => b, None => return };
        let camera_buf = match self.camera_buf.as_ref() { Some(b) => b, None => return };
        let bind_group = match self.bind_group.as_ref() { Some(g) => g, None => return };

        ctx.queue.write_buffer(sky_buf, 0, bytemuck::bytes_of(&ctx.frame.sky));

        // Upload camera matrices + position
        #[repr(C)] #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct CamData { vp: [[f32;4];4], ivp: [[f32;4];4], pos: [f32;3], _p: f32 }
        let cam = &ctx.frame.camera;
        let cam_data = CamData {
            vp:  cam.view_proj.to_cols_array_2d(),
            ivp: cam.inv_view_proj.to_cols_array_2d(),
            pos: cam.position.to_array(),
            _p:  0.0,
        };
        ctx.queue.write_buffer(camera_buf, 0, bytemuck::bytes_of(&cam_data));

        let mut rpass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("skybox_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &ctx.resources.hdr_main.view, resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None, ..Default::default()
        });
        rpass.set_pipeline(pipeline);
        rpass.set_bind_group(0, bind_group, &[]);
        rpass.draw(0..3, 0..1);
    }
}

impl Default for SkyboxPass { fn default() -> Self { Self::new() } }
