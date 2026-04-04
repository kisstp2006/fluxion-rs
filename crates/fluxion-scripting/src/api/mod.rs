// ============================================================
// fluxion-scripting — API modules
//
// Each submodule exposes a `register(registry)` function that
// populates the ScriptBindingRegistry with its handlers.
//
// All modules combined form the engine scripting API, designed
// to mirror Unity's MonoBehaviour scripting surface as closely
// as possible.
// ============================================================

pub mod input_api;
pub mod physics_api;
pub mod ui_api;
pub mod window_api;
pub mod time_api;
pub mod gameobject_api;
pub mod debug_api;

use crate::binding_registry::ScriptBindingRegistry;

/// Register all built-in API modules into the registry.
///
/// Call once at engine startup before `apply_registry_to_vm`.
pub fn register_all(registry: &mut ScriptBindingRegistry) {
    input_api::register(registry);
    physics_api::register(registry);
    ui_api::register(registry);
    window_api::register(registry);
    time_api::register(registry);
    gameobject_api::register(registry);
    debug_api::register(registry);
}
