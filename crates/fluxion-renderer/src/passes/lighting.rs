// ============================================================
// fluxion-renderer — LightingPass
//
// Full-screen deferred PBR lighting. Reads GBuffer, accumulates
// all light contributions, writes to the HDR render target.
// ============================================================

use bytemuck::{Pod, Zeroable};
use crate::render_graph::{RenderPass, RenderContext, RenderResources};
use crate::shader::library as shaders;

/// Shadow uniforms uploaded per-frame to group(3) binding(0) in pbr_lighting.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShadowUniforms {
    light_view_proj: [[f32; 4]; 4],
    has_shadow:      u32,
    _pad0:           u32,
    _pad1:           u32,
    _pad2:           u32,
}

#[repr(C)] #[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniforms {
    view_proj:     [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    camera_pos:    [f32; 3],
    _pad:          f32,
}

pub struct LightingPass {
    pipeline:       Option<wgpu::RenderPipeline>,
    camera_buf:     Option<wgpu::Buffer>,
    camera_bgl:     Option<wgpu::BindGroupLayout>,
    light_bgl:      Option<wgpu::BindGroupLayout>,
    gbuf_bgl:       Option<wgpu::BindGroupLayout>,
    shadow_bgl:     Option<wgpu::BindGroupLayout>,
    camera_bg:      Option<wgpu::BindGroup>,
    light_bg:       Option<wgpu::BindGroup>,  // built once on first execute
    gbuf_bg:        Option<wgpu::BindGroup>,
    shadow_bg:      Option<wgpu::BindGroup>,  // rebuilt when shadow map changes
    shadow_buf:     Option<wgpu::Buffer>,     // ShadowUniforms UBO
}

impl LightingPass {
    pub fn new() -> Self {
        Self {
            pipeline:   None, camera_buf:  None, camera_bgl: None,
            light_bgl:  None, gbuf_bgl:    None, shadow_bgl: None,
            camera_bg:  None, light_bg:    None, gbuf_bg:    None,
            shadow_bg:  None, shadow_buf:  None,
        }
    }

    fn rebuild_gbuf_bind_group(&mut self, device: &wgpu::Device, resources: &RenderResources) {
        let bgl = match self.gbuf_bgl.as_ref() { Some(l) => l, None => return };
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lighting_gbuf_bg"),
            layout: bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&resources.gbuf_albedo_ao.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&resources.gbuf_normal.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&resources.gbuf_orm.view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&resources.gbuf_emission.view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&resources.depth.view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&resources.gbuf_albedo_ao.sampler) },
            ],
        });
        self.gbuf_bg = Some(bg);
    }

    fn rebuild_shadow_bind_group(&mut self, device: &wgpu::Device, resources: &RenderResources) {
        let bgl = match self.shadow_bgl.as_ref() { Some(l) => l, None => return };
        let buf = match self.shadow_buf.as_ref()  { Some(b) => b, None => return };
        // Comparison sampler for PCF (shadow_sampler in WGSL).
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:         Some("shadow_cmp_sampler"),
            compare:       Some(wgpu::CompareFunction::LessEqual),
            mag_filter:    wgpu::FilterMode::Linear,
            min_filter:    wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("lighting_shadow_bg"),
            layout:  bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&resources.shadow_map.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&shadow_sampler) },
            ],
        });
        self.shadow_bg = Some(bg);
    }
}

impl RenderPass for LightingPass {
    fn name(&self) -> &str { "lighting" }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn prepare(&mut self, device: &wgpu::Device, resources: &RenderResources) {
        let vert = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lighting_vert"), source: wgpu::ShaderSource::Wgsl(shaders::FULLSCREEN_VERT.into()) });
        let frag = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lighting_frag"), source: wgpu::ShaderSource::Wgsl(shaders::PBR_LIGHTING.into()) });

        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lighting_camera_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });

        // group(1): light buffer uniform
        let light_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lighting_light_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });

        // group(2): GBuffer textures
        let gbuf_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lighting_gbuf_bgl"),
            entries: &[
                // bindings 0-4: GBuffer textures
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                // binding 4: depth (non-filterable)
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                // binding 5: shared sampler
                wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });

        let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lighting_camera_ubo"), size: std::mem::size_of::<CameraUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });

        let camera_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lighting_camera_bg"), layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_buf.as_entire_binding() }],
        });

        // group(3): shadow uniforms + shadow map depth texture + comparison sampler
        let shadow_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("lighting_shadow_bgl"),
            entries: &[
                // binding(0): ShadowUniforms UBO
                wgpu::BindGroupLayoutEntry {
                    binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false, min_binding_size: None,
                    },
                    count: None,
                },
                // binding(1): shadow depth texture
                wgpu::BindGroupLayoutEntry {
                    binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type:    wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled:   false,
                    },
                    count: None,
                },
                // binding(2): comparison sampler for PCF
                wgpu::BindGroupLayoutEntry {
                    binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });

        let shadow_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lighting_shadow_ubo"),
            size:  std::mem::size_of::<ShadowUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lighting_layout"),
            bind_group_layouts: &[&camera_bgl, &light_bgl, &gbuf_bgl, &shadow_bgl],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lighting_pipeline"), layout: Some(&layout),
            vertex: wgpu::VertexState { module: &vert, entry_point: "vs_main", buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &frag, entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba16Float, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default() }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None, cache: None,
        });

        self.gbuf_bgl   = Some(gbuf_bgl);
        self.shadow_bgl = Some(shadow_bgl);
        self.light_bgl  = Some(light_bgl);
        self.camera_bgl = Some(camera_bgl);
        self.camera_buf = Some(camera_buf);
        self.shadow_buf = Some(shadow_buf);
        self.camera_bg  = Some(camera_bg);
        self.pipeline   = Some(pipeline);
        self.rebuild_gbuf_bind_group(device, resources);
        self.rebuild_shadow_bind_group(device, resources);
    }

    fn resize(&mut self, _device: &wgpu::Device, _w: u32, _h: u32) {
        // GBuffer and shadow textures were recreated — null them out so they rebuild on next execute.
        self.gbuf_bg   = None;
        self.light_bg  = None;
        self.shadow_bg = None;
    }

    fn execute(&mut self, ctx: &mut RenderContext) {
        // Lazy-build stable bind groups (lost on resize, rebuilt here on demand).
        if self.light_bg.is_none() {
            if let Some(bgl) = self.light_bgl.as_ref() {
                self.light_bg = Some(ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("lighting_light_bg"),
                    layout: bgl,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: ctx.light_buffer.as_entire_binding(),
                    }],
                }));
            }
        }
        if self.gbuf_bg.is_none() {
            self.rebuild_gbuf_bind_group(ctx.device, ctx.resources);
        }
        if self.shadow_bg.is_none() {
            self.rebuild_shadow_bind_group(ctx.device, ctx.resources);
        }

        let pipeline   = match self.pipeline.as_ref()    { Some(p) => p, None => return };
        let camera_bg  = match self.camera_bg.as_ref()   { Some(g) => g, None => return };
        let light_bg   = match self.light_bg.as_ref()    { Some(g) => g, None => return };
        let gbuf_bg    = match self.gbuf_bg.as_ref()      { Some(g) => g, None => return };
        let shadow_bg  = match self.shadow_bg.as_ref()   { Some(g) => g, None => return };
        let camera_buf = match self.camera_buf.as_ref()  { Some(b) => b, None => return };
        let shadow_buf = match self.shadow_buf.as_ref()  { Some(b) => b, None => return };

        let cam = &ctx.frame.camera;
        let data = CameraUniforms {
            view_proj:     cam.view_proj.to_cols_array_2d(),
            inv_view_proj: cam.inv_view_proj.to_cols_array_2d(),
            camera_pos:    cam.position.to_array(),
            _pad:          0.0,
        };
        ctx.queue.write_buffer(camera_buf, 0, bytemuck::bytes_of(&data));

        // Upload shadow uniforms (light-space matrix + has_shadow flag).
        ctx.queue.write_buffer(shadow_buf, 0, bytemuck::bytes_of(&ShadowUniforms {
            light_view_proj: ctx.frame.shadow_view_proj.to_cols_array_2d(),
            has_shadow:      ctx.frame.has_shadow_caster as u32,
            _pad0: 0, _pad1: 0, _pad2: 0,
        }));

        let mut rpass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("lighting_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &ctx.resources.hdr_main.view, resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });

        rpass.set_pipeline(pipeline);
        rpass.set_bind_group(0, camera_bg,  &[]);
        rpass.set_bind_group(1, light_bg,   &[]);
        rpass.set_bind_group(2, gbuf_bg,    &[]);
        rpass.set_bind_group(3, shadow_bg,  &[]);
        rpass.draw(0..3, 0..1); // fullscreen triangle
    }
}

impl Default for LightingPass { fn default() -> Self { Self::new() } }
