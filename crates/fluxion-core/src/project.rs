// ============================================================
// fluxion-core — Project system
//
// Mirrors FluxionJsV3's ProjectManager in Rust.
// A project is a folder containing a `.fluxproj` JSON file.
// Recent projects are stored in `~/.fluxion/recent.json`.
// ============================================================

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use crate::input::{InputAction, default_input_actions};

// ── Project audio settings ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectAudioSettings {
    pub master_volume: f32,
    pub music_volume:  f32,
    pub sfx_volume:    f32,
}

impl Default for ProjectAudioSettings {
    fn default() -> Self {
        Self { master_volume: 1.0, music_volume: 1.0, sfx_volume: 1.0 }
    }
}

// ── Project input settings ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInputSettings {
    pub mouse_sensitivity: f32,
    pub gamepad_deadzone:  f32,
    /// Named logical input actions and their key/button bindings.
    #[serde(default = "default_input_actions")]
    pub actions: Vec<InputAction>,
}

impl Default for ProjectInputSettings {
    fn default() -> Self {
        Self {
            mouse_sensitivity: 1.0,
            gamepad_deadzone:  0.15,
            actions:           default_input_actions(),
        }
    }
}

// ── Project tags settings ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectTagSettings {
    pub list: Vec<String>,
}

impl Default for ProjectTagSettings {
    fn default() -> Self {
        Self {
            list: vec![
                "Untagged".to_string(),
                "Player".to_string(),
                "Enemy".to_string(),
                "Environment".to_string(),
            ],
        }
    }
}

// ── Project build settings ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBuildSettings {
    pub target_platform: String,
    pub window_title:    String,
    pub window_width:    u32,
    pub window_height:   u32,
    pub vsync:           bool,
    pub fullscreen:      bool,
}

impl Default for ProjectBuildSettings {
    fn default() -> Self {
        Self {
            target_platform: "Windows".to_string(),
            window_title:    String::new(),
            window_width:    1920,
            window_height:   1080,
            vsync:           true,
            fullscreen:      false,
        }
    }
}

impl ProjectBuildSettings {
    /// Returns a list of human-readable validation errors.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.window_title.trim().is_empty() {
            errors.push("Window title must not be empty.".to_string());
        }
        if self.window_width == 0 {
            errors.push("Window width must be greater than 0.".to_string());
        }
        if self.window_height == 0 {
            errors.push("Window height must be greater than 0.".to_string());
        }
        errors
    }
}

// ── Project physics settings ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPhysicsSettings {
    pub gravity:         [f32; 3],
    pub fixed_timestep:  f32,
}

impl Default for ProjectPhysicsSettings {
    fn default() -> Self {
        Self { gravity: [0.0, -9.81, 0.0], fixed_timestep: 1.0 / 60.0 }
    }
}

// ── Project render settings ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRenderSettings {
    pub shadows:        bool,
    pub shadow_map_size: u32,
    pub tone_mapping:   String,
    pub exposure:       f32,
}

impl Default for ProjectRenderSettings {
    fn default() -> Self {
        Self {
            shadows:         true,
            shadow_map_size: 2048,
            tone_mapping:    "ACES".to_string(),
            exposure:        1.2,
        }
    }
}

// ── Editor-specific settings ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEditorSettings {
    pub snap_translation: f32,
    pub snap_rotation:    f32,
    pub snap_scale:       f32,
    pub show_grid:        bool,
}

impl Default for ProjectEditorSettings {
    fn default() -> Self {
        Self {
            snap_translation: 1.0,
            snap_rotation:    15.0,
            snap_scale:       0.25,
            show_grid:        true,
        }
    }
}

// ── Collision layer settings ──────────────────────────────────────────────────

fn default_layer_names() -> Vec<String> {
    let mut v: Vec<String> = (0..32).map(|i| if i == 0 { "Default".to_string() } else { format!("Layer {}", i) }).collect();
    v[1] = "TransparentFX".to_string();
    v[2] = "IgnoreRaycast".to_string();
    v[3] = "Water".to_string();
    v[4] = "UI".to_string();
    v
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollisionLayerSettings {
    /// 32 layer names. Index == bit position in collision_layer/collision_mask.
    #[serde(default = "default_layer_names")]
    pub names: Vec<String>,
}

impl Default for CollisionLayerSettings {
    fn default() -> Self {
        Self { names: default_layer_names() }
    }
}

// ── CVar (console variable) storage ──────────────────────────────────────────

/// Runtime console variables stored in the .fluxproj file.
/// Values are always strings; the Rune API provides typed getters/setters.
/// Built-in cvar names: "r.shadows", "r.vsync", "r.tonemap",
///   "a.master_volume", "p.gravity_y", "e.show_grid".
pub fn default_cvars() -> std::collections::HashMap<String, String> {
    std::collections::HashMap::new()
}

// ── Aggregated project settings ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectSettings {
    pub physics:  ProjectPhysicsSettings,
    pub render:   ProjectRenderSettings,
    pub editor:   ProjectEditorSettings,
    #[serde(default)]
    pub audio:    ProjectAudioSettings,
    #[serde(default)]
    pub input:    ProjectInputSettings,
    #[serde(default)]
    pub tags:     ProjectTagSettings,
    #[serde(default)]
    pub build:    ProjectBuildSettings,
    /// Named collision layers (32 entries, index = bit position).
    #[serde(default)]
    pub collision_layers: CollisionLayerSettings,
    /// Runtime console variables.  Key = cvar name, value = string representation.
    #[serde(default)]
    pub cvars:    std::collections::HashMap<String, String>,
}

// ── Main project config (.fluxproj) ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConfig {
    pub name:          String,
    pub version:       String,
    pub engine:        String,
    pub schema:        u32,
    pub default_scene: String,
    pub settings:      ProjectSettings,
}

impl ProjectConfig {
    pub fn new(name: impl Into<String>, default_scene: impl Into<String>) -> Self {
        Self {
            name:          name.into(),
            version:       "0.1.0".to_string(),
            engine:        "FluxionRS".to_string(),
            schema:        1,
            default_scene: default_scene.into(),
            settings:      ProjectSettings::default(),
        }
    }
}

// ── Recent project entry ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentProject {
    pub name:         String,
    pub path:         String,
    pub last_opened:  String,
}

// ── Native file operations (desktop only) ─────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
pub fn fluxion_config_dir() -> PathBuf {
    dirs_next::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("fluxion")
}

/// Path of the `.fluxproj` file for a given project root.
pub fn project_file_path(root: &Path) -> PathBuf {
    root.join(".fluxproj")
}

/// Load a `.fluxproj` from `root/.fluxproj`.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_project(root: &Path) -> Result<ProjectConfig, String> {
    let p = project_file_path(root);
    let raw = std::fs::read_to_string(&p)
        .map_err(|e| format!("Cannot read '{}': {e}", p.display()))?;
    serde_json::from_str(&raw)
        .map_err(|e| format!("Malformed .fluxproj '{}': {e}", p.display()))
}

/// Write `config` to `root/.fluxproj` (atomic temp-rename).
#[cfg(not(target_arch = "wasm32"))]
pub fn save_project(root: &Path, config: &ProjectConfig) -> Result<(), String> {
    let p   = project_file_path(root);
    let tmp = p.with_extension("fluxproj.tmp");
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Serialize error: {e}"))?;
    std::fs::write(&tmp, &json)
        .map_err(|e| format!("Write '{}': {e}", tmp.display()))?;
    std::fs::rename(&tmp, &p)
        .map_err(|e| format!("Rename failed: {e}"))?;
    Ok(())
}

/// Create a new project: mkdir, write `.fluxproj`, return the config.
#[cfg(not(target_arch = "wasm32"))]
pub fn create_project(root: &Path, name: impl Into<String>) -> Result<ProjectConfig, String> {
    std::fs::create_dir_all(root)
        .map_err(|e| format!("mkdir '{}': {e}", root.display()))?;
    // Create empty scenes sub-directory
    let scenes_dir = root.join("scenes");
    std::fs::create_dir_all(&scenes_dir)
        .map_err(|e| format!("mkdir scenes: {e}"))?;
    let cfg = ProjectConfig::new(name, "scenes/main.scene");
    save_project(root, &cfg)?;
    Ok(cfg)
}

// ── Recent projects ───────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn recent_projects_path() -> PathBuf {
    fluxion_config_dir().join("recent.json")
}

/// Load the list of recent projects (empty vec if none or parse error).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_recent_projects() -> Vec<RecentProject> {
    let p = recent_projects_path();
    std::fs::read_to_string(&p).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Prepend `entry` to the recent projects list and persist it (max 20 entries).
#[cfg(not(target_arch = "wasm32"))]
pub fn push_recent_project(entry: RecentProject) {
    let mut list = load_recent_projects();
    list.retain(|r| r.path != entry.path);
    list.insert(0, entry);
    list.truncate(20);
    save_recent_projects(&list);
}

/// Persist the recent projects list.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_recent_projects(list: &[RecentProject]) {
    let p = recent_projects_path();
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(list) {
        let _ = std::fs::write(&p, json);
    }
}
