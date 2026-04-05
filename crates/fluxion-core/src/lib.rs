// ============================================================
// fluxion-core
//
// The engine core library. Contains:
//   - ECS (Entity Component System)
//   - Transform hierarchy
//   - Event bus
//   - Time / fixed-timestep
//   - Scene serialization + instantiate into ECSWorld (FluxionJS-compatible)
//   - Input snapshot (platform-agnostic)
//   - Unity-style facade (GameObject)
//   - Built-in components (Transform, MeshRenderer, Camera, Light)
//
// This crate has NO wgpu / rendering dependency.
// It compiles cleanly to native and to WASM (wasm32-unknown-unknown).
//
// C++/C# developers: think of this as the "engine runtime" DLL —
// it defines the data model but not the rendering backend.
// ============================================================

pub mod ecs;
pub mod hierarchy;
pub mod transform;
pub mod event;
pub mod time;
pub mod scene;
pub mod components;
pub mod input;
pub mod facade;
pub mod assets;
pub mod particles;
pub mod color;
pub mod debug_draw;
pub mod project;
pub mod registry;
pub mod reflect;

// Re-export the most commonly used types at the crate root so users
// can write `use fluxion_core::ECSWorld` instead of the full path.
pub use ecs::world::ECSWorld;
pub use ecs::entity::EntityId;
pub use ecs::component::Component;
pub use transform::Transform;
pub use event::{EventBus, EventHandle, EngineEvent};
pub use time::Time;
pub use input::InputState;
pub use facade::GameObject;
pub use scene::{
    instantiate_entities, load_scene_from_bytes, load_scene_into_world, parse_prefab_json,
    spawn_prefab_into_world, PrefabFileData,
};
#[cfg(not(target_arch = "wasm32"))]
pub use scene::world_to_scene_data;
pub use registry::ComponentRegistry;
pub use reflect::{Reflect, ReflectValue, FieldDescriptor, ReflectFieldType, RangeHint};
pub use reflect::{ParameterDescriptor, MethodDescriptor, MethodType, MethodVisibility, ReflectMethods};
#[doc(hidden)]
pub use fluxion_reflect_derive::Reflect as DeriveReflect;
pub use components::{RigidBody, PhysicsShape, BodyType};
pub use components::{Camera, ProjectionMode, ClearFlags};
pub use components::{Animator, AnimationClip, Skeleton, JointDef, JointChannel, MAX_JOINTS};
pub use components::AnimationSystem;
pub use components::{LodGroup, LodLevel, LodSystem};
pub use components::{CameraController, ControllerType, CameraControllerSystem};
pub use components::{Environment, ToneMapMode, FogMode, AmbientSettings, FogSettings, ToneMapSettings, DofSettings, VignetteSettings, FilmSettings};
pub use components::{EnvBloomSettings, EnvSsaoSettings};
pub use particles::step_particle_emitters;
pub use color::Color;
pub use project::{ProjectConfig, ProjectSettings, ProjectPhysicsSettings,
    ProjectRenderSettings, ProjectEditorSettings, RecentProject};
#[cfg(not(target_arch = "wasm32"))]
pub use project::{
    load_project, save_project, create_project,
    load_recent_projects, push_recent_project, save_recent_projects,
    project_file_path, fluxion_config_dir,
};
pub use debug_draw::{DebugDraw, DebugLine, draw_line, draw_ray, draw_sphere,
    draw_aabb, draw_box_rotated, draw_capsule, draw_cross, draw_grid,
    draw_cone, draw_frustum, drain_debug_lines};

// WASM entry-point: sets up the browser panic hook so Rust panics appear
// as readable messages in the browser console instead of "unreachable".
#[cfg(feature = "wasm")]
use wasm_bindgen::prelude::*;

#[cfg(feature = "wasm")]
#[wasm_bindgen(start)]
pub fn wasm_init() {
    #[cfg(feature = "wasm")]
    console_error_panic_hook::set_once();
}
