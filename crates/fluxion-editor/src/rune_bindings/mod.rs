pub mod ui_module;
pub mod world_module;
pub mod viewport_module;
pub mod physics_module;
pub mod input_module;
pub mod camera_module;
pub mod environment_module;
pub mod gameplay_module;
pub mod settings_module;

use anyhow::Result;
use rune::Module;

pub use ui_module::{set_current_ui, UiContextGuard, get_viewport_rect, drain_cursor_grab, drain_cursor_visible, accumulate_raw_mouse_delta, drain_raw_mouse_delta, set_egui_ctx};
pub use world_module::{
    set_world_context, WorldContextGuard,
    drain_pending_edits, PendingEdit,
    get_selected_id, set_selected_id, set_project_root, set_undo_state, set_frame_time, set_time_elapsed,
    set_editor_shell_state, drain_action_signals, get_editor_mode_str, force_editor_mode, get_transform_tool_str,
    get_editor_cam_pos, get_editor_cam_yaw, get_editor_cam_pitch,
    get_editor_cam_state, EditorCameraState,
    init_editor_cam, take_editor_cam_dirty,
    set_asset_db_context, clear_asset_db_context,
    set_frame_stats,
    get_snap_enabled, get_snap_translate, get_snap_rotate, get_snap_scale,
    get_multi_selected,
    get_box_gizmo_mode_raw,
    set_editor_cam_entity, get_editor_cam_entity_id,
    rescan_asset_refs, load_asset_refs, apply_path_rename, clear_asset_refs_by_guid,
    entity_from_id,
};
pub use camera_module::{
    set_camera_snapshot, set_camera_world, clear_camera_world,
    drain_camera_edits, CameraSnapshot,
};
pub use environment_module::{
    set_environment_world, clear_environment_world, drain_environment_edits, EnvEditValue,
};
pub use viewport_module::{set_viewport_texture, set_pane_texture, get_layout, get_fullscreen};
pub use physics_module::{set_physics_context, clear_physics_context};
pub use fluxion_audio::{set_audio_context, clear_audio_context};
pub use input_module::{set_input_context, clear_input_context, set_action_map};
pub use gameplay_module::{
    set_self_entity, clear_self_entity,
    set_self_script, clear_self_script,
    set_script_error, clear_script_error,
    drain_pending_destroys, drain_pending_spawns,
    build_gameplay_modules,
    set_compile_summary,
    set_script_fields, drain_script_fields,
    set_script_hotreload_pending,
};
#[allow(unused_imports)]
pub use settings_module::{
    set_settings_context, clear_settings_context,
    drain_settings_saves, get_current_prefs,
    get_show_project_settings, get_show_editor_prefs,
};

pub fn all_editor_modules() -> Result<Vec<Module>> {
    let mut modules = vec![
        ui_module::build_ui_module()?,
        world_module::build_world_module()?,
        viewport_module::build_viewport_module()?,
        fluxion_physics::build_physics_rune_module()?,
        fluxion_audio::build_audio_rune_module()?,
        input_module::build_input_module()?,
        environment_module::build_environment_module()?,
        settings_module::build_settings_module()?,
    ];
    // build_camera_module returns two modules: fluxion::camera + fluxion::Camera
    modules.extend(camera_module::build_camera_module()?);
    Ok(modules)
}
