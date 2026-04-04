// ============================================================
// fluxion-renderer — Built-in WGSL shader library
//
// Each shader is a &'static str embedded at compile time.
// The actual WGSL source lives in assets/shaders/ alongside the
// project root — this file just re-exports them so the renderer
// can reference them by name without filesystem access at runtime.
//
// Naming convention: MODULE_SHADER (e.g. FULLSCREEN_VERT, GEOMETRY_FRAG).
// ============================================================

/// Reusable fullscreen triangle vertex shader.
/// Outputs a clip-space triangle that covers the entire viewport.
/// All post-processing passes use this as their vertex stage.
pub const FULLSCREEN_VERT: &str = include_str!("../../../../assets/shaders/fullscreen.vert.wgsl");

/// GBuffer fill — vertex stage (transforms mesh vertices to clip space).
pub const GEOMETRY_VERT: &str = include_str!("../../../../assets/shaders/geometry.vert.wgsl");

/// GBuffer fill — fragment stage (writes albedo, normal, ORM, emission).
pub const GEOMETRY_FRAG: &str = include_str!("../../../../assets/shaders/geometry.frag.wgsl");

/// Deferred PBR lighting — full-screen pass reading GBuffer.
pub const PBR_LIGHTING: &str = include_str!("../../../../assets/shaders/pbr_lighting.wgsl");

/// ACES tonemapping + gamma correction + vignette — final blit to surface.
pub const TONEMAP: &str = include_str!("../../../../assets/shaders/tonemap.wgsl");

/// Bloom bright-pass — extracts pixels above luminance threshold.
pub const BLOOM_BRIGHT: &str = include_str!("../../../../assets/shaders/bloom_bright.wgsl");

/// Bloom blur — dual-pass Kawase blur (used for both horizontal and vertical).
pub const BLOOM_BLUR: &str = include_str!("../../../../assets/shaders/bloom_blur.wgsl");

/// Bloom composite — adds blurred bloom texture to HDR scene.
pub const BLOOM_COMPOSITE: &str = include_str!("../../../../assets/shaders/bloom_composite.wgsl");

/// SSAO — hemisphere sampling in screen space.
pub const SSAO: &str = include_str!("../../../../assets/shaders/ssao.wgsl");

/// SSAO blur — bilateral blur to smooth SSAO samples.
pub const SSAO_BLUR: &str = include_str!("../../../../assets/shaders/ssao_blur.wgsl");

/// Skybox — renders a cubemap or procedural sky behind all geometry.
pub const SKYBOX: &str = include_str!("../../../../assets/shaders/skybox.wgsl");

/// Instanced billboard particles (overlay, alpha blend).
pub const PARTICLES: &str = include_str!("../../../../assets/shaders/particles.wgsl");

/// Debug line overlay — colored LineList, no depth test.
pub const DEBUG_LINES: &str = include_str!("../../../../assets/shaders/debug_lines.wgsl");
