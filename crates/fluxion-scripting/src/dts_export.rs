// ============================================================
// dts_export.rs — Write combined TypeScript .d.ts files
//
// Combines:
//   1. API function declarations from ScriptBindingRegistry
//   2. Component interface declarations from ComponentRegistry
//
// Called once at startup (native only) to write:
//   <asset_root>/types/engine.d.ts  — all engine API namespaces
//   <asset_root>/types/components.d.ts — component interfaces
// ============================================================

use std::path::Path;

use fluxion_core::ComponentRegistry;

use crate::binding_registry::ScriptBindingRegistry;

/// Build a `ScriptBindingRegistry` populated with all built-in API modules.
fn build_api_registry() -> ScriptBindingRegistry {
    let mut reg = ScriptBindingRegistry::new();
    crate::api::debug_api::register(&mut reg);
    crate::api::time_api::register(&mut reg);
    crate::api::input_api::register(&mut reg);
    crate::api::physics_api::register(&mut reg);
    crate::api::ui_api::register(&mut reg);
    crate::api::window_api::register(&mut reg);
    crate::api::gameobject_api::register(&mut reg);
    reg
}

/// Write both `.d.ts` files to `<output_dir>/`.
///
/// Creates the directory if it does not exist.
/// Silently skips write on any I/O error (non-fatal).
pub fn write_dts_files(output_dir: &Path, registry: &ComponentRegistry) {
    if let Err(e) = std::fs::create_dir_all(output_dir) {
        log::warn!("[DTS] Could not create output dir {:?}: {e}", output_dir);
        return;
    }

    // 1. Engine API declarations.
    let api_dts = build_api_registry().generate_dts();
    let engine_path = output_dir.join("engine.d.ts");
    if let Err(e) = std::fs::write(&engine_path, &api_dts) {
        log::warn!("[DTS] Could not write {:?}: {e}", engine_path);
    } else {
        log::info!("[DTS] Wrote {:?}", engine_path);
    }

    // 2. Component interface declarations.
    let comp_dts = registry.generate_component_dts();
    let comp_path = output_dir.join("components.d.ts");
    if let Err(e) = std::fs::write(&comp_path, &comp_dts) {
        log::warn!("[DTS] Could not write {:?}: {e}", comp_path);
    } else {
        log::info!("[DTS] Wrote {:?}", comp_path);
    }
}
