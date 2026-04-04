// ============================================================
// fluxion-renderer — SkyboxPass (procedural gradient sky)
//
// Renders only sky pixels (depth = far plane). The gradient and sun
// direction are configurable at runtime.
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
}

impl SkyboxPass {
    pub fn new() -> Self {
        Self { params: SkyParams::default(), pipeline: None, bgl: None,
               bind_group: None, sky_buf: None, camera_buf: None }
    }
}

impl RenderPass for SkyboxPass {
    fn name(&self) -> &str { "skybox" }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn prepare(&mut self, device: &wgpu::Device, _resources: &RenderResources) {
        use wgpu::util::DeviceExt;

        let vert = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sky_vert"), source: wgpu::ShaderSource::Wgsl(shaders::FULLSCREEN_VERT.into()) });
        let frag = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sky_frag"), source: wgpu::ShaderSource::Wgsl(shaders::SKYBOX.into()) });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky_bgl"),
            entries: &[
                // binding 0: SkyParams
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                // binding 1: CameraUniforms
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                // binding 2: depth texture (for sky mask)
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            ],
        });

        let sky_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sky_params_buf"), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            contents: bytemuck::bytes_of(&self.params),
        });

        let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky_camera_buf"), size: 144, // mat4x4 * 2 + vec3 + pad
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None, bind_group_layouts: &[&bgl], push_constant_ranges: &[] });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky_pipeline"), layout: Some(&layout),
            vertex: wgpu::VertexState { module: &vert, entry_point: "vs_main", buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &frag, entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba16Float, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default() }),
            primitive: wgpu::PrimitiveState::default(), depth_stencil: None,
            multisample: wgpu::MultisampleState::default(), multiview: None, cache: None,
        });

        self.bgl      = Some(bgl);
        self.sky_buf  = Some(sky_buf);
        self.camera_buf = Some(camera_buf);
        self.pipeline = Some(pipeline);
    }

    fn resize(&mut self, _device: &wgpu::Device, _w: u32, _h: u32) {
        // depth texture was recreated on resize — invalidate cached bind group
        self.bind_group = None;
    }

    fn execute(&mut self, ctx: &mut RenderContext) {
        // Lazy-build bind group (depth.view is recreated on resize)
        if self.bind_group.is_none() {
            if let (Some(bgl), Some(sky_buf), Some(camera_buf)) =
                (self.bgl.as_ref(), self.sky_buf.as_ref(), self.camera_buf.as_ref())
            {
                self.bind_group = Some(ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("sky_bg"), layout: bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: sky_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: camera_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&ctx.resources.depth.view) },
                    ],
                }));
            }
        }

        let pipeline   = match self.pipeline.as_ref()   { Some(p) => p, None => return };
        let sky_buf    = match self.sky_buf.as_ref()     { Some(b) => b, None => return };
        let camera_buf = match self.camera_buf.as_ref()  { Some(b) => b, None => return };
        let bind_group = match self.bind_group.as_ref()  { Some(g) => g, None => return };

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
