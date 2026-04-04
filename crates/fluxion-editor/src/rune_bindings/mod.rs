pub mod ui_module;
pub mod world_module;
pub mod viewport_module;
pub mod gizmo_module;

use anyhow::Result;
use rune::Module;

pub use ui_module::{set_current_ui, UiContextGuard};
pub use world_module::{
    set_world_context, clear_world_context, WorldContextGuard,
    drain_pending_edits, PendingEdit,
    push_log, get_selected_id, set_project_root, set_undo_state, set_frame_time,
};
pub use viewport_module::set_viewport_texture;
pub use gizmo_module::clear_gizmo_frame;

pub fn all_editor_modules() -> Result<Vec<Module>> {
    Ok(vec![
        ui_module::build_ui_module()?,
        world_module::build_world_module()?,
        viewport_module::build_viewport_module()?,
        gizmo_module::build_gizmo_module()?,
    ])
}
