// ============================================================
// fluxion-renderer — SsaoPass
//
// Screen-Space Ambient Occlusion using the ssao.wgsl shader.
//
// Pipeline:
//   1. SSAO raw: sample 32-point hemisphere kernel, write to ssao_raw
//   2. SSAO blur: 4x4 box blur, write to ssao_blur
//
// The result (ssao_blur) is read by the lighting pass (multiply into ambient).
//
// Bind group layout matches ssao.wgsl exactly:
//   group(0): SsaoParams + gbuf_normal + gbuf_depth + noise_tex + sampler
//   group(1): CameraUniforms
//
// All GPU objects are pre-allocated in prepare(); execute() only calls
// write_buffer() — zero GPU allocations per frame.
// ============================================================

use bytemuck::{Pod, Zeroable};
use crate::render_graph::{RenderPass, RenderContext, RenderResources};
use crate::shader::library as shaders;

const SAMPLE_COUNT: usize = 32;
const NOISE_SIZE:   u32   = 4;

/// Must match SsaoParams in ssao.wgsl exactly.
/// Total size: 4 + 4 + 4 + 4 (header) + 32 * 16 (samples) = 16 + 512 = 528 bytes.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SsaoParamsGpu {
    radius:    f32,
    bias:      f32,
    intensity: f32,
    _pad:      f32,
    samples:   [[f32; 4]; SAMPLE_COUNT],  // xyz = direction, w = unused
}

/// Camera uniforms — must match CameraUniforms in ssao.wgsl.
/// view_proj (64) + inv_view_proj (64) + proj (64) + inv_proj (64) + camera_pos (12) + pad (4) = 272 bytes.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniformsGpu {
    view_proj:     [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    proj:          [[f32; 4]; 4],
    inv_proj:      [[f32; 4]; 4],
    camera_pos:    [f32; 3],
    _pad:          f32,
}

pub struct SsaoPass {
    pub enabled: bool,
    pub radius:    f32,
    pub bias:      f32,
    pub intensity: f32,

    ssao_pipeline: Option<wgpu::RenderPipeline>,
    blur_pipeline: Option<wgpu::RenderPipeline>,

    ssao_params_bgl:  Option<wgpu::BindGroupLayout>,
    ssao_camera_bgl:  Option<wgpu::BindGroupLayout>,
    blur_bgl:         Option<wgpu::BindGroupLayout>,

    params_buf:   Option<wgpu::Buffer>,
    camera_buf:   Option<wgpu::Buffer>,
    noise_tex:      Option<wgpu::Texture>,
    noise_view:     Option<wgpu::TextureView>,
    noise_uploaded: bool,
    sampler:        Option<wgpu::Sampler>,

    ssao_params_bg:   Option<wgpu::BindGroup>,
    ssao_camera_bg:   Option<wgpu::BindGroup>,
    blur_bg:          Option<wgpu::BindGroup>,

    // Pre-generated hemisphere kernel (reused every frame)
    kernel: [[f32; 4]; SAMPLE_COUNT],
}

impl SsaoPass {
    pub fn new() -> Self {
        Self {
            enabled:  false,
            radius:   0.5,
            bias:     0.025,
            intensity: 1.5,

            ssao_pipeline: None,
            blur_pipeline: None,

            ssao_params_bgl: None,
            ssao_camera_bgl: None,
            blur_bgl:        None,

            params_buf:     None,
            camera_buf:     None,
            noise_tex:      None,
            noise_view:     None,
            noise_uploaded: false,
            sampler:        None,

            ssao_params_bg:  None,
            ssao_camera_bg:  None,
            blur_bg:         None,

            kernel: [[0.0; 4]; SAMPLE_COUNT],
        }
    }

    /// Generate a hemisphere sample kernel (in tangent space, pointing along +Z).
    /// Samples are distributed with an accelerating interpolation so more samples
    /// cluster near the origin, improving SSAO quality for short-range occlusion.
    fn generate_kernel() -> [[f32; 4]; SAMPLE_COUNT] {
        let mut kernel = [[0.0f32; 4]; SAMPLE_COUNT];
        // Deterministic LCG — no rand crate dependency
        let mut seed = 0x12345678u32;
        let mut rng = move || -> f32 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            ((seed >> 16) & 0xFFFF) as f32 / 65535.0
        };

        for i in 0..SAMPLE_COUNT {
            // Random hemisphere direction (z >= 0 → upper hemisphere)
            let x = rng() * 2.0 - 1.0;
            let y = rng() * 2.0 - 1.0;
            let z = rng();                      // [0, 1] → upper hemisphere only
            let len = (x*x + y*y + z*z).sqrt().max(1e-6);
            let (nx, ny, nz) = (x/len, y/len, z/len);

            // Accelerating scale — more samples near origin
            let scale = (i as f32) / (SAMPLE_COUNT as f32);
            let scale = 0.1 + scale * scale * 0.9;   // lerp(0.1, 1.0, scale²)
            let mag = rng().max(0.1) * scale;

            kernel[i] = [nx * mag, ny * mag, nz * mag, 0.0];
        }
        kernel
    }

    fn rebuild_bind_groups(&mut self, device: &wgpu::Device, resources: &RenderResources) {
        // SSAO params bind group: params + gbuf_normal + gbuf_depth + noise_tex + sampler
        if let (Some(bgl), Some(pbuf), Some(nview), Some(samp)) =
            (self.ssao_params_bgl.as_ref(), self.params_buf.as_ref(),
             self.noise_view.as_ref(), self.sampler.as_ref())
        {
            self.ssao_params_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ssao_params_bg"), layout: bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: pbuf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&resources.gbuf_normal.view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&resources.depth.view) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(nview) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(samp) },
                ],
            }));
        }

        // Camera bind group
        if let (Some(bgl), Some(cbuf)) = (self.ssao_camera_bgl.as_ref(), self.camera_buf.as_ref()) {
            self.ssao_camera_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ssao_camera_bg"), layout: bgl,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: cbuf.as_entire_binding() }],
            }));
        }

        // Blur bind group: ssao_raw + sampler
        if let (Some(bgl), Some(samp)) = (self.blur_bgl.as_ref(), self.sampler.as_ref()) {
            self.blur_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ssao_blur_bg"), layout: bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&resources.ssao_raw.view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(samp) },
                ],
            }));
        }
    }
}

impl RenderPass for SsaoPass {
    fn name(&self) -> &str { "ssao" }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn prepare(&mut self, device: &wgpu::Device, resources: &RenderResources) {
        use wgpu::util::DeviceExt;

        // Generate kernel once
        self.kernel = Self::generate_kernel();

        // Create noise texture (needs queue — store it; we create it during prepare)
        // We can't call queue here (prepare has no queue), so we defer noise upload
        // to execute() on first frame. For now create the texture without data.
        let noise_tex = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some("ssao_noise_tex"),
            size:            wgpu::Extent3d { width: NOISE_SIZE, height: NOISE_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          wgpu::TextureFormat::Rgba8Unorm,
            usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats:    &[],
        });
        let noise_view = noise_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:            Some("ssao_sampler"),
            address_mode_u:   wgpu::AddressMode::Repeat,
            address_mode_v:   wgpu::AddressMode::Repeat,
            address_mode_w:   wgpu::AddressMode::Repeat,
            mag_filter:       wgpu::FilterMode::Nearest,
            min_filter:       wgpu::FilterMode::Nearest,
            mipmap_filter:    wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // --- Bind group layouts ---
        let ssao_params_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ssao_params_bgl"),
            entries: &[
                // binding 0: SsaoParams
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                // binding 1: gbuf_normal (filterable float)
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                // binding 2: gbuf_depth (depth, non-filterable)
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                // binding 3: noise texture (filterable float)
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                // binding 4: sampler (filtering)
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });

        let ssao_camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ssao_camera_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });

        let blur_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ssao_blur_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });

        // --- Uniform buffers ---
        let params_data = SsaoParamsGpu {
            radius: self.radius, bias: self.bias, intensity: self.intensity, _pad: 0.0,
            samples: self.kernel,
        };
        let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ssao_params_buf"),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            contents: bytemuck::bytes_of(&params_data),
        });

        let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ssao_camera_buf"),
            size:  std::mem::size_of::<CameraUniformsGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // --- Shaders + pipelines ---
        let vert = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ssao_fullscreen_vert"), source: wgpu::ShaderSource::Wgsl(shaders::FULLSCREEN_VERT.into()) });

        let ssao_frag = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ssao_frag"), source: wgpu::ShaderSource::Wgsl(shaders::SSAO.into()) });

        let blur_frag = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ssao_blur_frag"), source: wgpu::ShaderSource::Wgsl(shaders::SSAO_BLUR.into()) });

        let ssao_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ssao_layout"),
            bind_group_layouts: &[&ssao_params_bgl, &ssao_camera_bgl],
            push_constant_ranges: &[],
        });
        let blur_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ssao_blur_layout"),
            bind_group_layouts: &[&blur_bgl],
            push_constant_ranges: &[],
        });

        let ssao_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ssao_pipeline"), layout: Some(&ssao_layout),
            vertex: wgpu::VertexState { module: &vert, entry_point: "vs_main", buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &ssao_frag, entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default() }),
            primitive: wgpu::PrimitiveState::default(), depth_stencil: None,
            multisample: wgpu::MultisampleState::default(), multiview: None, cache: None,
        });

        let blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ssao_blur_pipeline"), layout: Some(&blur_layout),
            vertex: wgpu::VertexState { module: &vert, entry_point: "vs_main", buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &blur_frag, entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default() }),
            primitive: wgpu::PrimitiveState::default(), depth_stencil: None,
            multisample: wgpu::MultisampleState::default(), multiview: None, cache: None,
        });

        self.ssao_pipeline    = Some(ssao_pipeline);
        self.blur_pipeline    = Some(blur_pipeline);
        self.ssao_params_bgl  = Some(ssao_params_bgl);
        self.ssao_camera_bgl  = Some(ssao_camera_bgl);
        self.blur_bgl         = Some(blur_bgl);
        self.params_buf       = Some(params_buf);
        self.camera_buf       = Some(camera_buf);
        self.noise_tex        = Some(noise_tex);
        self.noise_view       = Some(noise_view);
        self.noise_uploaded   = false;
        self.sampler          = Some(sampler);

        self.rebuild_bind_groups(device, resources);
    }

    fn resize(&mut self, _device: &wgpu::Device, _w: u32, _h: u32) {
        // GBuffer and depth textures were recreated — invalidate bind groups
        self.ssao_params_bg = None;
        self.blur_bg        = None;
        // camera_bg doesn't depend on textures — keep it
    }

    fn execute(&mut self, ctx: &mut RenderContext) {
        if !self.enabled { return; }

        // Upload noise texture once on first frame (queue is only available in execute)
        if !self.noise_uploaded {
            if let Some(tex) = self.noise_tex.as_ref() {
                let n = (NOISE_SIZE * NOISE_SIZE) as usize;
                let mut data = vec![0u8; n * 4];
                let mut seed = 0xDEADBEEFu32;
                let mut rng = move || -> u8 {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    ((seed >> 16) & 0xFF) as u8
                };
                for i in 0..n {
                    data[i * 4 + 0] = rng();
                    data[i * 4 + 1] = rng();
                    data[i * 4 + 2] = 0;
                    data[i * 4 + 3] = 255;
                }
                ctx.queue.write_texture(
                    wgpu::ImageCopyTexture { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    &data,
                    wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(NOISE_SIZE * 4), rows_per_image: None },
                    wgpu::Extent3d { width: NOISE_SIZE, height: NOISE_SIZE, depth_or_array_layers: 1 },
                );
                self.noise_uploaded = true;
            }
        }

        // Lazy-rebuild bind groups after resize
        if self.ssao_params_bg.is_none() {
            self.rebuild_bind_groups(ctx.device, ctx.resources);
        }

        let ssao_pl = match self.ssao_pipeline.as_ref() { Some(p) => p, None => return };
        let blur_pl = match self.blur_pipeline.as_ref() { Some(p) => p, None => return };
        let params_bg = match self.ssao_params_bg.as_ref()  { Some(g) => g, None => return };
        let camera_bg = match self.ssao_camera_bg.as_ref()  { Some(g) => g, None => return };
        let blur_bg   = match self.blur_bg.as_ref()          { Some(g) => g, None => return };
        let params_buf = match self.params_buf.as_ref() { Some(b) => b, None => return };
        let camera_buf = match self.camera_buf.as_ref() { Some(b) => b, None => return };

        // Upload SSAO params
        let params_data = SsaoParamsGpu {
            radius: self.radius, bias: self.bias, intensity: self.intensity, _pad: 0.0,
            samples: self.kernel,
        };
        ctx.queue.write_buffer(params_buf, 0, bytemuck::bytes_of(&params_data));

        // Upload camera
        let cam = &ctx.frame.camera;
        ctx.queue.write_buffer(camera_buf, 0, bytemuck::bytes_of(&CameraUniformsGpu {
            view_proj:     cam.view_proj.to_cols_array_2d(),
            inv_view_proj: cam.inv_view_proj.to_cols_array_2d(),
            proj:          cam.projection.to_cols_array_2d(),
            inv_proj:      cam.inv_proj.to_cols_array_2d(),
            camera_pos:    cam.position.to_array(),
            _pad:          0.0,
        }));

        // ── Pass 1: SSAO raw ───────────────────────────────────────────────────
        {
            let mut rpass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ssao_raw_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &ctx.resources.ssao_raw.view, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None, ..Default::default()
            });
            rpass.set_pipeline(ssao_pl);
            rpass.set_bind_group(0, params_bg, &[]);
            rpass.set_bind_group(1, camera_bg, &[]);
            rpass.draw(0..3, 0..1);
        }

        // ── Pass 2: blur ───────────────────────────────────────────────────────
        {
            let mut rpass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ssao_blur_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &ctx.resources.ssao_blur.view, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None, ..Default::default()
            });
            rpass.set_pipeline(blur_pl);
            rpass.set_bind_group(0, blur_bg, &[]);
            rpass.draw(0..3, 0..1);
        }
    }
}

impl Default for SsaoPass { fn default() -> Self { Self::new() } }
