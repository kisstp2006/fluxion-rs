// ============================================================
// physics_module.rs — thin re-export for the editor host
//
// The actual fluxion::physics Rune module and its thread-local
// context live in fluxion-physics/src/rune_module.rs.
// The editor host only needs to call set_physics_context() /
// clear_physics_context() and register the module — all logic
// stays inside the physics crate.
// ============================================================

pub use fluxion_physics::{
    set_physics_context,
    clear_physics_context,
};
