// ============================================================
// fluxion-renderer — RenderContext + RenderResources
//
// RenderContext: per-frame state passed to every RenderPass::execute().
//   - Borrowed device + queue (for occasional GPU object creation)
//   - Mutable CommandEncoder (all passes share one; submitted at frame end)
//   - Shared GBuffer textures (RenderResources)
//   - Per-frame CPU data (camera matrices, draw calls, lights)
//
// RenderResources: size-dependent GPU textures shared across passes.
//   - GBuffer attachments (geometry pass writes, lighting pass reads)
//   - Intermediate HDR targets for post-processing ping-pong
//   - Depth texture
//
// Design note: all passes share ONE CommandEncoder (instead of each
// pass creating and submitting its own). This is the correct wgpu
// pattern — submitting multiple small command buffers is more
// expensive than one large one due to synchronization overhead.
// ============================================================

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::TextureView;

use fluxion_core::{DebugLine, ClearFlags};

use crate::texture::GpuTexture;
use crate::lighting::LightUniform;
use crate::mesh::MeshRegistry;
use crate::material::MaterialRegistry;

/// Default shadow map resolution (pixels per side). Power of two.
pub const SHADOW_MAP_SIZE: u32 = 2048;

/// GPU layout for [`SkyboxPass`](crate::passes::SkyboxPass).
/// All offsets are manually verified for std140/WGSL alignment.
///
/// Byte layout (96 bytes total):
///   0: horizon_color  [f32;3]  (12)
///  12: sky_mode       u32      ( 4) → 16
///  16: zenith_color   [f32;3]  (12)
///  28: _pad1          f32      ( 4) → 32
///  32: sun_direction  [f32;3]  (12)
///  44: sun_intensity  f32      ( 4) → 48
///  48: sun_size       f32      ( 4)
///  52: _pad2a         f32      ( 4)
///  56: _pad2b         f32      ( 4)
///  60: _pad2c         f32      ( 4) → 64  (solid_color must be 16-byte aligned)
///  64: solid_color    [f32;3]  (12)
///  76: turbidity      f32      ( 4) → 80
///  80: rayleigh       f32      ( 4)
///  84: mie_coefficient f32     ( 4)
///  88: mie_directional_g f32   ( 4)
///  92: _pad3          f32      ( 4) → 96
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct SkyParams {
    pub horizon_color:      [f32; 3],
    pub sky_mode:           u32,
    pub zenith_color:       [f32; 3],
    pub _pad1:              f32,
    pub sun_direction:      [f32; 3],
    pub sun_intensity:      f32,
    pub sun_size:           f32,
    pub _pad2a:             f32,
    pub _pad2b:             f32,
    pub _pad2c:             f32,
    pub solid_color:        [f32; 3],
    pub turbidity:          f32,
    pub rayleigh:           f32,
    pub mie_coefficient:    f32,
    pub mie_directional_g:  f32,
    pub _pad3:              f32,
}

impl Default for SkyParams {
    fn default() -> Self {
        Self {
            horizon_color:     [0.6, 0.75, 1.0],
            sky_mode:          0,
            zenith_color:      [0.1, 0.3, 0.8],
            _pad1:             0.0,
            sun_direction:     [0.5, 0.8, 0.3],
            sun_intensity:     20.0,
            sun_size:          0.02,
            _pad2a:            0.0,
            _pad2b:            0.0,
            _pad2c:            0.0,
            solid_color:       [0.05, 0.07, 0.10],
            turbidity:         2.0,
            rayleigh:          1.0,
            mie_coefficient:   0.005,
            mie_directional_g: 0.8,
            _pad3:             0.0,
        }
    }
}

// ── Per-frame CPU data extracted from ECS ─────────────────────────────────────

/// Camera matrices for this frame.
#[derive(Debug, Clone)]
pub struct CameraData {
    pub view:             Mat4,
    pub projection:       Mat4,
    pub view_proj:        Mat4,
    pub inv_view_proj:    Mat4,
    pub inv_proj:         Mat4,
    pub position:         Vec3,
    pub near:             f32,
    pub far:              f32,
    /// Normalized viewport rect `[x, y, w, h]` (0–1).
    pub viewport_rect:    [f32; 4],
    /// How this camera clears the render target.
    pub clear_flags:      ClearFlags,
    /// Background colour used when `clear_flags == SolidColor`.
    pub background_color: [f32; 4],
}

impl CameraData {
    pub fn identity() -> Self {
        CameraData {
            view:             Mat4::IDENTITY,
            projection:       Mat4::IDENTITY,
            view_proj:        Mat4::IDENTITY,
            inv_view_proj:    Mat4::IDENTITY,
            inv_proj:         Mat4::IDENTITY,
            position:         Vec3::ZERO,
            near:             0.1,
            far:              1000.0,
            viewport_rect:    [0.0, 0.0, 1.0, 1.0],
            clear_flags:      ClearFlags::Skybox,
            background_color: [0.1, 0.1, 0.1, 1.0],
        }
    }
}

/// A single skinned mesh draw call (has joint matrices for GPU skinning).
pub struct SkinnedDrawCall {
    pub skinned_mesh: u32,    // handle into SkinnedMeshRegistry
    pub material:     u32,
    pub world_matrix: Mat4,
    pub normal_matrix: Mat4,
    pub cast_shadow:  bool,
    pub layer:        u8,
    /// Per-joint bone matrices (MAX_JOINTS entries).
    pub joint_matrices: Vec<Mat4>,
}

/// A single opaque mesh draw call.
pub struct MeshDrawCall {
    pub mesh:         u32,        // handle into MeshRegistry
    pub material:     u32,        // handle into MaterialRegistry
    pub world_matrix: Mat4,
    pub normal_matrix: Mat4,      // inverse-transpose of world matrix
    pub cast_shadow:  bool,
    pub layer:        u8,
}

/// One GPU instance for the particle overlay pass (tightly packed for WGSL `locations` 0–2).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParticleInstance {
    pub position: [f32; 3],
    pub size:     f32,
    pub color:    [f32; 4],
}

/// All per-frame rendering data extracted from the ECS world.
/// Built by `FluxionRenderer::extract_frame_data()` before the render graph runs.
pub struct FrameData {
    pub camera:      CameraData,
    pub draw_calls:  Vec<MeshDrawCall>,
    pub lights:      Vec<LightUniform>,
    pub viewport:    (u32, u32),  // (width, height)
    pub time:        f32,         // engine elapsed time in seconds
    /// Procedural sky (from [`fluxion_core::scene::SceneSettings`] + first directional light).
    pub sky:         SkyParams,
    /// Billboard particles (world space), drawn in overlay pass.
    pub particles:   Vec<ParticleInstance>,
    /// Debug line segments drained from `fluxion_core::drain_debug_lines()` each frame.
    pub debug_lines: Vec<DebugLine>,
    /// Light-space view-projection matrix for the first shadow-casting directional light.
    /// [`Mat4::IDENTITY`] when no shadow-casting light is active.
    pub shadow_view_proj: Mat4,
    /// Whether at least one light with `cast_shadow = true` is present this frame.
    pub has_shadow_caster: bool,
    /// Skinned mesh draw calls (entities with Animator + SkinnedMeshRenderer).
    pub skinned_draw_calls: Vec<SkinnedDrawCall>,
    /// Debug view mode: 0=Lit, 1=Albedo, 2=Normal, 3=Roughness, 4=Metalness, 5=AO, 6=Emissive, 7=Unlit.
    pub debug_view: u32,
}

// ── Shared GPU render targets ─────────────────────────────────────────────────

/// GBuffer and intermediate render targets shared by all passes.
///
/// Re-created when the viewport is resized. Passes hold borrowed references
/// to these for the duration of one frame.
pub struct RenderResources {
    // ── GBuffer (written by geometry pass, read by lighting pass) ─────────────
    /// RGB = albedo (linear sRGB), A = ambient occlusion
    pub gbuf_albedo_ao: GpuTexture,
    /// RGB = world-space normal encoded to [0,1]
    pub gbuf_normal:    GpuTexture,
    /// R = occlusion, G = roughness, B = metalness
    pub gbuf_orm:       GpuTexture,
    /// RGB = emission, A = unused
    pub gbuf_emission:  GpuTexture,
    /// Depth buffer (Depth32Float)
    pub depth:          GpuTexture,

    // ── HDR intermediate targets (ping-pong for post-fx) ──────────────────────
    /// Output of lighting pass. Input to post-processing chain.
    pub hdr_main: GpuTexture,
    /// Second HDR buffer for ping-pong (bloom blur, etc.)
    pub hdr_ping: GpuTexture,
    /// Third HDR buffer (bloom upsample, etc.)
    pub hdr_pong: GpuTexture,

    // ── SSAO ──────────────────────────────────────────────────────────────────
    pub ssao_raw:    GpuTexture,  // unblurred SSAO output
    pub ssao_blur:   GpuTexture,  // blurred SSAO (multiplied into lighting)

    // ── Bloom ─────────────────────────────────────────────────────────────────
    pub bloom_bright: GpuTexture, // bright-pass extracted
    pub bloom_blur_a: GpuTexture, // blur ping
    pub bloom_blur_b: GpuTexture, // blur pong

    // ── Shadow map ────────────────────────────────────────────────────────────
    /// Depth texture rendered from the first directional shadow-casting light.
    pub shadow_map: GpuTexture,

    pub width:  u32,
    pub height: u32,
}

impl RenderResources {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        use wgpu::TextureFormat::*;
        let rt  = |label, fmt| GpuTexture::render_target(device, label, width, height, fmt);
        let rth = |label, fmt| GpuTexture::render_target(device, label, width / 2, height / 2, fmt);

        Self {
            gbuf_albedo_ao: rt("gbuf_albedo_ao", Rgba8UnormSrgb),
            gbuf_normal:    rt("gbuf_normal",    Rgba8Unorm),
            gbuf_orm:       rt("gbuf_orm",       Rgba8Unorm),
            gbuf_emission:  rt("gbuf_emission",  Rgba16Float),
            depth:          GpuTexture::depth(device, "depth", width, height),

            hdr_main:       rt("hdr_main", Rgba16Float),
            hdr_ping:       rt("hdr_ping", Rgba16Float),
            hdr_pong:       rt("hdr_pong", Rgba16Float),

            ssao_raw:       rt("ssao_raw",  Rgba8Unorm),
            ssao_blur:      rt("ssao_blur", Rgba8Unorm),

            bloom_bright:   rth("bloom_bright",   Rgba16Float),
            bloom_blur_a:   rth("bloom_blur_a",   Rgba16Float),
            bloom_blur_b:   rth("bloom_blur_b",   Rgba16Float),

            shadow_map: GpuTexture::depth(device, "shadow_map", SHADOW_MAP_SIZE, SHADOW_MAP_SIZE),

            width,
            height,
        }
    }

    /// Recreate all textures at the new size.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        *self = Self::new(device, width, height);
    }
}

// ── RenderContext ─────────────────────────────────────────────────────────────

/// Per-frame context passed to each RenderPass::execute().
///
/// Passes record GPU commands via `encoder`. They read shared targets from
/// `resources` and per-frame scene data from `frame`.
pub struct RenderContext<'frame> {
    pub device:       &'frame wgpu::Device,
    pub queue:        &'frame wgpu::Queue,
    pub encoder:      &'frame mut wgpu::CommandEncoder,
    pub resources:    &'frame RenderResources,
    pub frame:        &'frame FrameData,
    /// The surface texture view — final render target written by TonemapPass.
    pub surface_view: &'frame TextureView,
    /// The GPU-side light buffer (uploaded by FluxionRenderer before graph runs).
    pub light_buffer: &'frame wgpu::Buffer,
    /// Mesh registry — provides vertex/index buffers for each draw call.
    pub meshes:       &'frame MeshRegistry,
    /// Material registry — provides bind groups for each draw call.
    pub materials:    &'frame MaterialRegistry,
    /// Skinned mesh registry — provides skinned vertex/index buffers.
    pub skinned_meshes: &'frame crate::mesh::SkinnedMeshRegistry,
}
