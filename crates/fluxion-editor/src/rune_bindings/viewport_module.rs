// ============================================================
// viewport_module.rs — fluxion::viewport Rune module
//
// Exposes the egui TextureId (as i64) and the current viewport
// dimensions to Rune panel scripts.  The texture is registered
// by EditorInner::frame() each frame after render_to_viewport.
// ============================================================

use std::cell::Cell;

use rune::Module;

// ── Thread-local state ────────────────────────────────────────────────────────

thread_local! {
    static VP_TEXTURE: Cell<Option<egui::TextureId>> = Cell::new(None);
}

/// Set the current viewport texture ID.
/// Called by `EditorInner::frame()` after `render_to_viewport`.
/// Width/height are pushed to the VM separately via `RuneVm::push_viewport`.
pub fn set_viewport_texture(id: egui::TextureId, width: u32, height: u32) {
    let _ = (width, height); // dimensions go through RuneVm::push_viewport instead
    VP_TEXTURE.with(|c| c.set(Some(id)));
}

/// Build the `fluxion::viewport` extension module.
///
/// Only registers `texture_id` — `width` and `height` are already provided
/// by the base `fluxion-rune-scripting` viewport module and must not be
/// re-registered here (duplicate function hashes cause a panic at startup).
pub fn build_viewport_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["viewport"])?;

    m.function("texture_id", || -> i64 {
        VP_TEXTURE.with(|c| match c.get() {
            None                              => -1i64,
            Some(egui::TextureId::Managed(v)) => v as i64,
            Some(egui::TextureId::User(v))    => (v | (1u64 << 62)) as i64,
        })
    }).build()?;

    Ok(m)
}
