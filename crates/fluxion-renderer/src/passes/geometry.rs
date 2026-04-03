// ============================================================
// fluxion-renderer — GeometryPass
//
// First G-Buffer fill pass. Iterates over all opaque MeshDrawCalls
// in FrameData and writes material properties to the GBuffer MRT.
//
// Outputs (written to RenderResources):
//   gbuf_albedo_ao — RGB=albedo, A=AO
//   gbuf_normal    — packed world normal
//   gbuf_orm       — R=occlusion, G=roughness, B=metalness
//   gbuf_emission  — RGB=emission
//   depth          — depth buffer
// ============================================================

use bytemuck::{Pod, Zeroable};

use crate::render_graph::{RenderPass, RenderContext, RenderResources};
use crate::mesh::Vertex;
use crate::shader::library as shaders;

/// Per-draw-call GPU uniforms (model + normal matrix).
/// Pushed to group(1) binding(0) for each mesh.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ModelUniforms {
    world_matrix:  [[f32; 4]; 4],
    normal_matrix: [[f32; 4]; 4],
}

/// Per-frame camera uniforms.
/// Pushed to group(0) binding(0) once per frame.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniforms {
    view_proj: [[f32; 4]; 4],
    camera_position: [f32; 3],
    _pad: f32,
}

pub struct GeometryPass {
    pipeline:         Option<wgpu::RenderPipeline>,
    camera_bgl:       Option<wgpu::BindGroupLayout>,
    model_bgl:        Option<wgpu::BindGroupLayout>,
    material_bgl:     Option<wgpu::BindGroupLayout>,
    camera_buffer:    Option<wgpu::Buffer>,
    model_buffer:     Option<wgpu::Buffer>,
    camera_bind_group: Option<wgpu::BindGroup>,
}

impl GeometryPass {
    pub fn new() -> Self {
        Self {
            pipeline:          None,
            camera_bgl:        None,
            model_bgl:         None,
            material_bgl:      None,
            camera_buffer:     None,
            model_buffer:      None,
            camera_bind_group: None,
        }
    }
}

impl RenderPass for GeometryPass {
    fn name(&self) -> &str { "geometry" }

    fn prepare(&mut self, device: &wgpu::Device, _resources: &RenderResources) {
        // ── Bind group layouts ────────────────────────────────────────────────
        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("geometry_camera_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding:    0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty:         wgpu::BindingType::Buffer {
                    ty:                 wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size:   None,
                },
                count: None,
            }],
        });

        let model_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("geometry_model_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding:    0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty:         wgpu::BindingType::Buffer {
                    ty:                 wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size:   None,
                },
                count: None,
            }],
        });

        // Material bind group: 1 uniform + 4 texture/sampler pairs (8 texture bindings + 1 UBO)
        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled:   false,
            },
            count: None,
        };
        let samp_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };

        let material_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("geometry_material_bgl"),
            entries: &[
                // binding 0: PbrParams uniform
                wgpu::BindGroupLayoutEntry {
                    binding:    0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty:         wgpu::BindingType::Buffer {
                        ty:                 wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size:   None,
                    },
                    count: None,
                },
                tex_entry(1), samp_entry(2), // albedo
                tex_entry(3), samp_entry(4), // normal
                tex_entry(5), samp_entry(6), // orm
                tex_entry(7), samp_entry(8), // emissive
            ],
        });

        // ── Uniform buffers ───────────────────────────────────────────────────
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("geometry_camera_ubo"),
            size:               std::mem::size_of::<CameraUniforms>() as u64,
            usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let model_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("geometry_model_ubo"),
            size:               std::mem::size_of::<ModelUniforms>() as u64,
            usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("geometry_camera_bg"),
            layout:  &camera_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding:  0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        // ── Shaders ───────────────────────────────────────────────────────────
        let vert_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("geometry_vert"),
            source: wgpu::ShaderSource::Wgsl(shaders::GEOMETRY_VERT.into()),
        });
        let frag_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("geometry_frag"),
            source: wgpu::ShaderSource::Wgsl(shaders::GEOMETRY_FRAG.into()),
        });

        // ── Pipeline ─────────────────────────────────────────────────────────
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:                Some("geometry_pipeline_layout"),
            bind_group_layouts:   &[&camera_bgl, &model_bgl, &material_bgl],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("geometry_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module:              &vert_module,
                entry_point:         "vs_main",
                buffers:             &[Vertex::layout()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module:              &frag_module,
                entry_point:         "fs_main",
                targets: &[
                    Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8UnormSrgb, blend: None, write_mask: wgpu::ColorWrites::ALL }),
                    Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm,     blend: None, write_mask: wgpu::ColorWrites::ALL }),
                    Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm,     blend: None, write_mask: wgpu::ColorWrites::ALL }),
                    Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba16Float,    blend: None, write_mask: wgpu::ColorWrites::ALL }),
                ],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology:           wgpu::PrimitiveTopology::TriangleList,
                cull_mode:          Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format:              wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare:       wgpu::CompareFunction::Less,
                stencil:             Default::default(),
                bias:                Default::default(),
            }),
            multisample:   wgpu::MultisampleState::default(),
            multiview:     None,
            cache:         None,
        });

        self.pipeline          = Some(pipeline);
        self.camera_bgl        = Some(camera_bgl);
        self.model_bgl         = Some(model_bgl);
        self.material_bgl      = Some(material_bgl);
        self.camera_buffer     = Some(camera_buffer);
        self.model_buffer      = Some(model_buffer);
        self.camera_bind_group = Some(camera_bind_group);
    }

    fn execute(&mut self, ctx: &mut RenderContext) {
        let pipeline         = match self.pipeline.as_ref()          { Some(p) => p, None => return };
        let camera_buf       = match self.camera_buffer.as_ref()     { Some(b) => b, None => return };
        let model_buf        = match self.model_buffer.as_ref()      { Some(b) => b, None => return };
        let camera_bg        = match self.camera_bind_group.as_ref() { Some(g) => g, None => return };
        let model_bgl        = match self.model_bgl.as_ref()         { Some(l) => l, None => return };

        let res = ctx.resources;
        let cam = &ctx.frame.camera;

        // Upload camera uniforms
        let cam_data = CameraUniforms {
            view_proj:       cam.view_proj.to_cols_array_2d(),
            camera_position: cam.position.to_array(),
            _pad:            0.0,
        };
        ctx.queue.write_buffer(camera_buf, 0, bytemuck::bytes_of(&cam_data));

        // Begin render pass — MRT: 4 color attachments + depth
        let mut rpass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("geometry_pass"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment { view: &res.gbuf_albedo_ao.view, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store }}),
                Some(wgpu::RenderPassColorAttachment { view: &res.gbuf_normal.view, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.5, g: 0.5, b: 1.0, a: 0.0 }), store: wgpu::StoreOp::Store }}),
                Some(wgpu::RenderPassColorAttachment { view: &res.gbuf_orm.view, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store }}),
                Some(wgpu::RenderPassColorAttachment { view: &res.gbuf_emission.view, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store }}),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &res.depth.view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        rpass.set_pipeline(pipeline);
        rpass.set_bind_group(0, camera_bg, &[]);

        // Build a per-draw model bind group and issue draw calls.
        for draw in &ctx.frame.draw_calls {
            let mesh = match ctx.meshes.get(draw.mesh) {
                Some(m) => m,
                None    => continue,
            };
            let material = match ctx.materials.get(draw.material) {
                Some(m) => m,
                None    => continue,
            };

            // Upload model uniforms for this draw call
            let model_data = ModelUniforms {
                world_matrix:  draw.world_matrix.to_cols_array_2d(),
                normal_matrix: draw.normal_matrix.to_cols_array_2d(),
            };
            ctx.queue.write_buffer(model_buf, 0, bytemuck::bytes_of(&model_data));

            // Build model bind group (one per draw — cheap, buffer is reused)
            let model_bg = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label:   Some("geometry_model_bg"),
                layout:  model_bgl,
                entries: &[wgpu::BindGroupEntry {
                    binding:  0,
                    resource: model_buf.as_entire_binding(),
                }],
            });

            rpass.set_bind_group(1, &model_bg, &[]);
            rpass.set_bind_group(2, &material.bind_group, &[]);
            rpass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            rpass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            rpass.draw_indexed(0..mesh.index_count, 0, 0..1);
        }
    }
}

impl Default for GeometryPass { fn default() -> Self { Self::new() } }
