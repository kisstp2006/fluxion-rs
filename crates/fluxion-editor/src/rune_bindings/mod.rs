pub mod ui_module;
pub mod world_module;
pub mod viewport_module;
pub mod physics_module;
pub mod input_module;
pub mod camera_module;

use anyhow::Result;
use rune::Module;

pub use ui_module::{set_current_ui, UiContextGuard, get_viewport_rect};
pub use world_module::{
    set_world_context, clear_world_context, WorldContextGuard,
    drain_pending_edits, PendingEdit,
    push_log, get_selected_id, set_project_root, set_undo_state, set_frame_time,
};
pub use camera_module::{
    set_camera_snapshot, set_camera_world, clear_camera_world,
    drain_camera_edits, CameraSnapshot,
};
pub use viewport_module::set_viewport_texture;
pub use physics_module::{set_physics_context, clear_physics_context};
pub use fluxion_audio::{set_audio_context, clear_audio_context};
pub use input_module::{set_input_context, clear_input_context};

pub fn all_editor_modules() -> Result<Vec<Module>> {
    Ok(vec![
        ui_module::build_ui_module()?,
        world_module::build_world_module()?,
        viewport_module::build_viewport_module()?,
        camera_module::build_camera_module()?,
        fluxion_physics::build_physics_rune_module()?,
        fluxion_audio::build_audio_rune_module()?,
        input_module::build_input_module()?,
    ])
}
