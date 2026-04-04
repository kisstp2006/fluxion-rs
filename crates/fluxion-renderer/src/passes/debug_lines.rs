// ============================================================
// fluxion-renderer — Debug line overlay pass
//
// Renders accumulated debug line segments as an overlay
// (no depth test — always on top of scene geometry).
// Lines come from fluxion_core::drain_debug_lines() which is
// called by the renderer before the render graph runs.
// ============================================================

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

use crate::render_graph::{RenderPass, RenderContext, RenderResources};
use crate::shader::library as shaders;

/// Vertex layout for a single line endpoint: position + RGBA color.
/// Stride = 28 bytes.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LineVertex {
    position: [f32; 3],
    color:    [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DebugCameraUniform {
    view_proj: [[f32; 4]; 4],
}

const INITIAL_CAPACITY: usize = 4096; // vertices (each line = 2 vertices)

pub struct DebugLinePass {
    surface_format: wgpu::TextureFormat,
    pipeline:       Option<wgpu::RenderPipeline>,
    camera_buf:     Option<wgpu::Buffer>,
    vertex_buf:     Option<wgpu::Buffer>,
    vertex_cap:     usize,
    bind_group:     Option<wgpu::BindGroup>,
    bgl:            Option<wgpu::BindGroupLayout>,
}

impl DebugLinePass {
    pub fn new(surface_format: wgpu::TextureFormat) -> Self {
        Self {
            surface_format,
            pipeline:   None,
            camera_buf: None,
            vertex_buf: None,
            vertex_cap: 0,
            bind_group: None,
            bgl:        None,
        }
    }

    fn ensure_vertex_buf(&mut self, device: &wgpu::Device, needed: usize) {
        if needed <= self.vertex_cap { return; }
        let new_cap = needed.next_power_of_two().max(INITIAL_CAPACITY);
        self.vertex_buf = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("debug_lines_vbuf"),
            size:               (std::mem::size_of::<LineVertex>() * new_cap) as u64,
            usage:              wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        self.vertex_cap = new_cap;
    }
}

impl RenderPass for DebugLinePass {
    fn name(&self) -> &str { "debug_lines" }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn prepare(&mut self, device: &wgpu::Device, _resources: &RenderResources) {
        use wgpu::util::DeviceExt;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("debug_lines_shader"),
            source: wgpu::ShaderSource::Wgsl(shaders::DEBUG_LINES.into()),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("debug_lines_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding:    0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty:                 wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size:   None,
                },
                count: None,
            }],
        });

        let camera_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some("debug_lines_camera_ubo"),
            contents: bytemuck::bytes_of(&DebugCameraUniform {
                view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("debug_lines_bg"),
            layout:  &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding:  0,
                resource: camera_buf.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:                Some("debug_lines_pl"),
            bind_group_layouts:   &[&bgl],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("debug_lines_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module:               &shader,
                entry_point:          "vs_main",
                compilation_options:  Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<LineVertex>() as u64,
                    step_mode:    wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset:           0,
                            shader_location:  0,
                            format:           wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset:           12,
                            shader_location:  1,
                            format:           wgpu::VertexFormat::Float32x4,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module:              &shader,
                entry_point:         "fs_main",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format:     self.surface_format,
                    blend:      Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology:          wgpu::PrimitiveTopology::LineList,
                strip_index_format: None,
                front_face:        wgpu::FrontFace::Ccw,
                cull_mode:         None,
                polygon_mode:      wgpu::PolygonMode::Fill,
                unclipped_depth:   false,
                conservative:      false,
            },
            depth_stencil: None,
            multisample:   wgpu::MultisampleState::default(),
            multiview:     None,
            cache:         None,
        });

        // Allocate initial vertex buffer
        self.ensure_vertex_buf(device, INITIAL_CAPACITY);
        self.bgl        = Some(bgl);
        self.camera_buf = Some(camera_buf);
        self.bind_group = Some(bind_group);
        self.pipeline   = Some(pipeline);
    }

    fn execute(&mut self, ctx: &mut RenderContext) {
        let lines = &ctx.frame.debug_lines;
        if lines.is_empty() { return; }

        let n_verts = lines.len() * 2;

        // Grow vertex buffer FIRST (requires &mut self), then borrow immutably.
        self.ensure_vertex_buf(ctx.device, n_verts);

        let pipeline   = match self.pipeline.as_ref()   { Some(p) => p, None => return };
        let camera_buf = match self.camera_buf.as_ref() { Some(b) => b, None => return };
        let bind_group = match self.bind_group.as_ref() { Some(g) => g, None => return };
        let vertex_buf = match self.vertex_buf.as_ref() { Some(b) => b, None => return };

        // Upload camera
        ctx.queue.write_buffer(
            camera_buf, 0,
            bytemuck::bytes_of(&DebugCameraUniform {
                view_proj: ctx.frame.camera.view_proj.to_cols_array_2d(),
            }),
        );

        // Pack line vertices
        let vertices: Vec<LineVertex> = lines.iter().flat_map(|l| {
            let c = [l.color.x, l.color.y, l.color.z, l.color.w];
            [
                LineVertex { position: [l.start.x, l.start.y, l.start.z], color: c },
                LineVertex { position: [l.end.x,   l.end.y,   l.end.z  ], color: c },
            ]
        }).collect();

        ctx.queue.write_buffer(vertex_buf, 0, bytemuck::cast_slice(&vertices));

        let mut rpass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("debug_lines_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view:           ctx.surface_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load:  wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });

        rpass.set_pipeline(pipeline);
        rpass.set_bind_group(0, bind_group, &[]);
        rpass.set_vertex_buffer(0, vertex_buf.slice(..));
        rpass.draw(0..n_verts as u32, 0..1);
    }
}

impl Default for DebugLinePass {
    fn default() -> Self {
        Self::new(wgpu::TextureFormat::Bgra8UnormSrgb)
    }
}
