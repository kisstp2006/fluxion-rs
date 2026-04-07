// ============================================================
// settings_module.rs — fluxion::settings Rune module
//
// Exposes project settings (backed by .fluxproj) and editor
// preferences (backed by ~/.fluxion/editor_prefs.json) to
// Rune panel scripts.
//
// Thread-locals hold a copy of the live ProjectConfig and
// EditorPrefs. When the Rune script calls save_project_settings()
// or save_editor_prefs(), the dirty flag is set. main.rs drains
// these each frame and writes to disk + applies live changes.
// ============================================================

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use rune::Module;

use fluxion_core::{
    ProjectConfig, EditorPrefs,
};

thread_local! {
    /// Live copy of the project config — set by set_settings_context.
    static SETTINGS_CONFIG: RefCell<Option<ProjectConfig>> = RefCell::new(None);
    /// Live copy of editor prefs — set by set_settings_context.
    static SETTINGS_PREFS:  RefCell<Option<EditorPrefs>>   = RefCell::new(None);
    /// Project root path — needed to write .fluxproj.
    static SETTINGS_ROOT:   RefCell<PathBuf>               = RefCell::new(PathBuf::new());

    /// True when project settings have been mutated since last save/drain.
    static SETTINGS_DIRTY_P: Cell<bool> = Cell::new(false);
    /// True when editor prefs have been mutated since last save/drain.
    static SETTINGS_DIRTY_E: Cell<bool> = Cell::new(false);

    /// Last save error message — shown in the settings UI, auto-cleared.
    static SETTINGS_LAST_ERROR: RefCell<String> = RefCell::new(String::new());
    /// Error display timer in seconds (counts down; cleared when hits 0).
    static SETTINGS_ERR_TIMER:  Cell<f32>       = Cell::new(0.0);

    /// Which settings tab is active in the Project Settings modal.
    static SETTINGS_TAB_PROJECT: RefCell<String> = RefCell::new("Physics".to_string());
    /// Which settings tab is active in the Preferences modal.
    static SETTINGS_TAB_PREFS:   RefCell<String> = RefCell::new("General".to_string());

    /// Flags to show/hide each modal (set by menubar / Ctrl+,).
    static SHOW_PROJECT_SETTINGS: Cell<bool> = Cell::new(false);
    static SHOW_EDITOR_PREFS:     Cell<bool> = Cell::new(false);

    // Default snapshots (cloned at set_settings_context time) for modified-field detection.
    static SETTINGS_DEFAULTS_P: RefCell<Option<ProjectConfig>> = RefCell::new(None);
    static SETTINGS_DEFAULTS_E: RefCell<Option<EditorPrefs>>   = RefCell::new(None);
    // Search query for the settings UI (shared between both panels).
    static SETTINGS_SEARCH: RefCell<String> = RefCell::new(String::new());
}

// ── Built-in CVar definitions ────────────────────────────────────────────────

/// Hardcoded built-in console variables with their default values.
/// These are seeded into every project's cvar map on first load.
pub const BUILTIN_CVARS: &[(&str, &str)] = &[
    ("r.shadows",      "true"),
    ("r.vsync",        "true"),
    ("r.tonemap",      "aces"),
    ("a.master_volume","1.0"),
    ("p.gravity_y",    "-9.81"),
    ("e.show_grid",    "true"),
];

// ── Public host API ────────────────────────────────────────────────────────────

/// Called by main.rs after a project is opened.
pub fn set_settings_context(mut config: ProjectConfig, prefs: EditorPrefs, root: PathBuf) {
    // Seed built-in cvars with defaults if not already stored in the project.
    let mut seeded = false;
    for &(name, default) in BUILTIN_CVARS {
        if !config.settings.cvars.contains_key(name) {
            config.settings.cvars.insert(name.to_string(), default.to_string());
            seeded = true;
        }
    }
    SETTINGS_DEFAULTS_P.with(|d| *d.borrow_mut() = Some(config.clone()));
    SETTINGS_DEFAULTS_E.with(|d| *d.borrow_mut() = Some(prefs.clone()));
    SETTINGS_CONFIG.with(|c| *c.borrow_mut() = Some(config));
    SETTINGS_PREFS .with(|p| *p.borrow_mut() = Some(prefs));
    SETTINGS_ROOT  .with(|r| *r.borrow_mut() = root);
    SETTINGS_DIRTY_P.with(|d| d.set(seeded));
    SETTINGS_DIRTY_E.with(|d| d.set(false));
    SETTINGS_SEARCH.with(|s| s.borrow_mut().clear());
}

/// Called by main.rs on project close.
#[allow(dead_code)]
pub fn clear_settings_context() {
    SETTINGS_CONFIG.with(|c| *c.borrow_mut() = None);
    SETTINGS_PREFS .with(|p| *p.borrow_mut() = None);
    SETTINGS_DIRTY_P.with(|d| d.set(false));
    SETTINGS_DIRTY_E.with(|d| d.set(false));
}

/// Returns dirty configs for main.rs to write to disk (drains dirty flags).
pub fn drain_settings_saves() -> (Option<(ProjectConfig, PathBuf)>, Option<EditorPrefs>) {
    let proj = if SETTINGS_DIRTY_P.with(|d| d.get()) {
        SETTINGS_DIRTY_P.with(|d| d.set(false));
        let cfg  = SETTINGS_CONFIG.with(|c| c.borrow().clone());
        let root = SETTINGS_ROOT.with(|r| r.borrow().clone());
        cfg.map(|c| (c, root))
    } else {
        None
    };
    let prefs = if SETTINGS_DIRTY_E.with(|d| d.get()) {
        SETTINGS_DIRTY_E.with(|d| d.set(false));
        SETTINGS_PREFS.with(|p| p.borrow().clone())
    } else {
        None
    };
    (proj, prefs)
}

/// Returns the current editor prefs (for immediate live-apply at startup).
#[allow(dead_code)]
pub fn get_current_prefs() -> Option<EditorPrefs> {
    SETTINGS_PREFS.with(|p| p.borrow().clone())
}

#[allow(dead_code)]
pub fn get_show_project_settings()     -> bool { SHOW_PROJECT_SETTINGS.with(|c| c.get()) }
#[allow(dead_code)]
pub fn get_show_editor_prefs()         -> bool { SHOW_EDITOR_PREFS.with(|c| c.get()) }
pub fn set_show_editor_prefs_flag(v: bool)  { SHOW_EDITOR_PREFS.with(|c| c.set(v)); }
#[allow(dead_code)]
pub fn set_show_project_settings_flag(v: bool) { SHOW_PROJECT_SETTINGS.with(|c| c.set(v)); }

// ── UI-module helper API (called from ui_module.rs to render V3 settings UI) ──

pub fn settings_search_query() -> String       { SETTINGS_SEARCH.with(|s| s.borrow().clone()) }
pub fn set_settings_search_query(s: String)    { SETTINGS_SEARCH.with(|sq| *sq.borrow_mut() = s); }

pub fn project_tab() -> String                 { SETTINGS_TAB_PROJECT.with(|t| t.borrow().clone()) }
pub fn set_project_tab_ui(s: String)           { SETTINGS_TAB_PROJECT.with(|t| *t.borrow_mut() = s); }

pub fn prefs_tab() -> String                   { SETTINGS_TAB_PREFS.with(|t| t.borrow().clone()) }
pub fn set_prefs_tab_ui(s: String)             { SETTINGS_TAB_PREFS.with(|t| *t.borrow_mut() = s); }

pub fn close_project_settings_ui()             { SHOW_PROJECT_SETTINGS.with(|c| c.set(false)); }
pub fn close_editor_prefs_ui()                 { SHOW_EDITOR_PREFS.with(|c| c.set(false)); }

pub fn with_project_config<F, T>(f: F) -> Option<T>
where F: FnOnce(&ProjectConfig) -> T
{ SETTINGS_CONFIG.with(|c| c.borrow().as_ref().map(f)) }

pub fn modify_project_config<F>(f: F)
where F: FnOnce(&mut ProjectConfig)
{
    SETTINGS_CONFIG.with(|c| {
        if let Some(cfg) = c.borrow_mut().as_mut() {
            f(cfg);
            SETTINGS_DIRTY_P.with(|d| d.set(true));
        }
    });
}

pub fn with_project_defaults<F, T>(f: F) -> Option<T>
where F: FnOnce(&ProjectConfig) -> T
{ SETTINGS_DEFAULTS_P.with(|d| d.borrow().as_ref().map(f)) }

pub fn with_prefs<F, T>(f: F) -> Option<T>
where F: FnOnce(&EditorPrefs) -> T
{ SETTINGS_PREFS.with(|p| p.borrow().as_ref().map(f)) }

/// Called by world_module::is_protected_editor_cam — does NOT require Rune context.
pub fn get_show_editor_camera() -> bool {
    SETTINGS_PREFS.with(|p| {
        p.borrow().as_ref().map(|pr| pr.show_editor_camera).unwrap_or(false)
    })
}

pub fn modify_prefs<F>(f: F)
where F: FnOnce(&mut EditorPrefs)
{
    SETTINGS_PREFS.with(|p| {
        if let Some(prefs) = p.borrow_mut().as_mut() {
            f(prefs);
            SETTINGS_DIRTY_E.with(|d| d.set(true));
        }
    });
}

pub fn with_prefs_defaults<F, T>(f: F) -> Option<T>
where F: FnOnce(&EditorPrefs) -> T
{ SETTINGS_DEFAULTS_E.with(|d| d.borrow().as_ref().map(f)) }

pub fn reset_project_to_defaults() {
    SETTINGS_DEFAULTS_P.with(|d| {
        if let Some(def) = d.borrow().clone() {
            SETTINGS_CONFIG.with(|c| *c.borrow_mut() = Some(def));
            SETTINGS_DIRTY_P.with(|dd| dd.set(true));
        }
    });
}

pub fn reset_prefs_to_defaults() {
    SETTINGS_DEFAULTS_E.with(|d| {
        if let Some(def) = d.borrow().clone() {
            SETTINGS_PREFS.with(|p| *p.borrow_mut() = Some(def));
            SETTINGS_DIRTY_E.with(|dd| dd.set(true));
        }
    });
}

pub fn project_category_modified_count(cat: &str) -> usize {
    let cur = match SETTINGS_CONFIG.with(|c| c.borrow().clone())   { Some(c) => c, None => return 0 };
    let def = match SETTINGS_DEFAULTS_P.with(|d| d.borrow().clone()) { Some(d) => d, None => return 0 };
    match cat {
        "Physics" => {
            let mut n = 0usize;
            if cur.settings.physics.gravity != def.settings.physics.gravity { n += 1; }
            if (cur.settings.physics.fixed_timestep - def.settings.physics.fixed_timestep).abs() > 1e-6 { n += 1; }
            n
        }
        "Rendering" => {
            let mut n = 0usize;
            if cur.settings.render.shadows         != def.settings.render.shadows         { n += 1; }
            if cur.settings.render.shadow_map_size != def.settings.render.shadow_map_size { n += 1; }
            if cur.settings.render.tone_mapping    != def.settings.render.tone_mapping    { n += 1; }
            if (cur.settings.render.exposure - def.settings.render.exposure).abs() > 1e-4 { n += 1; }
            if cur.settings.editor.show_grid != def.settings.editor.show_grid { n += 1; }
            if (cur.settings.editor.snap_translation - def.settings.editor.snap_translation).abs() > 1e-4 { n += 1; }
            if (cur.settings.editor.snap_rotation    - def.settings.editor.snap_rotation).abs()    > 1e-4 { n += 1; }
            if (cur.settings.editor.snap_scale       - def.settings.editor.snap_scale).abs()       > 1e-4 { n += 1; }
            n
        }
        "Audio" => {
            let mut n = 0usize;
            if (cur.settings.audio.master_volume - def.settings.audio.master_volume).abs() > 1e-4 { n += 1; }
            if (cur.settings.audio.music_volume  - def.settings.audio.music_volume).abs()  > 1e-4 { n += 1; }
            if (cur.settings.audio.sfx_volume    - def.settings.audio.sfx_volume).abs()    > 1e-4 { n += 1; }
            n
        }
        "Input" => {
            let mut n = 0usize;
            if (cur.settings.input.mouse_sensitivity - def.settings.input.mouse_sensitivity).abs() > 1e-4 { n += 1; }
            if (cur.settings.input.gamepad_deadzone  - def.settings.input.gamepad_deadzone).abs()  > 1e-4 { n += 1; }
            n
        }
        "Tags & Layers" => { if cur.settings.tags.list != def.settings.tags.list { 1 } else { 0 } }
        "Build" => {
            let mut n = 0usize;
            if cur.settings.build.target_platform != def.settings.build.target_platform { n += 1; }
            if cur.settings.build.window_title    != def.settings.build.window_title    { n += 1; }
            if cur.settings.build.window_width    != def.settings.build.window_width    { n += 1; }
            if cur.settings.build.window_height   != def.settings.build.window_height   { n += 1; }
            if cur.settings.build.vsync           != def.settings.build.vsync           { n += 1; }
            if cur.settings.build.fullscreen      != def.settings.build.fullscreen      { n += 1; }
            n
        }
        _ => 0,
    }
}

pub fn prefs_category_modified_count(cat: &str) -> usize {
    let cur = match SETTINGS_PREFS.with(|p| p.borrow().clone())      { Some(p) => p, None => return 0 };
    let def = match SETTINGS_DEFAULTS_E.with(|d| d.borrow().clone()) { Some(d) => d, None => return 0 };
    match cat {
        "General" => {
            let mut n = 0usize;
            if cur.theme != def.theme { n += 1; }
            if (cur.font_size - def.font_size).abs() > 0.1 { n += 1; }
            if cur.autosave_interval_secs != def.autosave_interval_secs { n += 1; }
            if cur.restore_layout != def.restore_layout { n += 1; }
            n
        }
        "Camera" => {
            let mut n = 0usize;
            if (cur.camera_speed       - def.camera_speed).abs()       > 1e-4 { n += 1; }
            if (cur.camera_sensitivity - def.camera_sensitivity).abs() > 1e-4 { n += 1; }
            if cur.show_editor_camera  != def.show_editor_camera                { n += 1; }
            n
        }
        "Console" => { if cur.log_max_entries != def.log_max_entries { 1 } else { 0 } }
        "Asset Browser" => { if cur.asset_view_mode != def.asset_view_mode { 1 } else { 0 } }
        _ => 0,
    }
}

pub fn validate_project() -> Vec<String> {
    SETTINGS_CONFIG.with(|c| {
        c.borrow().as_ref().map(|cfg| cfg.settings.build.validate()).unwrap_or_default()
    })
}

// ── Helper macros ──────────────────────────────────────────────────────────────

macro_rules! get_project_field {
    ($section:ident, $field:ident, $default:expr) => {
        SETTINGS_CONFIG.with(|c| {
            c.borrow()
                .as_ref()
                .map(|cfg| cfg.settings.$section.$field.clone())
                .unwrap_or($default)
        })
    };
}

macro_rules! set_project_field {
    ($section:ident, $field:ident, $value:expr) => {
        SETTINGS_CONFIG.with(|c| {
            if let Some(cfg) = c.borrow_mut().as_mut() {
                cfg.settings.$section.$field = $value;
                SETTINGS_DIRTY_P.with(|d| d.set(true));
            }
        });
    };
}

macro_rules! get_prefs_field {
    ($field:ident, $default:expr) => {
        SETTINGS_PREFS.with(|p| {
            p.borrow()
                .as_ref()
                .map(|prefs| prefs.$field.clone())
                .unwrap_or($default)
        })
    };
}

macro_rules! set_prefs_field {
    ($field:ident, $value:expr) => {
        SETTINGS_PREFS.with(|p| {
            if let Some(prefs) = p.borrow_mut().as_mut() {
                prefs.$field = $value;
                SETTINGS_DIRTY_E.with(|d| d.set(true));
            }
        });
    };
}

// ── Rune module builder ────────────────────────────────────────────────────────

pub fn build_settings_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["settings"])?;

    // ── Modal visibility flags ────────────────────────────────────────────────

    m.function("get_show_project_settings", || -> bool {
        SHOW_PROJECT_SETTINGS.with(|c| c.get())
    }).build()?;
    m.function("set_show_project_settings", |v: bool| {
        SHOW_PROJECT_SETTINGS.with(|c| c.set(v));
    }).build()?;
    m.function("get_show_editor_prefs", || -> bool {
        SHOW_EDITOR_PREFS.with(|c| c.get())
    }).build()?;
    m.function("set_show_editor_prefs", |v: bool| {
        SHOW_EDITOR_PREFS.with(|c| c.set(v));
    }).build()?;

    // ── Active tab state ──────────────────────────────────────────────────────

    m.function("get_project_tab", || -> String {
        SETTINGS_TAB_PROJECT.with(|t| t.borrow().clone())
    }).build()?;
    m.function("set_project_tab", |v: String| {
        SETTINGS_TAB_PROJECT.with(|t| *t.borrow_mut() = v);
    }).build()?;
    m.function("get_prefs_tab", || -> String {
        SETTINGS_TAB_PREFS.with(|t| t.borrow().clone())
    }).build()?;
    m.function("set_prefs_tab", |v: String| {
        SETTINGS_TAB_PREFS.with(|t| *t.borrow_mut() = v);
    }).build()?;

    // ── Error status ──────────────────────────────────────────────────────────

    m.function("settings_last_error", || -> String {
        SETTINGS_LAST_ERROR.with(|e| e.borrow().clone())
    }).build()?;
    m.function("settings_tick_error", |dt: f64| {
        SETTINGS_ERR_TIMER.with(|t| {
            let v = (t.get() - dt as f32).max(0.0);
            t.set(v);
            if v <= 0.0 {
                SETTINGS_LAST_ERROR.with(|e| e.borrow_mut().clear());
            }
        });
    }).build()?;

    // ── Physics ───────────────────────────────────────────────────────────────

    m.function("get_gravity", || -> Vec<f64> {
        let g = get_project_field!(physics, gravity, [0.0f32, -9.81, 0.0]);
        vec![g[0] as f64, g[1] as f64, g[2] as f64]
    }).build()?;
    m.function("set_gravity", |vals: Vec<f64>| {
        if vals.len() >= 3 {
            set_project_field!(physics, gravity,
                [vals[0] as f32, vals[1] as f32, vals[2] as f32]);
            // Live-apply to world_module snap (not physics — that's done in drain)
        }
    }).build()?;

    m.function("get_fixed_timestep", || -> f64 {
        get_project_field!(physics, fixed_timestep, 1.0f32 / 60.0) as f64
    }).build()?;
    m.function("set_fixed_timestep", |v: f64| {
        set_project_field!(physics, fixed_timestep, (v as f32).clamp(0.001, 1.0));
    }).build()?;

    // ── Rendering ─────────────────────────────────────────────────────────────

    m.function("get_shadows", || -> bool {
        get_project_field!(render, shadows, true)
    }).build()?;
    m.function("set_shadows", |v: bool| {
        set_project_field!(render, shadows, v);
    }).build()?;

    m.function("get_shadow_map_size", || -> i64 {
        get_project_field!(render, shadow_map_size, 2048u32) as i64
    }).build()?;
    m.function("set_shadow_map_size", |v: i64| {
        let valid = [256u32, 512, 1024, 2048, 4096, 8192];
        let nearest = valid.iter().copied().min_by_key(|&s| (s as i64 - v).abs()).unwrap_or(2048);
        set_project_field!(render, shadow_map_size, nearest);
    }).build()?;

    m.function("get_tone_mapping", || -> String {
        get_project_field!(render, tone_mapping, "ACES".to_string())
    }).build()?;
    m.function("set_tone_mapping", |v: String| {
        set_project_field!(render, tone_mapping, v);
    }).build()?;

    m.function("get_exposure", || -> f64 {
        get_project_field!(render, exposure, 1.2f32) as f64
    }).build()?;
    m.function("set_exposure", |v: f64| {
        set_project_field!(render, exposure, (v as f32).clamp(0.0, 10.0));
    }).build()?;

    // ── Editor / Snap ─────────────────────────────────────────────────────────

    m.function("get_snap_translation", || -> f64 {
        get_project_field!(editor, snap_translation, 1.0f32) as f64
    }).build()?;
    m.function("set_snap_translation", |v: f64| {
        let clamped = (v as f32).clamp(0.001, 100.0);
        set_project_field!(editor, snap_translation, clamped);
        // Live-apply
        crate::rune_bindings::world_module::set_snap_translate_value(clamped as f64);
    }).build()?;

    m.function("get_snap_rotation", || -> f64 {
        get_project_field!(editor, snap_rotation, 15.0f32) as f64
    }).build()?;
    m.function("set_snap_rotation", |v: f64| {
        let clamped = (v as f32).clamp(0.1, 180.0);
        set_project_field!(editor, snap_rotation, clamped);
        crate::rune_bindings::world_module::set_snap_rotate_value(clamped as f64);
    }).build()?;

    m.function("get_snap_scale", || -> f64 {
        get_project_field!(editor, snap_scale, 0.25f32) as f64
    }).build()?;
    m.function("set_snap_scale", |v: f64| {
        let clamped = (v as f32).clamp(0.001, 10.0);
        set_project_field!(editor, snap_scale, clamped);
        crate::rune_bindings::world_module::set_snap_scale_value(clamped as f64);
    }).build()?;

    m.function("get_show_grid", || -> bool {
        get_project_field!(editor, show_grid, true)
    }).build()?;
    m.function("set_show_grid", |v: bool| {
        set_project_field!(editor, show_grid, v);
    }).build()?;

    // ── Audio ─────────────────────────────────────────────────────────────────

    m.function("get_master_volume", || -> f64 {
        get_project_field!(audio, master_volume, 1.0f32) as f64
    }).build()?;
    m.function("set_master_volume", |v: f64| {
        set_project_field!(audio, master_volume, (v as f32).clamp(0.0, 1.0));
    }).build()?;

    m.function("get_music_volume", || -> f64 {
        get_project_field!(audio, music_volume, 1.0f32) as f64
    }).build()?;
    m.function("set_music_volume", |v: f64| {
        set_project_field!(audio, music_volume, (v as f32).clamp(0.0, 1.0));
    }).build()?;

    m.function("get_sfx_volume", || -> f64 {
        get_project_field!(audio, sfx_volume, 1.0f32) as f64
    }).build()?;
    m.function("set_sfx_volume", |v: f64| {
        set_project_field!(audio, sfx_volume, (v as f32).clamp(0.0, 1.0));
    }).build()?;

    // ── Input ─────────────────────────────────────────────────────────────────

    m.function("get_mouse_sensitivity", || -> f64 {
        get_project_field!(input, mouse_sensitivity, 1.0f32) as f64
    }).build()?;
    m.function("set_mouse_sensitivity", |v: f64| {
        set_project_field!(input, mouse_sensitivity, (v as f32).clamp(0.05, 10.0));
    }).build()?;

    m.function("get_gamepad_deadzone", || -> f64 {
        get_project_field!(input, gamepad_deadzone, 0.15f32) as f64
    }).build()?;
    m.function("set_gamepad_deadzone", |v: f64| {
        set_project_field!(input, gamepad_deadzone, (v as f32).clamp(0.0, 0.9));
    }).build()?;

    // ── Tags ──────────────────────────────────────────────────────────────────

    m.function("get_tags", || -> Vec<String> {
        SETTINGS_CONFIG.with(|c| {
            c.borrow()
                .as_ref()
                .map(|cfg| cfg.settings.tags.list.clone())
                .unwrap_or_default()
        })
    }).build()?;

    m.function("add_tag", |tag: String| {
        let tag = tag.trim().to_string();
        if tag.is_empty() { return; }
        SETTINGS_CONFIG.with(|c| {
            if let Some(cfg) = c.borrow_mut().as_mut() {
                if !cfg.settings.tags.list.contains(&tag) {
                    cfg.settings.tags.list.push(tag);
                    SETTINGS_DIRTY_P.with(|d| d.set(true));
                }
            }
        });
    }).build()?;

    m.function("remove_tag", |tag: String| {
        SETTINGS_CONFIG.with(|c| {
            if let Some(cfg) = c.borrow_mut().as_mut() {
                cfg.settings.tags.list.retain(|t| t != &tag);
                SETTINGS_DIRTY_P.with(|d| d.set(true));
            }
        });
    }).build()?;

    // ── Build ─────────────────────────────────────────────────────────────────

    m.function("get_target_platform", || -> String {
        get_project_field!(build, target_platform, "Windows".to_string())
    }).build()?;
    m.function("set_target_platform", |v: String| {
        set_project_field!(build, target_platform, v);
    }).build()?;

    m.function("get_window_title", || -> String {
        get_project_field!(build, window_title, String::new())
    }).build()?;
    m.function("set_window_title", |v: String| {
        set_project_field!(build, window_title, v);
    }).build()?;

    m.function("get_window_width", || -> i64 {
        get_project_field!(build, window_width, 1920u32) as i64
    }).build()?;
    m.function("set_window_width", |v: i64| {
        set_project_field!(build, window_width, v.max(1) as u32);
    }).build()?;

    m.function("get_window_height", || -> i64 {
        get_project_field!(build, window_height, 1080u32) as i64
    }).build()?;
    m.function("set_window_height", |v: i64| {
        set_project_field!(build, window_height, v.max(1) as u32);
    }).build()?;

    m.function("get_vsync", || -> bool {
        get_project_field!(build, vsync, true)
    }).build()?;
    m.function("set_vsync", |v: bool| {
        set_project_field!(build, vsync, v);
    }).build()?;

    m.function("get_fullscreen", || -> bool {
        get_project_field!(build, fullscreen, false)
    }).build()?;
    m.function("set_fullscreen", |v: bool| {
        set_project_field!(build, fullscreen, v);
    }).build()?;

    // ── Project settings validation + save/revert ─────────────────────────────

    m.function("project_settings_validate", || -> Vec<String> {
        SETTINGS_CONFIG.with(|c| {
            c.borrow()
                .as_ref()
                .map(|cfg| cfg.settings.build.validate())
                .unwrap_or_default()
        })
    }).build()?;

    m.function("project_settings_dirty", || -> bool {
        SETTINGS_DIRTY_P.with(|d| d.get())
    }).build()?;

    m.function("save_project_settings", || -> bool {
        // Validate first
        let errors = SETTINGS_CONFIG.with(|c| {
            c.borrow().as_ref().map(|cfg| cfg.settings.build.validate()).unwrap_or_default()
        });
        if !errors.is_empty() {
            let msg = format!("⚠ {}", errors.join("; "));
            SETTINGS_LAST_ERROR.with(|e| *e.borrow_mut() = msg);
            SETTINGS_ERR_TIMER.with(|t| t.set(4.0));
            return false;
        }
        SETTINGS_DIRTY_P.with(|d| d.set(true));
        true
    }).build()?;

    m.function("revert_project_settings", || {
        let root = SETTINGS_ROOT.with(|r| r.borrow().clone());
        match fluxion_core::load_project(&root) {
            Ok(cfg) => {
                SETTINGS_CONFIG.with(|c| *c.borrow_mut() = Some(cfg));
                SETTINGS_DIRTY_P.with(|d| d.set(false));
            }
            Err(e) => {
                let msg = format!("⚠ Revert failed: {e}");
                SETTINGS_LAST_ERROR.with(|err| *err.borrow_mut() = msg);
                SETTINGS_ERR_TIMER.with(|t| t.set(4.0));
            }
        }
    }).build()?;

    // ── Project name (read-only) ───────────────────────────────────────────────

    m.function("get_project_name", || -> String {
        SETTINGS_CONFIG.with(|c| {
            c.borrow().as_ref().map(|cfg| cfg.name.clone()).unwrap_or_default()
        })
    }).build()?;

    // ── Editor Preferences ────────────────────────────────────────────────────

    m.function("get_pref_theme", || -> String {
        get_prefs_field!(theme, "dark".to_string())
    }).build()?;
    m.function("set_pref_theme", |v: String| {
        let v = if v == "light" { "light".to_string() } else { "dark".to_string() };
        set_prefs_field!(theme, v);
    }).build()?;

    m.function("get_pref_font_size", || -> f64 {
        get_prefs_field!(font_size, 13.0f32) as f64
    }).build()?;
    m.function("set_pref_font_size", |v: f64| {
        set_prefs_field!(font_size, (v as f32).clamp(9.0, 24.0));
    }).build()?;

    m.function("get_pref_autosave", || -> i64 {
        get_prefs_field!(autosave_interval_secs, 120u32) as i64
    }).build()?;
    m.function("set_pref_autosave", |v: i64| {
        set_prefs_field!(autosave_interval_secs, v.max(0) as u32);
    }).build()?;

    m.function("get_pref_restore_layout", || -> bool {
        get_prefs_field!(restore_layout, true)
    }).build()?;
    m.function("set_pref_restore_layout", |v: bool| {
        set_prefs_field!(restore_layout, v);
    }).build()?;

    m.function("get_pref_log_max", || -> i64 {
        get_prefs_field!(log_max_entries, 10_000u32) as i64
    }).build()?;
    m.function("set_pref_log_max", |v: i64| {
        set_prefs_field!(log_max_entries, (v.clamp(100, 100_000)) as u32);
    }).build()?;

    m.function("get_pref_camera_speed", || -> f64 {
        get_prefs_field!(camera_speed, 5.0f32) as f64
    }).build()?;
    m.function("set_pref_camera_speed", |v: f64| {
        let clamped = (v as f32).clamp(0.1, 500.0);
        set_prefs_field!(camera_speed, clamped);
        // Live-apply camera speed
        crate::rune_bindings::world_module::set_editor_cam_speed(clamped as f64);
    }).build()?;

    m.function("get_pref_camera_sensitivity", || -> f64 {
        get_prefs_field!(camera_sensitivity, 1.0f32) as f64
    }).build()?;
    m.function("set_pref_camera_sensitivity", |v: f64| {
        set_prefs_field!(camera_sensitivity, (v as f32).clamp(0.05, 10.0));
    }).build()?;

    m.function("show_editor_camera", || -> bool {
        get_prefs_field!(show_editor_camera, false)
    }).build()?;
    m.function("set_show_editor_camera", |v: bool| {
        set_prefs_field!(show_editor_camera, v);
    }).build()?;

    m.function("editor_prefs_dirty", || -> bool {
        SETTINGS_DIRTY_E.with(|d| d.get())
    }).build()?;

    m.function("save_editor_prefs", || -> bool {
        SETTINGS_DIRTY_E.with(|d| d.set(true));
        true
    }).build()?;

    m.function("revert_editor_prefs", || {
        let prefs = fluxion_core::load_editor_prefs();
        SETTINGS_PREFS.with(|p| *p.borrow_mut() = Some(prefs));
        SETTINGS_DIRTY_E.with(|d| d.set(false));
    }).build()?;

    // ── CVar system ───────────────────────────────────────────────────────────

    m.function("cvar_get", |name: String| -> String {
        SETTINGS_CONFIG.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|cfg| cfg.settings.cvars.get(&name).cloned())
                .unwrap_or_default()
        })
    }).build()?;

    m.function("cvar_set", |name: String, value: String| {
        SETTINGS_CONFIG.with(|c| {
            if let Some(cfg) = c.borrow_mut().as_mut() {
                cfg.settings.cvars.insert(name, value);
                SETTINGS_DIRTY_P.with(|d| d.set(true));
            }
        });
    }).build()?;

    m.function("cvar_get_float", |name: String| -> f64 {
        SETTINGS_CONFIG.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|cfg| cfg.settings.cvars.get(&name))
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0)
        })
    }).build()?;

    m.function("cvar_set_float", |name: String, value: f64| {
        SETTINGS_CONFIG.with(|c| {
            if let Some(cfg) = c.borrow_mut().as_mut() {
                cfg.settings.cvars.insert(name, value.to_string());
                SETTINGS_DIRTY_P.with(|d| d.set(true));
            }
        });
    }).build()?;

    m.function("cvar_get_bool", |name: String| -> bool {
        SETTINGS_CONFIG.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|cfg| cfg.settings.cvars.get(&name))
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false)
        })
    }).build()?;

    m.function("cvar_set_bool", |name: String, value: bool| {
        SETTINGS_CONFIG.with(|c| {
            if let Some(cfg) = c.borrow_mut().as_mut() {
                cfg.settings.cvars.insert(name, if value { "true".into() } else { "false".into() });
                SETTINGS_DIRTY_P.with(|d| d.set(true));
            }
        });
    }).build()?;

    m.function("cvar_unset", |name: String| {
        SETTINGS_CONFIG.with(|c| {
            if let Some(cfg) = c.borrow_mut().as_mut() {
                cfg.settings.cvars.remove(&name);
                SETTINGS_DIRTY_P.with(|d| d.set(true));
            }
        });
    }).build()?;

    m.function("cvar_list", || -> Vec<Vec<String>> {
        SETTINGS_CONFIG.with(|c| {
            c.borrow()
                .as_ref()
                .map(|cfg| {
                    let mut pairs: Vec<Vec<String>> = cfg.settings.cvars.iter()
                        .map(|(k, v)| vec![k.clone(), v.clone()])
                        .collect();
                    pairs.sort_by(|a, b| a[0].cmp(&b[0]));
                    pairs
                })
                .unwrap_or_default()
        })
    }).build()?;

    // ── Built-in CVar helpers ──────────────────────────────────────────────────

    m.function("cvar_builtin_names", || -> Vec<String> {
        BUILTIN_CVARS.iter().map(|&(name, _)| name.to_string()).collect()
    }).build()?;

    m.function("cvar_builtin_default", |name: String| -> String {
        BUILTIN_CVARS.iter()
            .find(|&&(n, _)| n == name)
            .map(|&(_, d)| d.to_string())
            .unwrap_or_default()
    }).build()?;

    // ── Collision layer names ─────────────────────────────────────────────────

    m.function("layer_name", |index: i64| -> String {
        SETTINGS_CONFIG.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|cfg| cfg.settings.collision_layers.names.get(index as usize).cloned())
                .unwrap_or_else(|| format!("Layer {}", index))
        })
    }).build()?;

    m.function("layer_names", || -> Vec<String> {
        SETTINGS_CONFIG.with(|c| {
            c.borrow()
                .as_ref()
                .map(|cfg| cfg.settings.collision_layers.names.clone())
                .unwrap_or_else(|| (0..32).map(|i: usize| format!("Layer {}", i)).collect())
        })
    }).build()?;

    m.function("layer_index", |name: String| -> i64 {
        SETTINGS_CONFIG.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|cfg| cfg.settings.collision_layers.names.iter().position(|n| *n == name))
                .map(|i| i as i64)
                .unwrap_or(-1)
        })
    }).build()?;

    Ok(m)
}
