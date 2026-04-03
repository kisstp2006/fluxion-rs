// ============================================================
// fluxion-renderer — BloomPass
//
// 1. Bright-pass: extract pixels above luminance threshold
// 2. 4× downsample-blur (Kawase) at half resolution
// 3. Composite: additive blend bloom back onto HDR target
// ============================================================

use bytemuck::{Pod, Zeroable};
use crate::render_graph::{RenderPass, RenderContext, RenderResources};
use crate::shader::library as shaders;

#[repr(C)] #[derive(Clone, Copy, Pod, Zeroable)]
struct BrightParams { threshold: f32, soft_knee: f32, _p0: f32, _p1: f32 }

#[repr(C)] #[derive(Clone, Copy, Pod, Zeroable)]
struct BlurParams { iteration: u32, _p0: u32, _p1: u32, _p2: u32 }

#[repr(C)] #[derive(Clone, Copy, Pod, Zeroable)]
struct CompositeParams { strength: f32, _p0: f32, _p1: f32, _p2: f32 }

/// Bloom configuration. Modify at runtime to tune the effect.
pub struct BloomConfig {
    pub enabled:    bool,
    pub threshold:  f32,
    pub soft_knee:  f32,
    pub strength:   f32,
    pub blur_passes: u32,  // number of blur iterations (default 4)
}

impl Default for BloomConfig {
    fn default() -> Self {
        Self { enabled: true, threshold: 0.8, soft_knee: 0.5, strength: 0.4, blur_passes: 4 }
    }
}

pub struct BloomPass {
    pub config:          BloomConfig,
    bright_pipeline:     Option<wgpu::RenderPipeline>,
    blur_pipeline:       Option<wgpu::RenderPipeline>,
    composite_pipeline:  Option<wgpu::RenderPipeline>,
    bright_bgl:          Option<wgpu::BindGroupLayout>,
    blur_bgl:            Option<wgpu::BindGroupLayout>,
    composite_bgl:       Option<wgpu::BindGroupLayout>,
}

impl BloomPass {
    pub fn new() -> Self {
        Self {
            config:             BloomConfig::default(),
            bright_pipeline:    None, blur_pipeline:     None, composite_pipeline: None,
            bright_bgl:         None, blur_bgl:           None, composite_bgl:      None,
        }
    }

    fn make_bgl_tex_samp(device: &wgpu::Device, label: &str, extra_uniform: bool) -> wgpu::BindGroupLayout {
        let mut entries = Vec::new();
        if extra_uniform {
            entries.push(wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None });
        }
        let base = if extra_uniform { 1u32 } else { 0 };
        entries.push(wgpu::BindGroupLayoutEntry { binding: base, visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None });
        entries.push(wgpu::BindGroupLayoutEntry { binding: base+1, visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None });
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries: &entries })
    }

    fn fullscreen_pipeline(device: &wgpu::Device, frag_src: &str, frag_label: &str, bgl: &wgpu::BindGroupLayout, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let vert = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("bloom_vert"), source: wgpu::ShaderSource::Wgsl(shaders::FULLSCREEN_VERT.into()) });
        let frag = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(frag_label), source: wgpu::ShaderSource::Wgsl(frag_src.into()) });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[bgl], push_constant_ranges: &[] });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(frag_label), layout: Some(&layout),
            vertex: wgpu::VertexState { module: &vert, entry_point: "vs_main", buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &frag, entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default() }),
            primitive: wgpu::PrimitiveState::default(), depth_stencil: None,
            multisample: wgpu::MultisampleState::default(), multiview: None, cache: None,
        })
    }
}

impl RenderPass for BloomPass {
    fn name(&self) -> &str { "bloom" }

    fn prepare(&mut self, device: &wgpu::Device, _resources: &RenderResources) {
        let hdr = wgpu::TextureFormat::Rgba16Float;
        let bright_bgl    = Self::make_bgl_tex_samp(device, "bloom_bright_bgl", true);
        let blur_bgl      = Self::make_bgl_tex_samp(device, "bloom_blur_bgl",   true);
        // Composite needs: uniform + 2 textures + 2 samplers
        let composite_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom_composite_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });

        self.bright_pipeline   = Some(Self::fullscreen_pipeline(device, shaders::BLOOM_BRIGHT,    "bloom_bright",    &bright_bgl, hdr));
        self.blur_pipeline     = Some(Self::fullscreen_pipeline(device, shaders::BLOOM_BLUR,      "bloom_blur",      &blur_bgl,   hdr));
        self.composite_pipeline = Some(Self::fullscreen_pipeline(device, shaders::BLOOM_COMPOSITE, "bloom_composite", &composite_bgl, hdr));

        self.bright_bgl    = Some(bright_bgl);
        self.blur_bgl      = Some(blur_bgl);
        self.composite_bgl = Some(composite_bgl);
    }

    fn execute(&mut self, ctx: &mut RenderContext) {
        if !self.config.enabled { return; }
        let res = ctx.resources;

        // Helper: run a fullscreen pass into a target
        let blit_pass = |encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::RenderPipeline,
                          bind_group: &wgpu::BindGroup, target: &wgpu::TextureView, label: &str| {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None, ..Default::default()
            });
            rp.set_pipeline(pipeline);
            rp.set_bind_group(0, bind_group, &[]);
            rp.draw(0..3, 0..1);
        };

        // All pipelines required — skip pass if not ready
        let bright_pl = match self.bright_pipeline.as_ref()    { Some(p) => p, None => return };
        let blur_pl   = match self.blur_pipeline.as_ref()      { Some(p) => p, None => return };
        let comp_pl   = match self.composite_pipeline.as_ref() { Some(p) => p, None => return };
        let bright_bgl = match self.bright_bgl.as_ref()     { Some(l) => l, None => return };
        let blur_bgl   = match self.blur_bgl.as_ref()        { Some(l) => l, None => return };
        let comp_bgl   = match self.composite_bgl.as_ref()   { Some(l) => l, None => return };

        use wgpu::util::DeviceExt;

        // ── Step 1: bright-pass ────────────────────────────────────────────────
        let bright_params_buf = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bp_bright"), usage: wgpu::BufferUsages::UNIFORM,
            contents: bytemuck::bytes_of(&BrightParams { threshold: self.config.threshold, soft_knee: self.config.soft_knee, _p0: 0.0, _p1: 0.0 }),
        });
        let bright_bg = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom_bright_bg"), layout: bright_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: bright_params_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&res.hdr_main.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&res.hdr_main.sampler) },
            ],
        });
        blit_pass(ctx.encoder, bright_pl, &bright_bg, &res.bloom_bright.view, "bloom_bright");

        // ── Step 2: blur iterations (ping-pong between bloom_blur_a / bloom_blur_b) ─
        let mut src = &res.bloom_bright;
        let mut dst = &res.bloom_blur_a;
        for i in 0..self.config.blur_passes {
            let blur_params_buf = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("bp_blur"), usage: wgpu::BufferUsages::UNIFORM,
                contents: bytemuck::bytes_of(&BlurParams { iteration: i, _p0: 0, _p1: 0, _p2: 0 }),
            });
            let blur_bg = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom_blur_bg"), layout: blur_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: blur_params_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&src.view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&src.sampler) },
                ],
            });
            blit_pass(ctx.encoder, blur_pl, &blur_bg, &dst.view, "bloom_blur");
            std::mem::swap(&mut src, &mut dst);
        }
        // After the loop, `src` holds the final blurred result

        // ── Step 3: composite bloom onto HDR target ────────────────────────────
        let comp_params_buf = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bp_comp"), usage: wgpu::BufferUsages::UNIFORM,
            contents: bytemuck::bytes_of(&CompositeParams { strength: self.config.strength, _p0: 0.0, _p1: 0.0, _p2: 0.0 }),
        });
        let comp_bg = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom_composite_bg"), layout: comp_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: comp_params_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&res.hdr_main.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&src.view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&res.hdr_main.sampler) },
            ],
        });
        blit_pass(ctx.encoder, comp_pl, &comp_bg, &res.hdr_ping.view, "bloom_composite");

        // Copy hdr_ping back to hdr_main (ping-pong result)
        ctx.encoder.copy_texture_to_texture(
            wgpu::ImageCopyTexture { texture: &res.hdr_ping.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::ImageCopyTexture { texture: &res.hdr_main.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: res.hdr_main.width, height: res.hdr_main.height, depth_or_array_layers: 1 },
        );
    }
}

impl Default for BloomPass { fn default() -> Self { Self::new() } }
