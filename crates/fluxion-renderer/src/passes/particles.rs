// ============================================================
// fluxion-renderer — Particle overlay pass (after tonemap)
//
// Instanced billboards, alpha blend on top of the swapchain.
// ============================================================

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

use crate::render_graph::{RenderPass, RenderContext, RenderResources};
use crate::render_graph::context::ParticleInstance;
use crate::shader::library as shaders;

const MAX_INSTANCES: usize = 4096;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_proj:   [[f32; 4]; 4],
    camera_pos:  [f32; 4],
}

pub struct ParticleOverlayPass {
    surface_format: wgpu::TextureFormat,
    pipeline:       Option<wgpu::RenderPipeline>,
    camera_buf:     Option<wgpu::Buffer>,
    instance_buf:   Option<wgpu::Buffer>,
    bind_group:     Option<wgpu::BindGroup>,
    bgl:            Option<wgpu::BindGroupLayout>,
}

impl ParticleOverlayPass {
    pub fn new(surface_format: wgpu::TextureFormat) -> Self {
        Self {
            surface_format,
            pipeline: None,
            camera_buf: None,
            instance_buf: None,
            bind_group: None,
            bgl: None,
        }
    }
}

impl RenderPass for ParticleOverlayPass {
    fn name(&self) -> &str { "particles_overlay" }

    fn prepare(&mut self, device: &wgpu::Device, _resources: &RenderResources) {
        use wgpu::util::DeviceExt;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particles_shader"),
            source: wgpu::ShaderSource::Wgsl(shaders::PARTICLES.into()),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particles_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let camera_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("particles_camera_ubo"),
            contents: bytemuck::bytes_of(&CameraUniform {
                view_proj: Mat4::IDENTITY.to_cols_array_2d(),
                camera_pos: [0.0; 4],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particles_instances"),
            size: (std::mem::size_of::<ParticleInstance>() * MAX_INSTANCES) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particles_bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buf.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particles_pl"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });

        let blend = wgpu::BlendState::ALPHA_BLENDING;
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particles_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<ParticleInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute { offset: 0,  shader_location: 0, format: wgpu::VertexFormat::Float32x3 },
                        wgpu::VertexAttribute { offset: 12, shader_location: 1, format: wgpu::VertexFormat::Float32 },
                        wgpu::VertexAttribute { offset: 16, shader_location: 2, format: wgpu::VertexFormat::Float32x4 },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.surface_format,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        self.bgl = Some(bgl);
        self.camera_buf = Some(camera_buf);
        self.instance_buf = Some(instance_buf);
        self.bind_group = Some(bind_group);
        self.pipeline = Some(pipeline);
    }

    fn execute(&mut self, ctx: &mut RenderContext) {
        let n = ctx.frame.particles.len().min(MAX_INSTANCES);
        if n == 0 {
            return;
        }

        let pipeline   = match self.pipeline.as_ref()     { Some(p) => p, None => return };
        let camera_buf = match self.camera_buf.as_ref()   { Some(b) => b, None => return };
        let inst_buf   = match self.instance_buf.as_ref() { Some(b) => b, None => return };
        let bind_group = match self.bind_group.as_ref()   { Some(g) => g, None => return };

        let cam = &ctx.frame.camera;
        let u = CameraUniform {
            view_proj: cam.view_proj.to_cols_array_2d(),
            camera_pos: [cam.position.x, cam.position.y, cam.position.z, 0.0],
        };
        ctx.queue.write_buffer(camera_buf, 0, bytemuck::bytes_of(&u));

        let slice = bytemuck::cast_slice(&ctx.frame.particles[..n]);
        ctx.queue.write_buffer(inst_buf, 0, slice);

        let mut rpass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("particles_overlay"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: ctx.surface_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        rpass.set_pipeline(pipeline);
        rpass.set_bind_group(0, bind_group, &[]);
        rpass.set_vertex_buffer(0, inst_buf.slice(..));
        rpass.draw(0..6, 0..n as u32);
    }
}

impl Default for ParticleOverlayPass {
    fn default() -> Self {
        Self::new(wgpu::TextureFormat::Bgra8UnormSrgb)
    }
}
