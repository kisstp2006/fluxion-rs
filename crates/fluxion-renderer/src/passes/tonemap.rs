// ============================================================
// fluxion-renderer — TonemapPass
//
// Final pass: ACES tonemap + gamma + vignette + grain.
// Reads hdr_main, writes directly to the surface texture.
// ============================================================

use bytemuck::{Pod, Zeroable};
use crate::render_graph::{RenderPass, RenderContext, RenderResources};
use crate::shader::library as shaders;

/// Configurable tonemapping parameters. Can be modified at runtime.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct TonemapConfig {
    pub exposure:             f32,
    pub vignette_intensity:   f32,
    pub vignette_roundness:   f32,
    pub chromatic_aberration: f32,
    pub film_grain:           f32,
    pub time:                 f32,
    pub _pad0:                f32,
    pub _pad1:                f32,
}

impl Default for TonemapConfig {
    fn default() -> Self {
        Self {
            exposure:             1.0,
            vignette_intensity:   0.3,
            vignette_roundness:   0.8,
            chromatic_aberration: 0.5,
            film_grain:           0.02,
            time:                 0.0,
            _pad0: 0.0, _pad1: 0.0,
        }
    }
}

pub struct TonemapPass {
    pub config:     TonemapConfig,
    pipeline:       Option<wgpu::RenderPipeline>,
    params_buf:     Option<wgpu::Buffer>,
    bind_group:     Option<wgpu::BindGroup>,
    bgl:            Option<wgpu::BindGroupLayout>,
    surface_format: wgpu::TextureFormat,
}

impl TonemapPass {
    pub fn new(surface_format: wgpu::TextureFormat) -> Self {
        Self { config: TonemapConfig::default(), pipeline: None,
               params_buf: None, bind_group: None, bgl: None, surface_format }
    }

    fn rebuild_bind_group(&mut self, device: &wgpu::Device, hdr_view: &wgpu::TextureView, hdr_sampler: &wgpu::Sampler) {
        let bgl = match self.bgl.as_ref() { Some(l) => l, None => return };
        let buf = match self.params_buf.as_ref() { Some(b) => b, None => return };
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tonemap_bg"), layout: bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(hdr_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(hdr_sampler) },
            ],
        });
        self.bind_group = Some(bg);
    }
}

impl RenderPass for TonemapPass {
    fn name(&self) -> &str { "tonemap" }

    fn prepare(&mut self, device: &wgpu::Device, resources: &RenderResources) {
        use wgpu::util::DeviceExt;

        let vert = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tonemap_vert"), source: wgpu::ShaderSource::Wgsl(shaders::FULLSCREEN_VERT.into()) });
        let frag = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tonemap_frag"), source: wgpu::ShaderSource::Wgsl(shaders::TONEMAP.into()) });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tonemap_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });

        let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tonemap_params"), contents: bytemuck::bytes_of(&self.config),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tonemap_layout"), bind_group_layouts: &[&bgl], push_constant_ranges: &[] });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tonemap_pipeline"), layout: Some(&layout),
            vertex: wgpu::VertexState { module: &vert, entry_point: "vs_main", buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &frag, entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState { format: self.surface_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default() }),
            primitive: wgpu::PrimitiveState::default(), depth_stencil: None,
            multisample: wgpu::MultisampleState::default(), multiview: None, cache: None,
        });

        self.bgl        = Some(bgl);
        self.params_buf = Some(params_buf);
        self.pipeline   = Some(pipeline);
        self.rebuild_bind_group(device, &resources.hdr_main.view, &resources.hdr_main.sampler);
    }

    fn resize(&mut self, _device: &wgpu::Device, _w: u32, _h: u32) {
        self.bind_group = None; // will be rebuilt on next execute via renderer
    }

    fn execute(&mut self, ctx: &mut RenderContext) {
        // Rebuild bind group after resize (hdr_main texture was recreated).
        if self.bind_group.is_none() {
            self.rebuild_bind_group(ctx.device, &ctx.resources.hdr_main.view, &ctx.resources.hdr_main.sampler);
        }

        let pipeline   = match self.pipeline.as_ref()    { Some(p) => p, None => return };
        let bind_group = match self.bind_group.as_ref()  { Some(g) => g, None => return };
        let params_buf = match self.params_buf.as_ref()  { Some(b) => b, None => return };

        // Update time for film grain animation
        self.config.time = ctx.frame.time;
        ctx.queue.write_buffer(params_buf, 0, bytemuck::bytes_of(&self.config));

        let mut rpass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("tonemap_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: ctx.surface_view, resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        rpass.set_pipeline(pipeline);
        rpass.set_bind_group(0, bind_group, &[]);
        rpass.draw(0..3, 0..1);
    }
}
