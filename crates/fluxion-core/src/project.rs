// ============================================================
// fluxion-core — Project system
//
// Mirrors FluxionJsV3's ProjectManager in Rust.
// A project is a folder containing a `.fluxproj` JSON file.
// Recent projects are stored in `~/.fluxion/recent.json`.
// ============================================================

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

// ── Aggregated project settings ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectSettings {
    pub physics:  ProjectPhysicsSettings,
    pub render:   ProjectRenderSettings,
    pub editor:   ProjectEditorSettings,
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
