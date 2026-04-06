// ============================================================
// fluxion-renderer — DofPass  (Depth of Field)
//
// Single-pass Poisson-disc blur driven by a per-pixel
// circle-of-confusion computed from the scene depth buffer.
//
// Pipeline:
//   Reads:  hdr_main (Rgba16Float) + depth (Depth32Float)
//   Writes: hdr_ping (Rgba16Float)
//
// The tonemap pass reads hdr_ping when DoF is enabled,
// hdr_main otherwise.
// ============================================================

use bytemuck::{Pod, Zeroable};
use crate::render_graph::{RenderPass, RenderContext, RenderResources};
use crate::shader::library as shaders;

/// Must match `DofParams` in dof.wgsl exactly (32 bytes).
///
/// Byte layout:
///   0: focus_distance f32 (4)
///   4: aperture       f32 (4)
///   8: max_blur       f32 (4)
///  12: near_plane     f32 (4)
///  16: far_plane      f32 (4)
///  20: _pad0          f32 (4)
///  24: _pad1          f32 (4)
///  28: _pad2          f32 (4)  → 32 bytes
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct DofParams {
    pub focus_distance: f32,
    pub aperture:       f32,
    pub max_blur:       f32,
    pub near_plane:     f32,
    pub far_plane:      f32,
    pub _pad0:          f32,
    pub _pad1:          f32,
    pub _pad2:          f32,
}

const _: () = assert!(std::mem::size_of::<DofParams>() == 32);

impl Default for DofParams {
    fn default() -> Self {
        Self {
            focus_distance: 10.0,
            aperture:       0.0,   // 0 = disabled
            max_blur:       8.0,
            near_plane:     0.1,
            far_plane:      1000.0,
            _pad0: 0.0, _pad1: 0.0, _pad2: 0.0,
        }
    }
}

pub struct DofPass {
    pub enabled: bool,
    pub params:  DofParams,

    pipeline:   Option<wgpu::RenderPipeline>,
    params_buf: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    bgl:        Option<wgpu::BindGroupLayout>,
}

impl DofPass {
    pub fn new() -> Self {
        Self {
            enabled:    false,
            params:     DofParams::default(),
            pipeline:   None,
            params_buf: None,
            bind_group: None,
            bgl:        None,
        }
    }

    fn rebuild_bind_group(&mut self, device: &wgpu::Device, resources: &RenderResources) {
        let (Some(bgl), Some(buf)) = (self.bgl.as_ref(), self.params_buf.as_ref()) else { return };
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dof_bg"), layout: bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&resources.hdr_main.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&resources.depth.view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&resources.hdr_main.sampler) },
            ],
        }));
    }
}

impl RenderPass for DofPass {
    fn name(&self) -> &str { "dof" }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn prepare(&mut self, device: &wgpu::Device, resources: &RenderResources) {
        use wgpu::util::DeviceExt;

        let vert = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("dof_vert"),
            source: wgpu::ShaderSource::Wgsl(shaders::FULLSCREEN_VERT.into()),
        });
        let frag = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("dof_frag"),
            source: wgpu::ShaderSource::Wgsl(shaders::DOF.into()),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("dof_bgl"),
            entries: &[
                // binding 0 — DofParams uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false, min_binding_size: None,
                    }, count: None,
                },
                // binding 1 — hdr_tex (filterable float)
                wgpu::BindGroupLayoutEntry {
                    binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2, multisampled: false,
                    }, count: None,
                },
                // binding 2 — depth_tex (non-filterable depth)
                wgpu::BindGroupLayoutEntry {
                    binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2, multisampled: false,
                    }, count: None,
                },
                // binding 3 — sampler (filtering)
                wgpu::BindGroupLayoutEntry {
                    binding: 3, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("dof_params"),
            contents: bytemuck::bytes_of(&self.params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("dof_layout"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("dof_pipeline"), layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &vert, entry_point: Some("vs_main"), buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &frag, entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: None, write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None, cache: None,
        });

        self.bgl        = Some(bgl);
        self.params_buf = Some(params_buf);
        self.pipeline   = Some(pipeline);
        self.rebuild_bind_group(device, resources);
    }

    fn resize(&mut self, _device: &wgpu::Device, _w: u32, _h: u32) {
        self.bind_group = None;
    }

    fn execute(&mut self, ctx: &mut RenderContext) {
        if !self.enabled { return; }

        if self.bind_group.is_none() {
            self.rebuild_bind_group(ctx.device, ctx.resources);
        }

        let (Some(pipeline), Some(bind_group), Some(params_buf)) =
            (self.pipeline.as_ref(), self.bind_group.as_ref(), self.params_buf.as_ref())
        else { return };

        // Sync camera near/far into params so CoC linearisation is accurate
        self.params.near_plane = ctx.frame.camera.near;
        self.params.far_plane  = ctx.frame.camera.far;
        ctx.queue.write_buffer(params_buf, 0, bytemuck::bytes_of(&self.params));

        // Write blurred result into hdr_ping
        let mut rpass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("dof_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &ctx.resources.hdr_ping.view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load:  wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        rpass.set_pipeline(pipeline);
        rpass.set_bind_group(0, bind_group, &[]);
        rpass.draw(0..3, 0..1);
    }
}
