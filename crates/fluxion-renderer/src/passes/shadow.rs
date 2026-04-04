// ============================================================
// fluxion-renderer — ShadowPass
//
// Renders scene geometry from the first directional shadow-casting
// light's point of view, writing a depth-only shadow map.
//
// The resulting shadow map texture is stored in RenderResources::shadow_map
// and sampled by LightingPass via group(3) in pbr_lighting.wgsl.
//
// Only the first shadow-casting directional light is supported (single
// cascade). CSM can be added later by replicating the pass N times.
// ============================================================

use bytemuck::{Pod, Zeroable};

use crate::render_graph::{RenderPass, RenderContext, RenderResources};
use crate::mesh::Vertex;
use crate::shader::library as shaders;

const MAX_DRAWS: usize = 1024;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShadowCamera {
    light_view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ModelUniforms {
    world_matrix:  [[f32; 4]; 4],
    normal_matrix: [[f32; 4]; 4],
}

pub struct ShadowPass {
    pipeline:       Option<wgpu::RenderPipeline>,
    cam_bgl:        Option<wgpu::BindGroupLayout>,
    model_bgl:      Option<wgpu::BindGroupLayout>,
    cam_buffer:     Option<wgpu::Buffer>,
    model_buffer:   Option<wgpu::Buffer>,
    model_stride:   u64,
    cam_bg:         Option<wgpu::BindGroup>,
    model_bg:       Option<wgpu::BindGroup>,
}

impl ShadowPass {
    pub fn new() -> Self {
        Self {
            pipeline:     None,
            cam_bgl:      None,
            model_bgl:    None,
            cam_buffer:   None,
            model_buffer: None,
            model_stride: 0,
            cam_bg:       None,
            model_bg:     None,
        }
    }
}

impl RenderPass for ShadowPass {
    fn name(&self) -> &str { "shadow" }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn prepare(&mut self, device: &wgpu::Device, _resources: &RenderResources) {
        let align  = device.limits().min_uniform_buffer_offset_alignment as u64;
        let raw    = std::mem::size_of::<ModelUniforms>() as u64;
        let stride = (raw + align - 1) / align * align;

        // ── Bind group layouts ────────────────────────────────────────────────
        let cam_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("shadow_cam_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let model_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("shadow_model_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: std::num::NonZeroU64::new(raw),
                },
                count: None,
            }],
        });

        // ── Buffers ───────────────────────────────────────────────────────────
        let cam_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow_cam_ubo"),
            size:  std::mem::size_of::<ShadowCamera>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let model_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow_model_ubo"),
            size:  stride * MAX_DRAWS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Bind groups ───────────────────────────────────────────────────────
        let cam_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("shadow_cam_bg"),
            layout:  &cam_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding:  0,
                resource: cam_buffer.as_entire_binding(),
            }],
        });

        let model_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("shadow_model_bg"),
            layout:  &model_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &model_buffer,
                    offset: 0,
                    size:   std::num::NonZeroU64::new(raw),
                }),
            }],
        });

        // ── Shader ────────────────────────────────────────────────────────────
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("shadow_depth_shader"),
            source: wgpu::ShaderSource::Wgsl(shaders::SHADOW_DEPTH.into()),
        });

        // ── Pipeline (depth-only, no color attachments) ───────────────────────
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow_pipeline_layout"),
            bind_group_layouts:   &[&cam_bgl, &model_bgl],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("shadow_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module:      &shader,
                entry_point: "vs_main",
                buffers:     &[Vertex::layout()],
                compilation_options: Default::default(),
            },
            fragment: None,  // depth-only — no fragment output
            primitive: wgpu::PrimitiveState {
                topology:           wgpu::PrimitiveTopology::TriangleList,
                cull_mode:          Some(wgpu::Face::Front), // front-face cull reduces peter-panning
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format:              wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare:       wgpu::CompareFunction::Less,
                stencil:             Default::default(),
                bias:                wgpu::DepthBiasState {
                    constant:   2,
                    slope_scale: 1.0,
                    clamp:       0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview:   None,
            cache:       None,
        });

        self.pipeline     = Some(pipeline);
        self.cam_bgl      = Some(cam_bgl);
        self.model_bgl    = Some(model_bgl);
        self.cam_buffer   = Some(cam_buffer);
        self.model_buffer = Some(model_buffer);
        self.model_stride = stride;
        self.cam_bg       = Some(cam_bg);
        self.model_bg     = Some(model_bg);
    }

    fn execute(&mut self, ctx: &mut RenderContext) {
        // Skip if no shadow-casting light this frame.
        if !ctx.frame.has_shadow_caster { return; }

        let pipeline   = match self.pipeline.as_ref()   { Some(p) => p, None => return };
        let cam_buf    = match self.cam_buffer.as_ref()  { Some(b) => b, None => return };
        let model_buf  = match self.model_buffer.as_ref(){ Some(b) => b, None => return };
        let cam_bg     = match self.cam_bg.as_ref()      { Some(g) => g, None => return };
        let model_bg   = match self.model_bg.as_ref()    { Some(g) => g, None => return };
        let stride     = self.model_stride;

        // Upload light-space camera UBO.
        ctx.queue.write_buffer(cam_buf, 0, bytemuck::bytes_of(&ShadowCamera {
            light_view_proj: ctx.frame.shadow_view_proj.to_cols_array_2d(),
        }));

        // Upload model matrices for shadow-casting draws.
        let raw = std::mem::size_of::<ModelUniforms>();
        let shadow_draws: Vec<_> = ctx.frame.draw_calls.iter()
            .filter(|d| d.cast_shadow)
            .take(MAX_DRAWS)
            .collect();

        let mut staging = vec![0u8; stride as usize * shadow_draws.len().max(1)];
        for (i, draw) in shadow_draws.iter().enumerate() {
            let data = ModelUniforms {
                world_matrix:  draw.world_matrix.to_cols_array_2d(),
                normal_matrix: draw.normal_matrix.to_cols_array_2d(),
            };
            let off = i * stride as usize;
            staging[off..off + raw].copy_from_slice(bytemuck::bytes_of(&data));
        }
        if !shadow_draws.is_empty() {
            ctx.queue.write_buffer(model_buf, 0,
                &staging[..shadow_draws.len() * stride as usize]);
        }

        // Begin depth-only render pass writing into shadow_map.
        let mut rpass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label:             Some("shadow_pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &ctx.resources.shadow_map.view,
                depth_ops: Some(wgpu::Operations {
                    load:  wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        rpass.set_pipeline(pipeline);
        rpass.set_bind_group(0, cam_bg, &[]);

        for (i, draw) in shadow_draws.iter().enumerate() {
            let mesh = match ctx.meshes.get(draw.mesh) { Some(m) => m, None => continue };
            let dynamic_offset = (i as u64 * stride) as u32;
            rpass.set_bind_group(1, model_bg, &[dynamic_offset]);
            rpass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            rpass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            rpass.draw_indexed(0..mesh.index_count, 0, 0..1);
        }
    }
}
