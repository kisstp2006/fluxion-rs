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
pub mod rigid_body;
pub mod camera_controller;
pub mod animator;
pub mod animation_system;
pub mod lod;
pub mod environment;

pub use mesh_renderer::MeshRenderer;
pub use camera::{Camera, ProjectionMode, ClearFlags};
pub use light::{Light, LightType};
pub use particle_emitter::{Particle, ParticleEmitter};
pub use rigid_body::{RigidBody, PhysicsShape, BodyType};
pub use camera_controller::{CameraController, ControllerType, CameraControllerSystem};
pub use animator::{Animator, AnimationClip, Skeleton, JointDef, JointChannel, KeyframeVec3, KeyframeQuat, MAX_JOINTS};
pub use animation_system::AnimationSystem;
pub use lod::{LodGroup, LodLevel, LodSystem};
pub use environment::{Environment, BackgroundMode, SkySettings, ToneMapMode, FogMode, AmbientSettings, FogSettings, ToneMapSettings, BloomSettings as EnvBloomSettings, SsaoSettings as EnvSsaoSettings, DofSettings, VignetteSettings, FilmSettings, sun_direction_from_angles};
