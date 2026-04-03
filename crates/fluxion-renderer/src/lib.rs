// ============================================================
// fluxion-renderer
//
// wgpu-based deferred PBR renderer for FluxionRS.
// Reads component data from fluxion-core's ECSWorld and produces
// frames on a wgpu Surface (native window or HTML canvas).
//
// Module overview:
//   renderer      — FluxionRenderer: top-level orchestrator
//   render_graph  — RenderGraph, RenderPass trait, RenderContext
//   passes/       — Built-in render passes (geometry, lighting, post-fx, etc.)
//   material/     — MaterialAsset (disk format) + PbrMaterial (GPU resident)
//   texture/      — GpuTexture + TextureCache (Arc-based dedup)
//   mesh/         — GpuMesh + MeshRegistry + primitive builders
//   lighting/     — LightUniform GPU layout + LightBuffer
//   shader/       — ShaderCache + embedded WGSL library
// ============================================================

pub mod renderer;
pub mod render_graph;
pub mod passes;
pub mod material;
pub mod texture;
pub mod mesh;
pub mod lighting;
pub mod shader;

pub use renderer::FluxionRenderer;
pub use render_graph::{RenderGraph, RenderPass, PassSlot};
pub use material::{MaterialAsset, PbrMaterial};
pub use texture::{GpuTexture, TextureCache};
pub use mesh::{GpuMesh, MeshRegistry};
