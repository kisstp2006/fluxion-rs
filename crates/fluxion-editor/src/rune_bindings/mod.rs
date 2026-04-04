pub mod ui_module;
pub mod world_module;

use anyhow::Result;
use rune::Module;

pub use ui_module::{set_current_ui, clear_current_ui};
pub use world_module::{
    set_world_context, clear_world_context,
    drain_pending_edits, PendingEdit,
    push_log, get_selected_id,
};

pub fn all_editor_modules() -> Result<Vec<Module>> {
    Ok(vec![
        ui_module::build_ui_module()?,
        world_module::build_world_module()?,
    ])
}
