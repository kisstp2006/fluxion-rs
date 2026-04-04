// ============================================================
// fluxion-renderer — GeometryPass
//
// First G-Buffer fill pass. Iterates over all opaque MeshDrawCalls
// in FrameData and writes material properties to the GBuffer MRT.
//
// Model matrices are stored in a single dynamic-offset UBO so we
// never allocate GPU objects per frame (no Device::maintain stalls).
// ============================================================

use bytemuck::{Pod, Zeroable};

use crate::render_graph::{RenderPass, RenderContext, RenderResources};
use crate::mesh::Vertex;
use crate::shader::library as shaders;

const MAX_DRAWS: usize = 1024;

/// Per-draw-call GPU uniforms (model + normal matrix). 128 bytes.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ModelUniforms {
    world_matrix:  [[f32; 4]; 4],
    normal_matrix: [[f32; 4]; 4],
}

/// Per-frame camera uniforms. 80 bytes.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniforms {
    view_proj:       [[f32; 4]; 4],
    camera_position: [f32; 3],
    _pad:            f32,
}

pub struct GeometryPass {
    pipeline:          Option<wgpu::RenderPipeline>,
    camera_bgl:        Option<wgpu::BindGroupLayout>,
    model_bgl:         Option<wgpu::BindGroupLayout>,
    camera_buffer:     Option<wgpu::Buffer>,
    model_buffer:      Option<wgpu::Buffer>,   // MAX_DRAWS slots, dynamic offset
    model_stride:      u64,                    // aligned sizeof(ModelUniforms)
    camera_bind_group: Option<wgpu::BindGroup>,
    model_bind_group:  Option<wgpu::BindGroup>, // bound once, offset per draw
}

impl GeometryPass {
    pub fn new() -> Self {
        Self {
            pipeline:          None,
            camera_bgl:        None,
            model_bgl:         None,
            camera_buffer:     None,
            model_buffer:      None,
            model_stride:      0,
            camera_bind_group: None,
            model_bind_group:  None,
        }
    }
}

impl RenderPass for GeometryPass {
    fn name(&self) -> &str { "geometry" }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn prepare(&mut self, device: &wgpu::Device, _resources: &RenderResources) {
        // wgpu requires dynamic uniform offsets to be aligned to
        // `min_uniform_buffer_offset_alignment` (256 bytes on most hardware).
        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let raw   = std::mem::size_of::<ModelUniforms>() as u64;
        let stride = (raw + align - 1) / align * align; // round up to alignment

        // ── Bind group layouts ────────────────────────────────────────────────
        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("geometry_camera_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size:   None,
                },
                count: None,
            }],
        });

        // Model BGL uses has_dynamic_offset = true so one bind group covers all draws.
        let model_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("geometry_model_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty:                 wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size:   std::num::NonZeroU64::new(raw),
                },
                count: None,
            }],
        });

        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding, visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2, multisampled: false,
            }, count: None,
        };
        let samp_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding, visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let material_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("geometry_material_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false, min_binding_size: None,
                    }, count: None,
                },
                tex_entry(1), samp_entry(2),
                tex_entry(3), samp_entry(4),
                tex_entry(5), samp_entry(6),
                tex_entry(7), samp_entry(8),
            ],
        });

        // ── Buffers ───────────────────────────────────────────────────────────
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("geometry_camera_ubo"),
            size:  std::mem::size_of::<CameraUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let model_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("geometry_model_ubo"),
            size:  stride * MAX_DRAWS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Bind groups ───────────────────────────────────────────────────────
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("geometry_camera_bg"), layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() }],
        });

        // The model bind group uses a range of exactly `raw` bytes so the
        // dynamic offset slides it to the correct per-draw slot.
        let model_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("geometry_model_bg"), layout: &model_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &model_buffer,
                    offset: 0,
                    size:   std::num::NonZeroU64::new(raw),
                }),
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
            label: Some("geometry_layout"),
            bind_group_layouts:   &[&camera_bgl, &model_bgl, &material_bgl],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("geometry_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vert_module, entry_point: "vs_main",
                buffers: &[Vertex::layout()], compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &frag_module, entry_point: "fs_main",
                targets: &[
                    Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8UnormSrgb, blend: None, write_mask: wgpu::ColorWrites::ALL }),
                    Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm,     blend: None, write_mask: wgpu::ColorWrites::ALL }),
                    Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm,     blend: None, write_mask: wgpu::ColorWrites::ALL }),
                    Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba16Float,    blend: None, write_mask: wgpu::ColorWrites::ALL }),
                ],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology:  wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format:              wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare:       wgpu::CompareFunction::Less,
                stencil:             Default::default(),
                bias:                Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview:   None,
            cache:       None,
        });

        self.pipeline          = Some(pipeline);
        self.camera_bgl        = Some(camera_bgl);
        self.model_bgl         = Some(model_bgl);
        self.camera_buffer     = Some(camera_buffer);
        self.model_buffer      = Some(model_buffer);
        self.model_stride      = stride;
        self.camera_bind_group = Some(camera_bind_group);
        self.model_bind_group  = Some(model_bind_group);
    }

    fn execute(&mut self, ctx: &mut RenderContext) {
        let pipeline   = match self.pipeline.as_ref()          { Some(p) => p, None => return };
        let camera_buf = match self.camera_buffer.as_ref()     { Some(b) => b, None => return };
        let model_buf  = match self.model_buffer.as_ref()      { Some(b) => b, None => return };
        let camera_bg  = match self.camera_bind_group.as_ref() { Some(g) => g, None => return };
        let model_bg   = match self.model_bind_group.as_ref()  { Some(g) => g, None => return };
        let stride     = self.model_stride;

        // ── Upload camera UBO ─────────────────────────────────────────────────
        let cam = &ctx.frame.camera;
        ctx.queue.write_buffer(camera_buf, 0, bytemuck::bytes_of(&CameraUniforms {
            view_proj:       cam.view_proj.to_cols_array_2d(),
            camera_position: cam.position.to_array(),
            _pad:            0.0,
        }));

        // ── Upload all model UBOs in one write_buffer call ────────────────────
        // Pack into a CPU buffer, then send to GPU in one shot.
        let raw = std::mem::size_of::<ModelUniforms>();
        let mut model_staging = vec![0u8; stride as usize * MAX_DRAWS];
        let mut draw_count = 0usize;

        for draw in ctx.frame.draw_calls.iter().take(MAX_DRAWS) {
            let data = ModelUniforms {
                world_matrix:  draw.world_matrix.to_cols_array_2d(),
                normal_matrix: draw.normal_matrix.to_cols_array_2d(),
            };
            let offset = draw_count * stride as usize;
            model_staging[offset..offset + raw].copy_from_slice(bytemuck::bytes_of(&data));
            draw_count += 1;
        }

        if draw_count > 0 {
            ctx.queue.write_buffer(model_buf, 0,
                &model_staging[..draw_count * stride as usize]);
        }

        // ── Begin MRT render pass ─────────────────────────────────────────────
        let res = ctx.resources;
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

        for (i, draw) in ctx.frame.draw_calls.iter().enumerate().take(MAX_DRAWS) {
            let mesh     = match ctx.meshes.get(draw.mesh)      { Some(m) => m, None => continue };
            let material = match ctx.materials.get(draw.material) { Some(m) => m, None => continue };

            let dynamic_offset = (i as u64 * stride) as u32;
            rpass.set_bind_group(1, model_bg, &[dynamic_offset]);
            rpass.set_bind_group(2, &material.bind_group, &[]);
            rpass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            rpass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            rpass.draw_indexed(0..mesh.index_count, 0, 0..1);
        }
    }
}

impl Default for GeometryPass { fn default() -> Self { Self::new() } }
