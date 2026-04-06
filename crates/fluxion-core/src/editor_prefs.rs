// ============================================================
// fluxion-core — Editor Preferences
//
// User-level preferences stored in ~/.fluxion/editor_prefs.json.
// These persist across all projects (unlike .fluxproj settings).
// ============================================================

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorPrefs {
    /// UI theme: "dark" | "light"
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Base font size in points (clamped 9–24).
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    /// Autosave scene interval in seconds; 0 = disabled.
    #[serde(default = "default_autosave")]
    pub autosave_interval_secs: u32,
    /// Restore the previous dock layout on startup.
    #[serde(default = "default_true")]
    pub restore_layout: bool,
    /// Maximum log entries kept in the Console panel.
    #[serde(default = "default_log_max")]
    pub log_max_entries: u32,
    /// Editor fly-camera movement speed (units/sec).
    #[serde(default = "default_camera_speed")]
    pub camera_speed: f32,
    /// Editor fly-camera mouse-look sensitivity multiplier.
    #[serde(default = "default_camera_sensitivity")]
    pub camera_sensitivity: f32,
}

fn default_theme()            -> String { "dark".to_string() }
fn default_font_size()        -> f32    { 13.0 }
fn default_autosave()         -> u32    { 120 }
fn default_true()             -> bool   { true }
fn default_log_max()          -> u32    { 10_000 }
fn default_camera_speed()     -> f32    { 5.0 }
fn default_camera_sensitivity() -> f32  { 1.0 }

impl Default for EditorPrefs {
    fn default() -> Self {
        Self {
            theme:                  default_theme(),
            font_size:              default_font_size(),
            autosave_interval_secs: default_autosave(),
            restore_layout:         default_true(),
            log_max_entries:        default_log_max(),
            camera_speed:           default_camera_speed(),
            camera_sensitivity:     default_camera_sensitivity(),
        }
    }
}

impl EditorPrefs {
    /// Clamp all numeric fields to their valid ranges.
    /// Call after load and before save.
    pub fn clamp(&mut self) {
        self.font_size            = self.font_size.clamp(9.0, 24.0);
        self.camera_speed         = self.camera_speed.clamp(0.1, 500.0);
        self.camera_sensitivity   = self.camera_sensitivity.clamp(0.05, 10.0);
        self.log_max_entries      = self.log_max_entries.clamp(100, 100_000);
        if self.theme != "light" { self.theme = "dark".to_string(); }
    }
}

// ── Native file operations (desktop only) ─────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn editor_prefs_path() -> std::path::PathBuf {
    crate::project::fluxion_config_dir().join("editor_prefs.json")
}

/// Load editor preferences from `~/.fluxion/editor_prefs.json`.
/// Returns `Default::default()` on any I/O or parse error (never panics).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_editor_prefs() -> EditorPrefs {
    let path = editor_prefs_path();
    match std::fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str::<EditorPrefs>(&raw) {
            Ok(mut prefs) => { prefs.clamp(); prefs }
            Err(e) => {
                log::warn!("editor_prefs.json parse error ({e}) — using defaults");
                EditorPrefs::default()
            }
        },
        Err(_) => EditorPrefs::default(),
    }
}

/// Write editor preferences to `~/.fluxion/editor_prefs.json` atomically.
/// Returns `Ok(())` on success or an error string.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_editor_prefs(prefs: &EditorPrefs) -> Result<(), String> {
    let path = editor_prefs_path();
    let tmp  = path.with_extension("json.tmp");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create_dir_all: {e}"))?;
    }
    let json = serde_json::to_string_pretty(prefs)
        .map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(&tmp, &json)
        .map_err(|e| format!("write '{}': {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| format!("rename: {e}"))?;
    Ok(())
}
