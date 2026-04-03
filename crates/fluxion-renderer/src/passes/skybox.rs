// ============================================================
// fluxion-renderer — SkyboxPass (procedural gradient sky)
//
// Renders only sky pixels (depth = far plane). The gradient and sun
// direction are configurable at runtime.
// ============================================================

use bytemuck::{Pod, Zeroable};
use crate::render_graph::{RenderPass, RenderContext, RenderResources};
use crate::shader::library as shaders;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct SkyParams {
    pub horizon_color: [f32; 3],
    pub _pad0:         f32,
    pub zenith_color:  [f32; 3],
    pub _pad1:         f32,
    pub sun_direction: [f32; 3],
    pub sun_intensity: f32,
    pub sun_size:      f32,
    pub _pad2:         f32,
    pub _pad3:         f32,
    pub _pad4:         f32,
}

impl Default for SkyParams {
    fn default() -> Self {
        Self {
            horizon_color: [0.6, 0.75, 1.0],
            zenith_color:  [0.1, 0.3, 0.8],
            sun_direction: [0.5, 0.8, 0.3],
            sun_intensity: 20.0,
            sun_size:      0.02,
            _pad0: 0.0, _pad1: 0.0, _pad2: 0.0, _pad3: 0.0, _pad4: 0.0,
        }
    }
}

pub struct SkyboxPass {
    pub params:   SkyParams,
    pipeline:     Option<wgpu::RenderPipeline>,
    bgl:          Option<wgpu::BindGroupLayout>,
    bind_group:   Option<wgpu::BindGroup>,
    sky_buf:      Option<wgpu::Buffer>,
    camera_buf:   Option<wgpu::Buffer>,
    surface_format: wgpu::TextureFormat,
}

impl SkyboxPass {
    pub fn new(surface_format: wgpu::TextureFormat) -> Self {
        Self { params: SkyParams::default(), pipeline: None, bgl: None,
               bind_group: None, sky_buf: None, camera_buf: None, surface_format }
    }
}

impl RenderPass for SkyboxPass {
    fn name(&self) -> &str { "skybox" }

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

    fn execute(&mut self, ctx: &mut RenderContext) {
        let pipeline   = match self.pipeline.as_ref()   { Some(p) => p, None => return };
        let sky_buf    = match self.sky_buf.as_ref()    { Some(b) => b, None => return };
        let camera_buf = match self.camera_buf.as_ref() { Some(b) => b, None => return };
        let bgl        = match self.bgl.as_ref()        { Some(l) => l, None => return };

        ctx.queue.write_buffer(sky_buf, 0, bytemuck::bytes_of(&self.params));

        // Upload camera inv_view_proj + position
        #[repr(C)] #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct CamData { vp: [[f32;4];4], ivp: [[f32;4];4], pos: [f32;3], _p: f32 }
        let cam = &ctx.frame.camera;
        let cam_data = CamData { vp: cam.view_proj.to_cols_array_2d(), ivp: cam.inv_view_proj.to_cols_array_2d(), pos: cam.position.to_array(), _p: 0.0 };
        ctx.queue.write_buffer(camera_buf, 0, bytemuck::bytes_of(&cam_data));

        let bg = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky_bg"), layout: bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: sky_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: camera_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&ctx.resources.depth.view) },
            ],
        });

        let mut rpass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("skybox_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &ctx.resources.hdr_main.view, resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None, ..Default::default()
        });
        rpass.set_pipeline(pipeline);
        rpass.set_bind_group(0, &bg, &[]);
        rpass.draw(0..3, 0..1);
    }
}
