// ============================================================
// fluxion-renderer — SkinnedGeometryPass
//
// GBuffer fill pass for skinned (skeletal) meshes.
// Identical to GeometryPass except:
//   - Uses SkinnedVertex layout (adds joint indices + weights).
//   - Uses skinned_geometry.vert.wgsl for GPU skinning.
//   - Binds a per-draw joint matrix buffer at group(4).
//   - Reads from FrameData::skinned_draw_calls.
// ============================================================

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

use crate::render_graph::{RenderPass, RenderContext, RenderResources};
use crate::mesh::SkinnedVertex;
use crate::shader::library as shaders;

const MAX_SKINNED_DRAWS: usize = 256;
const MAX_JOINTS:        usize = 128;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ModelUniforms {
    world_matrix:  [[f32; 4]; 4],
    normal_matrix: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniforms {
    view_proj:       [[f32; 4]; 4],
    camera_position: [f32; 3],
    _pad:            f32,
}

/// Joint matrix buffer: 128 mat4x4 = 128 * 64 = 8192 bytes.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct JointBuffer {
    joints: [[[f32; 4]; 4]; MAX_JOINTS],
}

pub struct SkinnedGeometryPass {
    pipeline:          Option<wgpu::RenderPipeline>,
    camera_bgl:        Option<wgpu::BindGroupLayout>,
    model_bgl:         Option<wgpu::BindGroupLayout>,
    joint_bgl:         Option<wgpu::BindGroupLayout>,
    camera_buffer:     Option<wgpu::Buffer>,
    model_buffer:      Option<wgpu::Buffer>,
    model_stride:      u64,
    camera_bind_group: Option<wgpu::BindGroup>,
    model_bind_group:  Option<wgpu::BindGroup>,
    /// Per-draw joint UBO (re-uploaded each skinned draw call).
    joint_buffer:      Option<wgpu::Buffer>,
    joint_bind_group:  Option<wgpu::BindGroup>,
}

impl SkinnedGeometryPass {
    pub fn new() -> Self {
        Self {
            pipeline: None, camera_bgl: None, model_bgl: None, joint_bgl: None,
            camera_buffer: None, model_buffer: None, model_stride: 0,
            camera_bind_group: None, model_bind_group: None,
            joint_buffer: None, joint_bind_group: None,
        }
    }
}

impl RenderPass for SkinnedGeometryPass {
    fn name(&self) -> &str { "skinned_geometry" }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn prepare(&mut self, device: &wgpu::Device, _resources: &RenderResources) {
        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let raw   = std::mem::size_of::<ModelUniforms>() as u64;
        let stride = (raw + align - 1) / align * align;

        // ── Camera BGL ────────────────────────────────────────────────────────
        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("sk_geom_camera_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false, min_binding_size: None,
                }, count: None,
            }],
        });

        // ── Model BGL (dynamic offset) ────────────────────────────────────────
        let model_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("sk_geom_model_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty:                 wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size:   std::num::NonZeroU64::new(raw),
                }, count: None,
            }],
        });

        // ── Material BGL (same as geometry pass — 9 entries) ──────────────────
        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding, visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2, multisampled: false,
            }, count: None,
        };
        let samp_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding, visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None,
        };
        let material_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("sk_geom_material_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false, min_binding_size: None,
                    }, count: None,
                },
                tex_entry(1), samp_entry(2), tex_entry(3), samp_entry(4),
                tex_entry(5), samp_entry(6), tex_entry(7), samp_entry(8),
            ],
        });

        // ── Joint BGL (group 3) ───────────────────────────────────────────────
        let joint_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("sk_geom_joint_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false, min_binding_size: None,
                }, count: None,
            }],
        });

        // ── Buffers ───────────────────────────────────────────────────────────
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sk_geom_camera_ubo"),
            size:  std::mem::size_of::<CameraUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let model_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sk_geom_model_ubo"),
            size:  stride * MAX_SKINNED_DRAWS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let joint_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sk_geom_joint_ubo"),
            size:  std::mem::size_of::<JointBuffer>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Bind groups ───────────────────────────────────────────────────────
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sk_geom_camera_bg"), layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() }],
        });
        let model_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sk_geom_model_bg"), layout: &model_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &model_buffer, offset: 0, size: std::num::NonZeroU64::new(raw),
                }),
            }],
        });
        let joint_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sk_geom_joint_bg"), layout: &joint_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: joint_buffer.as_entire_binding() }],
        });

        // ── Shaders ───────────────────────────────────────────────────────────
        let vert_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("sk_geom_vert"),
            source: wgpu::ShaderSource::Wgsl(shaders::SKINNED_GEOMETRY_VERT.into()),
        });
        let frag_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("sk_geom_frag"),
            source: wgpu::ShaderSource::Wgsl(shaders::GEOMETRY_FRAG.into()),
        });

        // ── Pipeline ─────────────────────────────────────────────────────────
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sk_geom_layout"),
            bind_group_layouts:   &[&camera_bgl, &model_bgl, &material_bgl, &joint_bgl],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("sk_geom_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vert_module, entry_point: "vs_main",
                buffers: &[SkinnedVertex::layout()],
                compilation_options: Default::default(),
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
        self.joint_bgl         = Some(joint_bgl);
        self.camera_buffer     = Some(camera_buffer);
        self.model_buffer      = Some(model_buffer);
        self.model_stride      = stride;
        self.camera_bind_group = Some(camera_bind_group);
        self.model_bind_group  = Some(model_bind_group);
        self.joint_buffer      = Some(joint_buffer);
        self.joint_bind_group  = Some(joint_bind_group);
    }

    fn execute(&mut self, ctx: &mut RenderContext) {
        if ctx.frame.skinned_draw_calls.is_empty() { return; }

        let pipeline   = match self.pipeline.as_ref()          { Some(p) => p, None => return };
        let camera_buf = match self.camera_buffer.as_ref()     { Some(b) => b, None => return };
        let model_buf  = match self.model_buffer.as_ref()      { Some(b) => b, None => return };
        let joint_buf  = match self.joint_buffer.as_ref()      { Some(b) => b, None => return };
        let camera_bg  = match self.camera_bind_group.as_ref() { Some(g) => g, None => return };
        let model_bg   = match self.model_bind_group.as_ref()  { Some(g) => g, None => return };
        let joint_bg   = match self.joint_bind_group.as_ref()  { Some(g) => g, None => return };
        let stride     = self.model_stride;

        // Upload camera.
        let cam = &ctx.frame.camera;
        ctx.queue.write_buffer(camera_buf, 0, bytemuck::bytes_of(&CameraUniforms {
            view_proj:       cam.view_proj.to_cols_array_2d(),
            camera_position: cam.position.to_array(),
            _pad:            0.0,
        }));

        // Upload all model matrices.
        let raw = std::mem::size_of::<ModelUniforms>();
        let mut model_staging = vec![0u8; stride as usize * MAX_SKINNED_DRAWS];
        let n = ctx.frame.skinned_draw_calls.len().min(MAX_SKINNED_DRAWS);
        for (i, draw) in ctx.frame.skinned_draw_calls.iter().enumerate().take(n) {
            let data = ModelUniforms {
                world_matrix:  draw.world_matrix.to_cols_array_2d(),
                normal_matrix: draw.normal_matrix.to_cols_array_2d(),
            };
            let off = i * stride as usize;
            model_staging[off..off + raw].copy_from_slice(bytemuck::bytes_of(&data));
        }
        if n > 0 {
            ctx.queue.write_buffer(model_buf, 0, &model_staging[..n * stride as usize]);
        }

        // Begin MRT pass (Load — geometry pass already cleared the GBuffer).
        let res = ctx.resources;
        let mut rpass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("skinned_geometry_pass"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment { view: &res.gbuf_albedo_ao.view, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }}),
                Some(wgpu::RenderPassColorAttachment { view: &res.gbuf_normal.view, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }}),
                Some(wgpu::RenderPassColorAttachment { view: &res.gbuf_orm.view, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }}),
                Some(wgpu::RenderPassColorAttachment { view: &res.gbuf_emission.view, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }}),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &res.depth.view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        rpass.set_pipeline(pipeline);
        rpass.set_bind_group(0, camera_bg, &[]);

        for (i, draw) in ctx.frame.skinned_draw_calls.iter().enumerate().take(n) {
            let mesh     = match ctx.skinned_meshes.get(draw.skinned_mesh) { Some(m) => m, None => continue };
            let material = match ctx.materials.get(draw.material)          { Some(m) => m, None => continue };

            // Upload joint matrices for this draw call.
            let mut jbuf = JointBuffer { joints: [[[0.0; 4]; 4]; MAX_JOINTS] };
            for (ji, mat) in draw.joint_matrices.iter().enumerate().take(MAX_JOINTS) {
                jbuf.joints[ji] = mat.to_cols_array_2d();
            }
            // Fill remaining with identity.
            for ji in draw.joint_matrices.len()..MAX_JOINTS {
                jbuf.joints[ji] = Mat4::IDENTITY.to_cols_array_2d();
            }
            ctx.queue.write_buffer(joint_buf, 0, bytemuck::bytes_of(&jbuf));

            let dynamic_offset = (i as u64 * stride) as u32;
            rpass.set_bind_group(1, model_bg, &[dynamic_offset]);
            rpass.set_bind_group(2, &material.bind_group, &[]);
            rpass.set_bind_group(3, joint_bg, &[]);
            rpass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            rpass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            rpass.draw_indexed(0..mesh.index_count, 0, 0..1);
        }
    }
}

impl Default for SkinnedGeometryPass { fn default() -> Self { Self::new() } }
