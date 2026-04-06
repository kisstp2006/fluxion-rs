// ============================================================
// fluxion-renderer — BloomPass
//
// 1. Bright-pass: extract pixels above luminance threshold
// 2. N× downsample-blur (Kawase) at half resolution
// 3. Composite: additive blend bloom back onto HDR target
//
// All uniform buffers are pre-allocated in prepare() and updated
// each frame via queue.write_buffer() — zero GPU allocations per frame.
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
    pub enabled:     bool,
    pub threshold:   f32,
    pub soft_knee:   f32,
    pub strength:    f32,
    pub blur_passes: u32,   // number of blur iterations (max MAX_BLUR_PASSES, default 4)
}

const MAX_BLUR_PASSES: usize = 8;

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

    // Pre-allocated uniform buffers (COPY_DST)
    bright_params_buf:   Option<wgpu::Buffer>,
    blur_params_bufs:    Vec<wgpu::Buffer>,    // one per MAX_BLUR_PASSES
    comp_params_buf:     Option<wgpu::Buffer>,

    // Cached bind groups — rebuilt lazily after resize
    bright_bg:     Option<wgpu::BindGroup>,
    blur_bgs:      Vec<Option<wgpu::BindGroup>>,  // [pass][ping/pong] — two alternating per pass
    comp_bg:       Option<wgpu::BindGroup>,

    bright_bgl:    Option<wgpu::BindGroupLayout>,
    blur_bgl:      Option<wgpu::BindGroupLayout>,
    composite_bgl: Option<wgpu::BindGroupLayout>,
}

impl BloomPass {
    pub fn new() -> Self {
        Self {
            config:             BloomConfig::default(),
            bright_pipeline:    None,
            blur_pipeline:      None,
            composite_pipeline: None,
            bright_params_buf:  None,
            blur_params_bufs:   Vec::new(),
            comp_params_buf:    None,
            bright_bg:          None,
            blur_bgs:           Vec::new(),
            comp_bg:            None,
            bright_bgl:         None,
            blur_bgl:           None,
            composite_bgl:      None,
        }
    }

    fn make_bgl_tex_samp(device: &wgpu::Device, label: &str, has_uniform: bool) -> wgpu::BindGroupLayout {
        let mut entries = Vec::new();
        if has_uniform {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            });
        }
        let base = if has_uniform { 1u32 } else { 0 };
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: base, visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        });
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: base + 1, visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        });
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries: &entries })
    }

    fn fullscreen_pipeline(device: &wgpu::Device, frag_src: &str, frag_label: &str, bgl: &wgpu::BindGroupLayout, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let vert = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("bloom_vert"), source: wgpu::ShaderSource::Wgsl(shaders::FULLSCREEN_VERT.into()) });
        let frag = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(frag_label), source: wgpu::ShaderSource::Wgsl(frag_src.into()) });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(bgl)], immediate_size: 0 });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(frag_label), layout: Some(&layout),
            vertex: wgpu::VertexState { module: &vert, entry_point: Some("vs_main"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &frag, entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default() }),
            primitive: wgpu::PrimitiveState::default(), depth_stencil: None,
            multisample: wgpu::MultisampleState::default(), multiview_mask: None, cache: None,
        })
    }

    fn rebuild_bind_groups(&mut self, device: &wgpu::Device, resources: &RenderResources) {
        // Bright-pass bind group: uniform + hdr_main
        if let (Some(bgl), Some(buf)) = (self.bright_bgl.as_ref(), self.bright_params_buf.as_ref()) {
            self.bright_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom_bright_bg"), layout: bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&resources.hdr_main.view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&resources.hdr_main.sampler) },
                ],
            }));
        }

        // Blur bind groups — alternate between bloom_bright → bloom_blur_a → bloom_blur_b → ...
        self.blur_bgs.clear();
        if let Some(bgl) = self.blur_bgl.as_ref() {
            let textures = [&resources.bloom_bright, &resources.bloom_blur_a, &resources.bloom_blur_b];
            for i in 0..MAX_BLUR_PASSES {
                if i < self.blur_params_bufs.len() {
                    let src = &textures[if i == 0 { 0 } else { 1 + (i - 1) % 2 }];
                    let buf = &self.blur_params_bufs[i];
                    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("bloom_blur_bg"), layout: bgl,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&src.view) },
                            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&src.sampler) },
                        ],
                    });
                    self.blur_bgs.push(Some(bg));
                } else {
                    self.blur_bgs.push(None);
                }
            }
        }

        // Composite bind group: uniform + hdr_main + final blur result
        // The final blur result after N passes (N even → bloom_blur_a, N odd → bloom_blur_b, 0 → bloom_bright)
        if let (Some(bgl), Some(buf)) = (self.composite_bgl.as_ref(), self.comp_params_buf.as_ref()) {
            let passes = self.config.blur_passes as usize;
            let final_blur = if passes == 0 {
                &resources.bloom_bright
            } else if passes % 2 == 1 {
                &resources.bloom_blur_a
            } else {
                &resources.bloom_blur_b
            };
            self.comp_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom_composite_bg"), layout: bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&resources.hdr_main.view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&final_blur.view) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&resources.hdr_main.sampler) },
                ],
            }));
        }
    }
}

impl RenderPass for BloomPass {
    fn name(&self) -> &str { "bloom" }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn prepare(&mut self, device: &wgpu::Device, resources: &RenderResources) {
        use wgpu::util::DeviceExt;

        let hdr = wgpu::TextureFormat::Rgba16Float;
        let bright_bgl    = Self::make_bgl_tex_samp(device, "bloom_bright_bgl", true);
        let blur_bgl      = Self::make_bgl_tex_samp(device, "bloom_blur_bgl",   true);
        // Composite: uniform + 2 textures + 1 sampler
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

        self.bright_pipeline    = Some(Self::fullscreen_pipeline(device, shaders::BLOOM_BRIGHT,    "bloom_bright",    &bright_bgl,    hdr));
        self.blur_pipeline      = Some(Self::fullscreen_pipeline(device, shaders::BLOOM_BLUR,      "bloom_blur",      &blur_bgl,      hdr));
        self.composite_pipeline = Some(Self::fullscreen_pipeline(device, shaders::BLOOM_COMPOSITE, "bloom_composite", &composite_bgl, hdr));

        // Pre-allocate uniform buffers (COPY_DST so we can update each frame)
        self.bright_params_buf = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bloom_bright_params"),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            contents: bytemuck::bytes_of(&BrightParams { threshold: self.config.threshold, soft_knee: self.config.soft_knee, _p0: 0.0, _p1: 0.0 }),
        }));

        self.blur_params_bufs.clear();
        for i in 0..MAX_BLUR_PASSES {
            self.blur_params_bufs.push(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("bloom_blur_params"),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                contents: bytemuck::bytes_of(&BlurParams { iteration: i as u32, _p0: 0, _p1: 0, _p2: 0 }),
            }));
        }

        self.comp_params_buf = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bloom_comp_params"),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            contents: bytemuck::bytes_of(&CompositeParams { strength: self.config.strength, _p0: 0.0, _p1: 0.0, _p2: 0.0 }),
        }));

        self.bright_bgl    = Some(bright_bgl);
        self.blur_bgl      = Some(blur_bgl);
        self.composite_bgl = Some(composite_bgl);

        self.rebuild_bind_groups(device, resources);
    }

    fn resize(&mut self, _device: &wgpu::Device, _w: u32, _h: u32) {
        // Textures were recreated — invalidate all bind groups
        self.bright_bg = None;
        self.blur_bgs.iter_mut().for_each(|bg| *bg = None);
        self.comp_bg   = None;
    }

    fn execute(&mut self, ctx: &mut RenderContext) {
        if !self.config.enabled { return; }

        // Lazy-rebuild bind groups after resize
        if self.bright_bg.is_none() {
            self.rebuild_bind_groups(ctx.device, ctx.resources);
        }

        let res = ctx.resources;

        let bright_pl = match self.bright_pipeline.as_ref()    { Some(p) => p, None => return };
        let blur_pl   = match self.blur_pipeline.as_ref()      { Some(p) => p, None => return };
        let comp_pl   = match self.composite_pipeline.as_ref() { Some(p) => p, None => return };
        let bright_bg = match self.bright_bg.as_ref()          { Some(g) => g, None => return };
        let comp_bg   = match self.comp_bg.as_ref()            { Some(g) => g, None => return };

        // Update uniforms via write_buffer (no allocations)
        if let Some(buf) = self.bright_params_buf.as_ref() {
            ctx.queue.write_buffer(buf, 0, bytemuck::bytes_of(&BrightParams {
                threshold: self.config.threshold, soft_knee: self.config.soft_knee, _p0: 0.0, _p1: 0.0,
            }));
        }
        if let Some(buf) = self.comp_params_buf.as_ref() {
            ctx.queue.write_buffer(buf, 0, bytemuck::bytes_of(&CompositeParams {
                strength: self.config.strength, _p0: 0.0, _p1: 0.0, _p2: 0.0,
            }));
        }

        let blur_targets = [&res.bloom_blur_a.view, &res.bloom_blur_b.view];

        // Helper: fullscreen blit pass
        let blit_pass = |encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::RenderPipeline,
                          bind_group: &wgpu::BindGroup, target: &wgpu::TextureView, label: &str| {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target, resolve_target: None, depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None, ..Default::default()
            });
            rp.set_pipeline(pipeline);
            rp.set_bind_group(0, bind_group, &[]);
            rp.draw(0..3, 0..1);
        };

        // ── Step 1: bright-pass ────────────────────────────────────────────────
        blit_pass(ctx.encoder, bright_pl, bright_bg, &res.bloom_bright.view, "bloom_bright");

        // ── Step 2: blur iterations ────────────────────────────────────────────
        let passes = self.config.blur_passes.min(MAX_BLUR_PASSES as u32) as usize;
        for i in 0..passes {
            if let Some(Some(bg)) = self.blur_bgs.get(i) {
                // Alternating output targets: pass 0 → blur_a, pass 1 → blur_b, etc.
                let dst = blur_targets[i % 2];
                blit_pass(ctx.encoder, blur_pl, bg, dst, "bloom_blur");
            }
        }

        // ── Step 3: composite bloom onto HDR target ────────────────────────────
        // Write result to hdr_ping, then copy back to hdr_main
        blit_pass(ctx.encoder, comp_pl, comp_bg, &res.hdr_ping.view, "bloom_composite");

        ctx.encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: &res.hdr_ping.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &res.hdr_main.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: res.hdr_main.width, height: res.hdr_main.height, depth_or_array_layers: 1 },
        );
    }
}

impl Default for BloomPass { fn default() -> Self { Self::new() } }
