pub mod ui_module;
pub mod world_module;
pub mod viewport_module;
pub mod physics_module;
pub mod input_module;
pub mod camera_module;
pub mod environment_module;

use anyhow::Result;
use rune::Module;

pub use ui_module::{set_current_ui, UiContextGuard, get_viewport_rect, drain_cursor_grab, drain_cursor_visible, accumulate_raw_mouse_delta, drain_raw_mouse_delta};
pub use world_module::{
    set_world_context, clear_world_context, WorldContextGuard,
    drain_pending_edits, PendingEdit,
    push_log, get_selected_id, set_project_root, set_undo_state, set_frame_time, set_time_elapsed,
    set_editor_shell_state, drain_action_signals, get_editor_mode_str, get_transform_tool_str,
    get_editor_cam_pos, get_editor_cam_yaw, get_editor_cam_pitch,
    init_editor_cam, take_editor_cam_dirty,
};
pub use camera_module::{
    set_camera_snapshot, set_camera_world, clear_camera_world,
    drain_camera_edits, CameraSnapshot,
};
pub use environment_module::{
    set_environment_world, clear_environment_world, drain_environment_edits, EnvEdit, EnvEditValue,
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
        environment_module::build_environment_module()?,
    ])
}
