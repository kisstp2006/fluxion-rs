// ============================================================
// fluxion-core — Built-in components
//
// These are the standard components every game engine needs.
// They live in fluxion-core (no rendering dependency) because:
//   - fluxion-renderer reads them to build FrameData
//   - User scripts read/write them
//   - Scene serialization handles them
//
// Each component is a plain data struct. The renderer creates
// GPU-resident resources (GpuMesh, PbrMaterial) lazily when it
// first sees a component, and caches the handles inside the component.
// The handles are NOT serialized — they are recreated on load.
// ============================================================

pub mod mesh_renderer;
pub mod camera;
pub mod light;
pub mod particle_emitter;

pub use mesh_renderer::MeshRenderer;
pub use camera::{Camera, ProjectionMode};
pub use light::{Light, LightType};
pub use particle_emitter::{Particle, ParticleEmitter};
