// ============================================================
// world_module.rs — fluxion::world Rune module
//
// Exposes ECS entity/component inspection and mutation to Rune
// panel scripts.  Thread-locals hold a read-only borrow of the
// ECSWorld + ComponentRegistry for the duration of each panel
// call.  Mutations are queued as PendingEdits and applied by
// host.rs after the Rune call returns (requires &mut ECSWorld).
// ============================================================

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::OnceLock;
use std::time::Instant;

use rune::{Module, runtime::Ref};

use fluxion_core::{
    AssetDatabase,
    ComponentRegistry, ECSWorld, EntityId,
    reflect::{FieldDescriptor, ReflectFieldType, ReflectValue},
};
use glam::EulerRot;

// ── Log infrastructure ───────────────────────────────────────────────────────

static LOG_START: OnceLock<Instant> = OnceLock::new();

#[derive(Clone, PartialEq)]
enum LogLevel { Info, Warn, Error }

impl LogLevel {
    fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Info  => "info",
            LogLevel::Warn  => "warn",
            LogLevel::Error => "error",
        }
    }
    fn from_line(line: &str) -> (Self, String) {
        if let Some(rest) = line.strip_prefix("[ERROR]") {
            (LogLevel::Error, rest.trim_start().to_string())
        } else if let Some(rest) = line.strip_prefix("[WARN]") {
            (LogLevel::Warn,  rest.trim_start().to_string())
        } else if let Some(rest) = line.strip_prefix("[INFO]") {
            (LogLevel::Info,  rest.trim_start().to_string())
        } else if let Some(rest) = line.strip_prefix("[LOG]") {
            (LogLevel::Info,  rest.trim_start().to_string())
        } else {
            (LogLevel::Info, line.to_string())
        }
    }
}

#[derive(Clone)]
struct LogEntry {
    level:   LogLevel,
    message: String,
    time_ms: u64,
}

fn elapsed_ms() -> u64 {
    LOG_START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

fn format_log_time(ms: u64) -> String {
    let secs   = ms / 1000;
    let millis = ms % 1000;
    let mins   = secs / 60;
    let secs   = secs % 60;
    format!("{:02}:{:02}.{:03}", mins, secs, millis)
}

// ── Thread-local context ──────────────────────────────────────────────────────

thread_local! {
    static WORLD_PTR:    Cell<Option<NonNull<ECSWorld>>>          = Cell::new(None);
    static REG_PTR:      Cell<Option<NonNull<ComponentRegistry>>> = Cell::new(None);
    static SELECTED:     RefCell<Option<EntityId>>               = RefCell::new(None);
    static PENDING:      RefCell<Vec<PendingEdit>>               = RefCell::new(Vec::new());
    /// Structured log entries — replaces the old flat Vec<String>.
    static LOG_ENTRIES:  RefCell<Vec<LogEntry>> = RefCell::new(Vec::new());
    /// Index of the selected log entry in the console detail panel (-1 = none).
    static LOG_SELECTED: Cell<i64>              = Cell::new(-1);
    // ── Console filter state (persisted between frames) ───────────────────────
    static CONSOLE_SHOW_INFO:   Cell<bool>      = Cell::new(true);
    static CONSOLE_SHOW_WARN:   Cell<bool>      = Cell::new(true);
    static CONSOLE_SHOW_ERROR:  Cell<bool>      = Cell::new(true);
    static CONSOLE_COLLAPSE:    Cell<bool>      = Cell::new(false);
    static CONSOLE_AUTO_SCROLL: Cell<bool>      = Cell::new(true);
    static CONSOLE_SEARCH:      RefCell<String> = RefCell::new(String::new());
    /// Per-frame cache: entity bits → EntityId. Rebuilt each frame in set_world_context.
    static ENTITY_CACHE: RefCell<HashMap<u64, EntityId>>         = RefCell::new(HashMap::new());
    /// Monotonic counter incremented on every push_log / clear_log call.
    /// Rune panels compare against their last-seen value to skip redundant clones.
    static LOG_GENERATION: Cell<u64> = Cell::new(0);
    /// Project root directory path — set once at editor startup.
    static PROJECT_ROOT: RefCell<PathBuf> = RefCell::new(PathBuf::new());
    /// Active directory in the asset browser right panel.
    static ASSET_ACTIVE_DIR: RefCell<String> = RefCell::new(String::new());
    /// Type filter active in asset browser ("" = All).
    static ASSET_TYPE_FILTER: RefCell<String> = RefCell::new(String::new());
    /// Tile zoom level for the asset grid (1.0 = 64 px).
    static ASSET_ZOOM: Cell<f64> = Cell::new(1.0);
    /// (can_undo, can_redo) — pushed each frame by EditorHost.
    static UNDO_STATE: Cell<(bool, bool)> = Cell::new((false, false));
    /// Last frame delta time in milliseconds.
    static FRAME_TIME_MS: Cell<f64> = Cell::new(0.0);
    /// Countdown timer (seconds) for the script hot-reload toast badge.
    static HOTRELOAD_TOAST_TIMER: Cell<f64> = Cell::new(0.0);
    /// Total elapsed time in seconds — accumulated by set_time_elapsed each frame.
    static TIME_ELAPSED: Cell<f64> = Cell::new(0.0);
    /// Editor mode string: "Editing" | "Playing" | "Paused".
    static EDITOR_MODE: RefCell<String> = RefCell::new(String::from("Editing"));
    /// Transform tool string: "Translate" | "Rotate" | "Scale".
    static TRANSFORM_TOOL: RefCell<String> = RefCell::new(String::from("Translate"));
    /// AssetDatabase pointer — set by set_asset_db_context each frame.
    static ASSET_DB_PTR: Cell<Option<NonNull<AssetDatabase>>> = Cell::new(None);
    /// Persistent search query for the asset browser panel.
    static ASSET_SEARCH_QUERY: RefCell<String> = RefCell::new(String::new());
    /// Currently selected asset (project-relative path).  Empty = nothing selected.
    static SELECTED_ASSET_PATH: RefCell<String> = RefCell::new(String::new());
    /// Inline creation mode for the asset panel: "" | "dir" | "file".
    static ASSET_CREATE_MODE:  RefCell<String> = RefCell::new(String::new());
    /// Text input buffer for the inline asset creation form.
    static ASSET_CREATE_INPUT: RefCell<String> = RefCell::new(String::new());
    /// Project name for display in toolbar.
    static PROJECT_NAME: RefCell<String> = RefCell::new(String::new());
    /// Scene name for display in toolbar.
    static SCENE_NAME: RefCell<String> = RefCell::new(String::new());
    /// Signals queued by Rune scripts for main.rs to consume.
    static ACTION_SIGNALS: RefCell<Vec<String>> = RefCell::new(Vec::new());
    /// Asset browser view mode: "tile" | "list".
    static ASSET_VIEW_MODE: RefCell<String> = RefCell::new(String::from("tile"));

    // ── Editor camera state (persisted between frames, mutated by editor_camera.rn) ──
    static EDITOR_CAM: RefCell<EditorCameraState> = RefCell::new(EditorCameraState::default());
    /// True when the editor camera has been mutated this frame (main.rs reads this to push to Transform).
    static EDITOR_CAM_DIRTY: Cell<bool> = Cell::new(false);

    // ── Viewport stats ────────────────────────────────────────────────────────
    /// (draw_calls, entity_count) — updated each frame from main.rs.
    static FRAME_STATS: Cell<(u32, u32)> = Cell::new((0, 0));

    // ── Snap settings ─────────────────────────────────────────────────────────
    static SNAP_ENABLED:   Cell<bool> = Cell::new(false);
    static SNAP_TRANSLATE: Cell<f64>  = Cell::new(0.5);
    static SNAP_ROTATE:    Cell<f64>  = Cell::new(15.0);
    static SNAP_SCALE:     Cell<f64>  = Cell::new(0.1);

    // ── Multi-selection ───────────────────────────────────────────────────────
    static SELECTED_MULTI: RefCell<Vec<EntityId>> = RefCell::new(Vec::new());

    // ── Prefab creation dialog ────────────────────────────────────────────────
    /// Entity ID bits pending prefab save (set when dialog opens, read on confirm).
    static PREFAB_PENDING: Cell<i64> = Cell::new(-1);

    // ── CSG box gizmo ─────────────────────────────────────────────────────────
    /// 0 = none, 1 = BoxFaceHandles, 2 = BoxAxisArrows
    static BOX_GIZMO_MODE: Cell<u8> = Cell::new(0);

    // ── Console command bar ───────────────────────────────────────────────────
    static CONSOLE_CMD_BUF: RefCell<String> = RefCell::new(String::new());

    // ── Asset browser navigation history ─────────────────────────────────────
    /// Ordered list of visited directories (newest at highest index).
    static ASSET_NAV_HISTORY: RefCell<Vec<String>> = RefCell::new(Vec::new());
    /// Current position in ASSET_NAV_HISTORY (index). -1 = uninitialized.
    static ASSET_NAV_POS: Cell<i64> = Cell::new(-1);

    // ── Editor camera entity tracking ─────────────────────────────────────────
    /// EntityId bits of the active Camera entity used as the editor fly-cam.
    /// When set, this entity is hidden from the hierarchy (unless show_editor_camera pref is on)
    /// and cannot be selected via normal editor tools.
    static EDITOR_CAM_ID: Cell<Option<EntityId>> = Cell::new(None);

    // ── Asset reference registry ───────────────────────────────────────────────
    /// GUID → (current_path, refs: [[entity_id, entity_name, component, field], ...])
    /// Updated by rescan_asset_refs() / load_asset_refs(). Read by Rune bindings.
    static ASSET_REF_CACHE: RefCell<HashMap<String, (String, Vec<Vec<String>>)>>
        = RefCell::new(HashMap::new());
}

/// A deferred field mutation queued by Rune, applied after the panel call.
pub struct PendingEdit {
    pub entity:    EntityId,
    pub component: String,
    pub field:     String,
    pub value:     ReflectValue,
}

// ── Public host API ────────────────────────────────────────────────────────────

/// RAII guard returned by `set_world_context`.
/// Clears all world-related thread-locals on drop, even if a panic unwinds
/// through the render closure.
pub struct WorldContextGuard;

impl Drop for WorldContextGuard {
    fn drop(&mut self) {
        WORLD_PTR   .with(|c| c.set(None));
        REG_PTR     .with(|c| c.set(None));
        ENTITY_CACHE.with(|cache| cache.borrow_mut().clear());
        ASSET_DB_PTR.with(|c| c.set(None));
    }
}

/// Set the AssetDatabase pointer for the current Rune call frame.
/// # Safety: pointer must remain valid for the duration of the guard lifetime.
pub fn set_asset_db_context(db: &AssetDatabase) {
    ASSET_DB_PTR.with(|c| c.set(Some(NonNull::from(db))));
}

/// Clear the AssetDatabase pointer (called from clear_rune_context).
pub fn clear_asset_db_context() {
    ASSET_DB_PTR.with(|c| c.set(None));
}

/// Set world + registry pointers before a Rune panel call.
/// Also rebuilds the entity ID cache for O(1) lookups this frame.
/// Returns a `WorldContextGuard` that clears the pointers on drop.
/// # Safety: pointers must remain valid for the lifetime of the guard.
pub fn set_world_context(world: &ECSWorld, registry: &ComponentRegistry) -> WorldContextGuard {
    SELECTED_MULTI.with(|s| {
        s.borrow_mut().retain(|e| world.all_entities().any(|w| w == *e));
    });
    WORLD_PTR.with(|c| c.set(Some(NonNull::from(world))));
    REG_PTR  .with(|c| c.set(Some(NonNull::from(registry))));
    ENTITY_CACHE.with(|cache| {
        let mut map = cache.borrow_mut();
        map.clear();
        for e in world.all_entities() {
            map.insert(e.to_bits(), e);
        }
    });
    WorldContextGuard
}

/// Clear world + registry pointers immediately.
/// Prefer holding the `WorldContextGuard` from `set_world_context` instead.
#[allow(dead_code)]
pub fn clear_world_context() {
    WORLD_PTR   .with(|c| c.set(None));
    REG_PTR     .with(|c| c.set(None));
    ENTITY_CACHE.with(|cache| cache.borrow_mut().clear());
}

/// Drain queued mutations for the host to apply with &mut ECSWorld.
pub fn drain_pending_edits() -> Vec<PendingEdit> {
    PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

/// Call `f` with a shared reference to the current frame's ECSWorld.
/// Returns `None` if the world context is not set.
///
/// Used by gameplay_module.rs to read entity data without importing WORLD_PTR.
pub fn with_world<R>(f: impl FnOnce(&ECSWorld) -> R) -> Option<R> {
    WORLD_PTR.with(|c| {
        let ptr = c.get()?;
        Some(f(unsafe { ptr.as_ref() }))
    })
}

/// Call `f` with a mutable reference to the current frame's ECSWorld.
/// Returns `None` if the world context is not set.
/// SAFETY: caller must not alias — only one mutable borrow at a time.
pub fn with_world_mut<R>(f: impl FnOnce(&mut ECSWorld) -> R) -> Option<R> {
    WORLD_PTR.with(|c| {
        let mut ptr = c.get()?;
        Some(f(unsafe { ptr.as_mut() }))
    })
}

/// Append a log line from Rust host code.
/// Parses the `[LEVEL]` prefix to determine level; caps at 10 000 entries.
pub fn push_log(line: String) {
    let (level, message) = LogLevel::from_line(&line);
    LOG_ENTRIES.with(|l| {
        let mut v = l.borrow_mut();
        if v.len() >= 10_000 {
            v.drain(..1_000);
        }
        v.push(LogEntry { level, message, time_ms: elapsed_ms() });
    });
    LOG_GENERATION.with(|g| g.set(g.get().wrapping_add(1)));
}

/// Get the currently selected entity (may be None).
pub fn get_selected_id() -> Option<EntityId> {
    SELECTED.with(|s| *s.borrow())
}

/// Set the selected entity from Rust host code.
pub fn set_selected_id(entity: Option<EntityId>) {
    SELECTED.with(|s| *s.borrow_mut() = entity);
}

/// Track which entity is the editor fly-camera (hidden from hierarchy).
pub fn set_editor_cam_entity(entity: Option<EntityId>) {
    EDITOR_CAM_ID.with(|c| c.set(entity));
}

/// Returns the editor camera entity bits as i64, or -1 if unset.
pub fn get_editor_cam_entity_id() -> i64 {
    EDITOR_CAM_ID.with(|c| c.get().map(|e| e.to_bits() as i64).unwrap_or(-1))
}

/// Returns true if the given entity id matches the editor camera AND show_editor_camera is false.
fn is_protected_editor_cam(id: i64) -> bool {
    let cam_id = EDITOR_CAM_ID.with(|c| c.get().map(|e| e.to_bits() as i64).unwrap_or(-1));
    if cam_id < 0 || cam_id != id { return false; }
    !crate::rune_bindings::settings_module::get_show_editor_camera()
}

/// Set the project root path so Rune scripts can enumerate assets.
pub fn set_project_root(root: &std::path::Path) {
    PROJECT_ROOT.with(|p| *p.borrow_mut() = root.to_path_buf());
}

/// Open a file using the OS default associated program.
fn open_with_default(path: &std::path::Path) {
    #[cfg(target_os = "windows")]
    { let _ = std::process::Command::new("cmd").args(["/c", "start", "", &path.to_string_lossy()]).spawn(); }
    #[cfg(target_os = "macos")]
    { let _ = std::process::Command::new("open").arg(path).spawn(); }
    #[cfg(target_os = "linux")]
    { let _ = std::process::Command::new("xdg-open").arg(path).spawn(); }
}

/// Get the current project root (used by ui_module for texture preview loading).
pub fn get_project_root() -> std::path::PathBuf {
    PROJECT_ROOT.with(|p| p.borrow().clone())
}

/// Update undo/redo state so Rune scripts can query it.
pub fn set_undo_state(can_undo: bool, can_redo: bool) {
    UNDO_STATE.with(|c| c.set((can_undo, can_redo)));
}

/// Push the last frame delta time (milliseconds) for the debugger panel.
pub fn set_frame_time(ms: f64) {
    FRAME_TIME_MS.with(|c| c.set(ms));
    HOTRELOAD_TOAST_TIMER.with(|t| {
        let remaining = t.get() - ms / 1000.0;
        t.set(remaining.max(0.0));
    });
}

pub fn set_time_elapsed(secs: f64) {
    TIME_ELAPSED.with(|c| c.set(secs));
}

/// Push per-frame stats (draw calls, entity count) for the viewport overlay.
pub fn set_frame_stats(draw_calls: u32, entity_count: u32) {
    FRAME_STATS.with(|c| c.set((draw_calls, entity_count)));
}

/// Read snap settings (for applying during gizmo drag in main.rs).
pub fn get_snap_enabled()   -> bool { SNAP_ENABLED  .with(|c| c.get()) }
pub fn get_snap_translate() -> f64  { SNAP_TRANSLATE.with(|c| c.get()) }
pub fn get_snap_rotate()    -> f64  { SNAP_ROTATE   .with(|c| c.get()) }
pub fn get_snap_scale()     -> f64  { SNAP_SCALE    .with(|c| c.get()) }

/// Live-set snap values from settings_module (called when user edits project settings).
pub fn set_snap_translate_value(v: f64) { SNAP_TRANSLATE.with(|c| c.set(v)); }
pub fn set_snap_rotate_value(v: f64)    { SNAP_ROTATE   .with(|c| c.set(v)); }
pub fn set_snap_scale_value(v: f64)     { SNAP_SCALE    .with(|c| c.set(v)); }

/// Live-set editor camera speed from settings_module (e.g. applied from EditorPrefs).
pub fn set_editor_cam_speed(v: f64) {
    EDITOR_CAM.with(|c| c.borrow_mut().speed = v);
}

/// Initialize asset view mode from EditorPrefs (called on project open and prefs save).
pub fn set_asset_view_mode(mode: &str) {
    let v = if mode == "list" { "list" } else { "tile" };
    ASSET_VIEW_MODE.with(|m| *m.borrow_mut() = v.to_string());
}

/// Read the current CSG box gizmo mode (0=none, 1=FaceHandles, 2=AxisArrows).
pub fn get_box_gizmo_mode_raw() -> u8 { BOX_GIZMO_MODE.with(|c| c.get()) }

/// Directly override the CSG box gizmo mode (called from main.rs to clear stale state).
pub fn set_box_gizmo_mode_raw(v: u8) { BOX_GIZMO_MODE.with(|c| c.set(v)); }

/// Get the list of multi-selected entity IDs (for Ctrl+D, gizmo average pivot).
pub fn get_multi_selected() -> Vec<EntityId> {
    SELECTED_MULTI.with(|s| s.borrow().clone())
}

/// Update editor mode/tool/names — called each frame from EditorHost.
pub fn set_editor_shell_state(mode: &str, tool: &str, project: &str, scene: &str) {
    EDITOR_MODE   .with(|c| *c.borrow_mut() = mode.to_string());
    TRANSFORM_TOOL.with(|c| *c.borrow_mut() = tool.to_string());
    PROJECT_NAME  .with(|c| *c.borrow_mut() = project.to_string());
    SCENE_NAME    .with(|c| *c.borrow_mut() = scene.to_string());
}

/// Drain action signals ("new_scene", "open_scene", "save_scene", "exit", etc.) queued by Rune.
pub fn drain_action_signals() -> Vec<String> {
    ACTION_SIGNALS.with(|s| std::mem::take(&mut *s.borrow_mut()))
}

/// Read the editor mode as set by Rune (may have changed this frame).
pub fn get_editor_mode_str() -> String {
    EDITOR_MODE.with(|c| c.borrow().clone())
}

/// Force-set the editor mode from Rust host code (used to reject invalid transitions).
pub fn force_editor_mode(mode: &str) {
    EDITOR_MODE.with(|c| *c.borrow_mut() = mode.to_string());
}

/// Read the transform tool as set by Rune.
pub fn get_transform_tool_str() -> String {
    TRANSFORM_TOOL.with(|c| c.borrow().clone())
}

// ── Editor camera host API ────────────────────────────────────────────────────

/// Isolated state for the editor fly-cam. Independent of any game Camera entity.
#[derive(Clone)]
pub struct EditorCameraState {
    pub pos:   [f64; 3],
    pub yaw:   f64,
    pub pitch: f64,
    pub fov:   f64,
    pub near:  f64,
    pub far:   f64,
    pub speed: f64,
    pub target: [f64; 3],
}

impl Default for EditorCameraState {
    fn default() -> Self {
        Self {
            pos:    [0.0, 2.0, 8.0],
            yaw:    0.0,
            pitch:  -0.15,
            fov:    60.0,
            near:   0.05,
            far:    2000.0,
            speed:  5.0,
            target: [0.0, 0.0, 0.0],
        }
    }
}

/// Read a snapshot of the full editor camera state (for main.rs to build CameraOverride).
pub fn get_editor_cam_state() -> EditorCameraState {
    EDITOR_CAM.with(|c| c.borrow().clone())
}

/// Read editor camera position [x,y,z] (for main.rs to push to renderer).
pub fn get_editor_cam_pos() -> [f64; 3] {
    EDITOR_CAM.with(|c| c.borrow().pos)
}

/// Read editor camera yaw (radians).
pub fn get_editor_cam_yaw() -> f64 {
    EDITOR_CAM.with(|c| c.borrow().yaw)
}

/// Read editor camera pitch (radians).
pub fn get_editor_cam_pitch() -> f64 {
    EDITOR_CAM.with(|c| c.borrow().pitch)
}

/// Initialize editor camera from the active Camera entity transform (called once at startup).
pub fn init_editor_cam(pos: [f64; 3], yaw: f64, pitch: f64) {
    EDITOR_CAM.with(|c| {
        let mut s = c.borrow_mut();
        s.pos   = pos;
        s.yaw   = yaw;
        s.pitch = pitch;
    });
    EDITOR_CAM_DIRTY.with(|c| c.set(false));
}

/// Returns true if the editor camera was mutated by Rune this frame, then clears the flag.
pub fn take_editor_cam_dirty() -> bool {
    EDITOR_CAM_DIRTY.with(|c| {
        let v = c.get();
        c.set(false);
        v
    })
}

// ── Internal helpers ─────────────────────────────────────────────────────────

fn with_adb<R>(f: impl FnOnce(&AssetDatabase) -> R) -> Option<R> {
    ASSET_DB_PTR.with(|c| c.get().map(|ptr| unsafe { f(ptr.as_ref()) }))
}

fn with_adb_mut<R>(f: impl FnOnce(&mut AssetDatabase) -> R) -> Option<R> {
    ASSET_DB_PTR.with(|c| c.get().map(|mut ptr| unsafe { f(ptr.as_mut()) }))
}

fn with_ctx<R>(f: impl FnOnce(&ECSWorld, &ComponentRegistry) -> R) -> Option<R> {
    let world = WORLD_PTR.with(|c| c.get())?;
    let reg   = REG_PTR  .with(|c| c.get())?;
    // SAFETY: pointers are valid for the panel call duration.
    Some(unsafe { f(world.as_ref(), reg.as_ref()) })
}

fn entity_to_id(id: EntityId) -> i64 {
    id.to_bits() as i64
}

fn id_to_entity(_world: &ECSWorld, id: i64) -> Option<EntityId> {
    entity_from_id(id)
}

/// Resolve a Rune i64 entity handle to an EntityId using the per-frame cache.
/// Returns `None` if the id is unknown or the cache is not set.
pub fn entity_from_id(id: i64) -> Option<EntityId> {
    let bits = id as u64;
    ENTITY_CACHE.with(|cache| cache.borrow().get(&bits).copied())
}

/// Public helper for ui_module: checks whether `ancestor_id` is an ancestor
/// of `entity_id`. Used to show invalid-drop visual feedback during DnD hover.
/// Returns `false` if either ID is unknown / the world is not set.
pub fn check_is_ancestor(ancestor_id: i64, entity_id: i64) -> bool {
    with_ctx(|world, _| {
        let ancestor = id_to_entity(world, ancestor_id)?;
        let entity   = id_to_entity(world, entity_id)?;
        Some(world.is_ancestor_of(ancestor, entity))
    }).flatten().unwrap_or(false)
}

/// Public helper for other modules (e.g. ui_module) to resolve an entity ID
/// to its display name without needing access to the private `with_ctx`.
pub fn entity_name_for_id(id: i64) -> String {
    if id < 0 {
        return "None".to_string();
    }
    with_ctx(|world, _| {
        let entity = id_to_entity(world, id)?;
        Some(world.get_name(entity).to_string())
    }).flatten().unwrap_or_else(|| format!("Entity #{}", id))
}

fn get_descriptor<'a>(
    registry: &'a ComponentRegistry,
    component: &str,
    field: &str,
) -> Option<&'a FieldDescriptor> {
    registry.component_fields(component)?
        .iter()
        .find(|f| f.name == field)
}

fn queue_edit(entity: EntityId, component: String, field: String, value: ReflectValue) {
    PENDING.with(|p| p.borrow_mut().push(PendingEdit { entity, component, field, value }));
}

// ── Asset reference registry ──────────────────────────────────────────────────

fn is_asset_field_type(ft: ReflectFieldType) -> bool {
    matches!(ft,
        ReflectFieldType::Texture  | ReflectFieldType::Material |
        ReflectFieldType::Mesh     | ReflectFieldType::Audio    |
        ReflectFieldType::Scene    | ReflectFieldType::OptionStr
    )
}

/// Returns the asset path from a field value, with a heuristic that the string looks
/// like a relative file path (contains a '.' indicating a file extension).
fn asset_path_from_value(val: &ReflectValue) -> Option<&str> {
    let s = match val {
        ReflectValue::AssetPath(Some(p)) if !p.is_empty() => p.as_str(),
        ReflectValue::OptionStr(Some(p)) if !p.is_empty() => p.as_str(),
        ReflectValue::Str(p)             if !p.is_empty() => p.as_str(),
        _ => return None,
    };
    // Must look like a file path (contains an extension dot) to avoid false positives.
    if s.contains('.') || s.contains('/') { Some(s) } else { None }
}

fn save_asset_registry(
    cache: &HashMap<String, (String, Vec<Vec<String>>)>,
    root:  &std::path::Path,
) {
    let mut obj = serde_json::Map::new();
    for (guid, (path, refs)) in cache {
        let refs_json: Vec<serde_json::Value> = refs.iter().map(|r| {
            let mut ro = serde_json::Map::new();
            ro.insert("entity_id".into(),   serde_json::Value::String(r.get(0).cloned().unwrap_or_default()));
            ro.insert("entity_name".into(), serde_json::Value::String(r.get(1).cloned().unwrap_or_default()));
            ro.insert("component".into(),   serde_json::Value::String(r.get(2).cloned().unwrap_or_default()));
            ro.insert("field".into(),       serde_json::Value::String(r.get(3).cloned().unwrap_or_default()));
            serde_json::Value::Object(ro)
        }).collect();
        let mut entry = serde_json::Map::new();
        entry.insert("path".into(), serde_json::Value::String(path.clone()));
        entry.insert("refs".into(), serde_json::Value::Array(refs_json));
        obj.insert(guid.clone(), serde_json::Value::Object(entry));
    }
    let registry_path = root.join("assets").join("asset_registry.json");
    match serde_json::to_string_pretty(&serde_json::Value::Object(obj)) {
        Ok(json) => { let _ = std::fs::write(&registry_path, json.as_bytes()); }
        Err(e)   => log::warn!("[AssetRef] failed to serialize registry: {e}"),
    }
}

/// Scan all ECS entities for asset-typed fields and rebuild the reference registry.
/// Saves result to `{root}/assets/asset_registry.json` and updates the in-memory cache.
pub fn rescan_asset_refs(
    world:    &ECSWorld,
    registry: &ComponentRegistry,
    adb:      &AssetDatabase,
    root:     &std::path::Path,
) {
    let mut new_cache: HashMap<String, (String, Vec<Vec<String>>)> = HashMap::new();
    for entity in world.all_entities() {
        let entity_name   = world.get_name(entity).to_string();
        let entity_id_str = entity.to_bits().to_string();
        for &comp_name in world.component_names(entity) {
            let Some(fields)  = registry.component_fields(comp_name) else { continue };
            let Some(reflect) = registry.get_reflect(comp_name, world, entity) else { continue };
            for field in fields {
                let Some(val) = reflect.get_field(field.name) else { continue };
                // Include: explicit AssetPath values, OR asset-typed fields with path-like values
                let is_asset_val = matches!(val, ReflectValue::AssetPath(Some(_)));
                if !is_asset_val && !is_asset_field_type(field.field_type) { continue }
                let Some(asset_path) = asset_path_from_value(&val) else { continue };
                let guid = adb.get_by_path(asset_path)
                    .map(|r| r.guid.clone())
                    .unwrap_or_else(|| format!("path:{asset_path}"));
                let entry = new_cache.entry(guid).or_insert_with(|| (asset_path.to_string(), Vec::new()));
                entry.0 = asset_path.to_string();
                entry.1.push(vec![
                    entity_id_str.clone(),
                    entity_name.clone(),
                    comp_name.to_string(),
                    field.name.to_string(),
                ]);
            }
        }
    }
    save_asset_registry(&new_cache, root);
    ASSET_REF_CACHE.with(|c| *c.borrow_mut() = new_cache);
    log::debug!("[AssetRef] registry rescanned");
}

/// Load the asset reference registry from `{root}/assets/asset_registry.json`.
pub fn load_asset_refs(root: &std::path::Path) {
    let registry_path = root.join("assets").join("asset_registry.json");
    let content = match std::fs::read_to_string(&registry_path) {
        Ok(s)  => s,
        Err(_) => return,
    };
    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v)  => v,
        Err(e) => { log::warn!("[AssetRef] failed to parse registry: {e}"); return; }
    };
    let Some(obj) = json.as_object() else { return };
    let mut cache: HashMap<String, (String, Vec<Vec<String>>)> = HashMap::new();
    for (guid, entry) in obj {
        let path_str = entry.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let refs = entry.get("refs")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|r| Some(vec![
                r.get("entity_id")?.as_str()?.to_string(),
                r.get("entity_name")?.as_str()?.to_string(),
                r.get("component")?.as_str()?.to_string(),
                r.get("field")?.as_str()?.to_string(),
            ])).collect::<Vec<_>>())
            .unwrap_or_default();
        cache.insert(guid.clone(), (path_str, refs));
    }
    ASSET_REF_CACHE.with(|c| *c.borrow_mut() = cache);
    log::debug!("[AssetRef] registry loaded from disk");
}

/// Update component fields that reference `old_path` → `new_path` after a rename/move.
/// Called from main.rs on the `update_asset_paths:` signal.
pub fn apply_path_rename(
    old_path: &str,
    new_path: &str,
    world:    &ECSWorld,
    registry: &ComponentRegistry,
) -> usize {
    let mut count = 0usize;
    for entity in world.all_entities() {
        for &comp_name in world.component_names(entity) {
            let Some(fields)  = registry.component_fields(comp_name) else { continue };
            let Some(reflect) = registry.get_reflect(comp_name, world, entity) else { continue };
            for field in fields {
                let Some(val) = reflect.get_field(field.name) else { continue };
                let is_asset_val = matches!(val, ReflectValue::AssetPath(Some(_)));
                if !is_asset_val && !is_asset_field_type(field.field_type) { continue }
                if asset_path_from_value(&val) != Some(old_path) { continue }
                let new_val = match val {
                    ReflectValue::AssetPath(_) => ReflectValue::AssetPath(Some(new_path.to_string())),
                    ReflectValue::OptionStr(_) => ReflectValue::OptionStr(Some(new_path.to_string())),
                    ReflectValue::Str(_)       => ReflectValue::Str(new_path.to_string()),
                    _ => continue,
                };
                if registry.set_reflect_field(comp_name, world, entity, field.name, new_val).is_ok() {
                    count += 1;
                }
            }
        }
    }
    ASSET_REF_CACHE.with(|c| {
        for (path, _) in c.borrow_mut().values_mut() {
            if path == old_path { *path = new_path.to_string(); }
        }
    });
    if count > 0 { log::info!("[AssetRef] renamed {count} field(s): {old_path} → {new_path}"); }
    count
}

/// Clear component fields referencing a deleted asset (looked up by GUID).
/// Called from main.rs on the `clear_asset_refs:` signal.
pub fn clear_asset_refs_by_guid(
    guid:     &str,
    world:    &ECSWorld,
    registry: &ComponentRegistry,
) -> usize {
    let asset_path = ASSET_REF_CACHE.with(|c| c.borrow().get(guid).map(|(p, _)| p.clone()));
    let Some(asset_path) = asset_path else { return 0 };
    let mut count = 0usize;
    for entity in world.all_entities() {
        for &comp_name in world.component_names(entity) {
            let Some(fields)  = registry.component_fields(comp_name) else { continue };
            let Some(reflect) = registry.get_reflect(comp_name, world, entity) else { continue };
            for field in fields {
                let Some(val) = reflect.get_field(field.name) else { continue };
                let is_asset_val = matches!(val, ReflectValue::AssetPath(Some(_)));
                if !is_asset_val && !is_asset_field_type(field.field_type) { continue }
                if asset_path_from_value(&val) != Some(asset_path.as_str()) { continue }
                let new_val = match val {
                    ReflectValue::AssetPath(_) => ReflectValue::AssetPath(None),
                    ReflectValue::OptionStr(_) => ReflectValue::OptionStr(None),
                    ReflectValue::Str(_)       => ReflectValue::Str(String::new()),
                    _ => continue,
                };
                if registry.set_reflect_field(comp_name, world, entity, field.name, new_val).is_ok() {
                    count += 1;
                }
            }
        }
    }
    ASSET_REF_CACHE.with(|c| { c.borrow_mut().remove(guid); });
    if count > 0 { log::info!("[AssetRef] cleared {count} field ref(s) after deleting GUID {guid}"); }
    count
}

// ── Module builder ────────────────────────────────────────────────────────────

pub fn build_world_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["world"])?;

    // ── Entity listing ────────────────────────────────────────────────────────
    // Entity IDs are exposed as i64 (integer bit-cast of EntityId).
    // Integers are Copy in Rune, so they can be passed to multiple functions
    // without ownership issues.

    m.function("entities", || -> Vec<i64> {
        with_ctx(|world, _| {
            world.all_entities().map(entity_to_id).collect()
        }).unwrap_or_default()
    }).build()?;

    m.function("entity_name", |id: i64| -> String {
        with_ctx(|world, _| {
            id_to_entity(world, id)
                .map(|e| world.get_name(e).to_string())
                .unwrap_or_else(|| format!("?{id}"))
        }).unwrap_or_default()
    }).build()?;

    // ── Selection ─────────────────────────────────────────────────────────────

    m.function("selected", || -> i64 {
        SELECTED.with(|s| {
            s.borrow().map(entity_to_id).unwrap_or(-1)
        })
    }).build()?;

    m.function("set_selected", |id: i64| {
        if id < 0 {
            SELECTED.with(|s| *s.borrow_mut() = None);
            return;
        }
        if is_protected_editor_cam(id) { return; }
        with_ctx(|world, _| {
            if let Some(entity) = id_to_entity(world, id) {
                SELECTED.with(|s| *s.borrow_mut() = Some(entity));
            }
        });
    }).build()?;

    // ── Component listing ─────────────────────────────────────────────────────

    m.function("component_types", |id: i64| -> Vec<String> {
        with_ctx(|world, registry| {
            if let Some(entity) = id_to_entity(world, id) {
                world.component_names(entity)
                    .iter()
                    .filter(|&&name| registry.is_visible(name))
                    .map(|&s| s.to_string())
                    .collect()
            } else {
                vec![]
            }
        }).unwrap_or_default()
    }).build()?;

    // ── Field metadata ────────────────────────────────────────────────────────

    m.function("fields", |id: i64, component: Ref<str>| -> Vec<String> {
        let _ = id;
        with_ctx(|_, registry| {
            registry.component_fields(&component)
                .map(|fields| fields.iter().map(|f| f.name.to_string()).collect())
                .unwrap_or_default()
        }).unwrap_or_default()
    }).build()?;

    m.function("field_type", |id: i64, component: Ref<str>, field: Ref<str>| -> String {
        let _ = id;
        with_ctx(|_, registry| {
            get_descriptor(registry, &component, &field)
                .map(|d| fluxion_core::reflect::field_type_str(d.field_type).to_string())
                .unwrap_or_default()
        }).unwrap_or_default()
    }).build()?;

    m.function("field_range", |id: i64, component: Ref<str>, field: Ref<str>| -> Vec<f64> {
        let _ = id;
        with_ctx(|_, registry| {
            get_descriptor(registry, &component, &field)
                .map(|d| {
                    let min = d.range.min.unwrap_or(-1_000_000.0) as f64;
                    let max = d.range.max.unwrap_or( 1_000_000.0) as f64;
                    vec![min, max]
                })
                .unwrap_or_else(|| vec![-1e9, 1e9])
        }).unwrap_or_else(|| vec![-1e9, 1e9])
    }).build()?;

    m.function("field_readonly", |id: i64, component: Ref<str>, field: Ref<str>| -> bool {
        let _ = id;
        with_ctx(|_, registry| {
            get_descriptor(registry, &component, &field)
                .map(|d| d.read_only)
                .unwrap_or(false)
        }).unwrap_or(false)
    }).build()?;

    m.function("get_enum_options", |id: i64, component: Ref<str>, field: Ref<str>| -> Vec<String> {
        let _ = id;
        with_ctx(|_, registry| {
            get_descriptor(registry, &component, &field)
                .and_then(|d| d.enum_variants)
                .map(|variants| variants.iter().map(|s| s.to_string()).collect())
        }).flatten().unwrap_or_default()
    }).build()?;

    m.function("field_display_name", |id: i64, component: Ref<str>, field: Ref<str>| -> String {
        let _ = id;
        with_ctx(|_, registry| {
            get_descriptor(registry, &component, &field)
                .map(|d| d.display_name.to_string())
                .unwrap_or_else(|| field.to_string())
        }).unwrap_or_else(|| field.to_string())
    }).build()?;

    m.function("field_group", |id: i64, component: Ref<str>, field: Ref<str>| -> String {
        let _ = id;
        with_ctx(|_, registry| {
            get_descriptor(registry, &component, &field)
                .and_then(|d| d.group)
                .unwrap_or("")
                .to_string()
        }).unwrap_or_default()
    }).build()?;

    m.function("field_header", |id: i64, component: Ref<str>, field: Ref<str>| -> String {
        let _ = id;
        with_ctx(|_, registry| {
            get_descriptor(registry, &component, &field)
                .and_then(|d| d.header)
                .unwrap_or("")
                .to_string()
        }).unwrap_or_default()
    }).build()?;

    m.function("field_tooltip", |id: i64, component: Ref<str>, field: Ref<str>| -> String {
        let _ = id;
        with_ctx(|_, registry| {
            get_descriptor(registry, &component, &field)
                .and_then(|d| d.tooltip)
                .unwrap_or("")
                .to_string()
        }).unwrap_or_default()
    }).build()?;

    m.function("field_render_hint", |id: i64, component: Ref<str>, field: Ref<str>| -> String {
        let _ = id;
        with_ctx(|_, registry| {
            get_descriptor(registry, &component, &field)
                .map(|d| match d.render_hint {
                    fluxion_core::reflect::RenderHint::Slider       => "slider",
                    fluxion_core::reflect::RenderHint::UniformScale => "uniform_scale",
                    fluxion_core::reflect::RenderHint::Default      => "default",
                })
                .unwrap_or("default")
                .to_string()
        }).unwrap_or_else(|| "default".to_string())
    }).build()?;

    m.function("field_visible", |id: i64, component: Ref<str>, field: Ref<str>| -> bool {
        with_ctx(|world, registry| {
            let descriptor = get_descriptor(registry, &component, &field)?;
            if descriptor.visible_if.is_none() { return Some(true); }
            let entity  = id_to_entity(world, id)?;
            let reflect = registry.get_reflect(&component, world, entity)?;
            Some(descriptor.is_visible(reflect.as_ref()))
        }).flatten().unwrap_or(true)
    }).build()?;

    // ── Typed getters ─────────────────────────────────────────────────────────

    m.function("get_f32", |id: i64, component: Ref<str>, field: Ref<str>| -> f64 {
        with_ctx(|world, registry| {
            let entity = id_to_entity(world, id)?;
            let reflect = registry.get_reflect(&component, world, entity)?;
            match reflect.get_field(&field)? {
                ReflectValue::F32(v) => Some(v as f64),
                _ => None,
            }
        }).flatten().unwrap_or(0.0)
    }).build()?;

    m.function("get_bool", |id: i64, component: Ref<str>, field: Ref<str>| -> bool {
        with_ctx(|world, registry| {
            let entity = id_to_entity(world, id)?;
            let reflect = registry.get_reflect(&component, world, entity)?;
            match reflect.get_field(&field)? {
                ReflectValue::Bool(v) => Some(v),
                _ => None,
            }
        }).flatten().unwrap_or(false)
    }).build()?;

    m.function("get_str", |id: i64, component: Ref<str>, field: Ref<str>| -> String {
        with_ctx(|world, registry| {
            let entity = id_to_entity(world, id)?;
            let reflect = registry.get_reflect(&component, world, entity)?;
            match reflect.get_field(&field)? {
                ReflectValue::Str(v)       => Some(v),
                ReflectValue::OptionStr(v) => Some(v.unwrap_or_default()),
                ReflectValue::Enum(v)      => Some(v),
                _ => None,
            }
        }).flatten().unwrap_or_default()
    }).build()?;

    m.function("get_vec3", |id: i64, component: Ref<str>, field: Ref<str>| -> Vec<f64> {
        with_ctx(|world, registry| {
            let entity = id_to_entity(world, id)?;
            let reflect = registry.get_reflect(&component, world, entity)?;
            match reflect.get_field(&field)? {
                ReflectValue::Vec3([x, y, z]) =>
                    Some(vec![x as f64, y as f64, z as f64]),
                _ => None,
            }
        }).flatten().unwrap_or_else(|| vec![0.0, 0.0, 0.0])
    }).build()?;

    m.function("get_quat", |id: i64, component: Ref<str>, field: Ref<str>| -> Vec<f64> {
        with_ctx(|world, registry| {
            let entity = id_to_entity(world, id)?;
            let reflect = registry.get_reflect(&component, world, entity)?;
            match reflect.get_field(&field)? {
                ReflectValue::Quat([x, y, z, w]) =>
                    Some(vec![x as f64, y as f64, z as f64, w as f64]),
                _ => None,
            }
        }).flatten().unwrap_or_else(|| vec![0.0, 0.0, 0.0, 1.0])
    }).build()?;

    m.function("get_color3", |id: i64, component: Ref<str>, field: Ref<str>| -> Vec<f64> {
        with_ctx(|world, registry| {
            let entity = id_to_entity(world, id)?;
            let reflect = registry.get_reflect(&component, world, entity)?;
            match reflect.get_field(&field)? {
                ReflectValue::Color3([r, g, b]) =>
                    Some(vec![r as f64, g as f64, b as f64]),
                _ => None,
            }
        }).flatten().unwrap_or_else(|| vec![1.0, 1.0, 1.0])
    }).build()?;

    m.function("get_color4", |id: i64, component: Ref<str>, field: Ref<str>| -> Vec<f64> {
        with_ctx(|world, registry| {
            let entity = id_to_entity(world, id)?;
            let reflect = registry.get_reflect(&component, world, entity)?;
            match reflect.get_field(&field)? {
                ReflectValue::Color4([r, g, b, a]) =>
                    Some(vec![r as f64, g as f64, b as f64, a as f64]),
                _ => None,
            }
        }).flatten().unwrap_or_else(|| vec![1.0, 1.0, 1.0, 1.0])
    }).build()?;

    m.function("get_u32", |id: i64, component: Ref<str>, field: Ref<str>| -> i64 {
        with_ctx(|world, registry| {
            let entity = id_to_entity(world, id)?;
            let reflect = registry.get_reflect(&component, world, entity)?;
            match reflect.get_field(&field)? {
                ReflectValue::U32(v)   => Some(v as i64),
                ReflectValue::U8(v)    => Some(v as i64),
                ReflectValue::USize(v) => Some(v as i64),
                _ => None,
            }
        }).flatten().unwrap_or(0)
    }).build()?;

    m.function("get_i32", |id: i64, component: Ref<str>, field: Ref<str>| -> i64 {
        with_ctx(|world, registry| {
            let entity = id_to_entity(world, id)?;
            let reflect = registry.get_reflect(&component, world, entity)?;
            match reflect.get_field(&field)? {
                ReflectValue::I32(v) => Some(v as i64),
                _ => None,
            }
        }).flatten().unwrap_or(0)
    }).build()?;

    m.function("get_vec2", |id: i64, component: Ref<str>, field: Ref<str>| -> Vec<f64> {
        with_ctx(|world, registry| {
            let entity = id_to_entity(world, id)?;
            let reflect = registry.get_reflect(&component, world, entity)?;
            match reflect.get_field(&field)? {
                ReflectValue::Vec2([x, y]) => Some(vec![x as f64, y as f64]),
                _ => None,
            }
        }).flatten().unwrap_or_else(|| vec![0.0, 0.0])
    }).build()?;

    // ── Typed setters (queued — applied after panel call) ─────────────────────

    m.function("set_f32", |id: i64, component: String, field: String, val: f64| {
        if let Some(e) = with_ctx(|world, _| id_to_entity(world, id)).flatten() {
            queue_edit(e, component, field, ReflectValue::F32(val as f32));
        }
    }).build()?;

    m.function("set_bool", |id: i64, component: String, field: String, val: bool| {
        if let Some(e) = with_ctx(|world, _| id_to_entity(world, id)).flatten() {
            queue_edit(e, component, field, ReflectValue::Bool(val));
        }
    }).build()?;

    m.function("set_str", |id: i64, component: String, field: String, val: String| {
        let result = with_ctx(|world, reg| {
            let entity = id_to_entity(world, id)?;
            let ft = get_descriptor(reg, &component, &field)?.field_type;
            let rv = match ft {
                ReflectFieldType::OptionStr
                | ReflectFieldType::Material
                | ReflectFieldType::Mesh
                | ReflectFieldType::Scene   => ReflectValue::OptionStr(
                    if val.is_empty() { None } else { Some(val) }
                ),
                ReflectFieldType::Audio     => ReflectValue::Str(val),
                ReflectFieldType::Enum      => ReflectValue::Enum(val),
                _                           => ReflectValue::Str(val),
            };
            queue_edit(entity, component, field, rv);
            Some(())
        });
        let _ = result;
    }).build()?;

    m.function("set_vec3", |id: i64, component: String, field: String, vals: Vec<f64>| {
        if vals.len() >= 3 {
            if let Some(e) = with_ctx(|world, _| id_to_entity(world, id)).flatten() {
                queue_edit(e, component, field,
                    ReflectValue::Vec3([vals[0] as f32, vals[1] as f32, vals[2] as f32]));
            }
        }
    }).build()?;

    m.function("set_color3", |id: i64, component: String, field: String, vals: Vec<f64>| {
        if vals.len() >= 3 {
            if let Some(e) = with_ctx(|world, _| id_to_entity(world, id)).flatten() {
                queue_edit(e, component, field,
                    ReflectValue::Color3([vals[0] as f32, vals[1] as f32, vals[2] as f32]));
            }
        }
    }).build()?;

    m.function("set_color4", |id: i64, component: String, field: String, vals: Vec<f64>| {
        if vals.len() >= 4 {
            if let Some(e) = with_ctx(|world, _| id_to_entity(world, id)).flatten() {
                queue_edit(e, component, field,
                    ReflectValue::Color4([vals[0] as f32, vals[1] as f32, vals[2] as f32, vals[3] as f32]));
            }
        }
    }).build()?;

    m.function("set_i32", |id: i64, component: String, field: String, val: i64| {
        if let Some(e) = with_ctx(|world, _| id_to_entity(world, id)).flatten() {
            queue_edit(e, component, field, ReflectValue::I32(val as i32));
        }
    }).build()?;

    m.function("get_entity_ref", |id: i64, component: Ref<str>, field: Ref<str>| -> i64 {
        with_ctx(|world, registry| {
            let entity = id_to_entity(world, id)?;
            let reflect = registry.get_reflect(&component, world, entity)?;
            match reflect.get_field(&field) {
                Some(ReflectValue::I32(v)) => Some(v as i64),
                _ => Some(-1),
            }
        }).flatten().unwrap_or(-1)
    }).build()?;

    m.function("set_entity_ref", |id: i64, component: String, field: String, ref_id: i64| {
        if let Some(e) = with_ctx(|world, _| id_to_entity(world, id)).flatten() {
            queue_edit(e, component, field, ReflectValue::I32(ref_id as i32));
        }
    }).build()?;

    m.function("set_vec2", |id: i64, component: String, field: String, vals: Vec<f64>| {
        if vals.len() >= 2 {
            if let Some(e) = with_ctx(|world, _| id_to_entity(world, id)).flatten() {
                queue_edit(e, component, field,
                    ReflectValue::Vec2([vals[0] as f32, vals[1] as f32]));
            }
        }
    }).build()?;

    m.function("set_u32", |id: i64, component: String, field: String, val: i64| {
        let result = with_ctx(|world, reg| {
            let entity = id_to_entity(world, id)?;
            let ft = get_descriptor(reg, &component, &field)?.field_type;
            let rv = match ft {
                ReflectFieldType::U8    => ReflectValue::U8(val.clamp(0, 255) as u8),
                ReflectFieldType::USize => ReflectValue::USize(val.max(0) as usize),
                _                       => ReflectValue::U32(val.max(0) as u32),
            };
            queue_edit(entity, component, field, rv);
            Some(())
        });
        let _ = result;
    }).build()?;

    // ── Entity creation / deletion ────────────────────────────────────────────

    m.function("create_entity", |name: Ref<str>| {
        PENDING.with(|p| p.borrow_mut().push(PendingEdit {
            entity:    fluxion_core::EntityId::INVALID,
            component: "__spawn__".to_string(),
            field:     name.as_ref().to_string(),
            value:     fluxion_core::reflect::ReflectValue::Bool(true),
        }));
    }).build()?;

    m.function("despawn", |id: i64| {
        with_ctx(|world, _| {
            if let Some(entity) = id_to_entity(world, id) {
                PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                    entity,
                    component: "__despawn__".to_string(),
                    field:     String::new(),
                    value:     fluxion_core::reflect::ReflectValue::Bool(true),
                }));
            }
        });
    }).build()?;

    // instantiate_prefab(path: str) -> i64
    // Loads a .scene file and spawns its entities as children of a new root entity.
    // Returns the root entity id (i64) on success, or -1 on failure.
    // The path is relative to the project root (e.g. "assets/prefabs/crate.scene").
    m.function("instantiate_prefab", |path: Ref<str>| -> i64 {
        let full_path = PROJECT_ROOT.with(|root| {
            root.borrow().join(path.as_ref())
        });
        PENDING.with(|p| p.borrow_mut().push(PendingEdit {
            entity:    fluxion_core::EntityId::INVALID,
            component: "__instantiate_prefab__".to_string(),
            field:     full_path.to_string_lossy().to_string(),
            value:     fluxion_core::reflect::ReflectValue::Bool(true),
        }));
        -1 // actual id is not available synchronously; host assigns it next flush
    }).build()?;

    // ── Asset browser (legacy + AssetDatabase-backed) ────────────────────────

    // list_assets / list_asset_dirs: kept for backward compat; now delegate to
    // AssetDatabase when available, fall back to direct FS scan otherwise.

    m.function("list_assets", |subdir: Ref<str>| -> Vec<String> {
        if let Some(names) = with_adb(|db| {
            db.list_dir(subdir.as_ref())
              .into_iter()
              .map(|r| {
                  // Return filename only (legacy callers expect just the name).
                  r.path.rsplit('/').next().unwrap_or(&r.path).to_string()
              })
              .collect::<Vec<_>>()
        }) {
            return names;
        }
        // Fallback: direct filesystem scan.
        PROJECT_ROOT.with(|root| {
            let base = root.borrow();
            let dir  = if subdir.is_empty() {
                base.join("assets")
            } else {
                base.join("assets").join(subdir.as_ref())
            };
            let mut paths = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_file() {
                        let name = p.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        if !name.ends_with(".fluxmeta") {
                            paths.push(name);
                        }
                    }
                }
            }
            paths.sort();
            paths
        })
    }).build()?;

    m.function("list_asset_dirs", || -> Vec<String> {
        if let Some(dirs) = with_adb(|db| db.list_dirs()) {
            return dirs;
        }
        PROJECT_ROOT.with(|root| {
            let dir  = root.borrow().join("assets");
            let mut dirs = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        if let Some(name) = p.file_name() {
                            dirs.push(name.to_string_lossy().to_string());
                        }
                    }
                }
            }
            dirs.sort();
            dirs
        })
    }).build()?;

    // ── AssetDatabase API (Unity-like: AssetDatabase.load / .find / .guid …) ──

    m.function("asset_count", || -> i64 {
        with_adb(|db| db.count() as i64).unwrap_or(0)
    }).build()?;

    // asset_list(subdir) — full relative paths; empty subdir = root-level files.
    m.function("asset_list", |subdir: Ref<str>| -> Vec<String> {
        with_adb(|db| {
            db.list_dir(subdir.as_ref())
              .into_iter()
              .map(|r| r.path.clone())
              .collect()
        }).unwrap_or_default()
    }).build()?;

    // asset_dirs() — immediate subdirectory names (sorted).
    m.function("asset_dirs", || -> Vec<String> {
        with_adb(|db| db.list_dirs()).unwrap_or_default()
    }).build()?;

    // asset_subdirs(subdir) — immediate child directories of a given folder (sorted).
    // Pass "" to get top-level dirs.  Returns relative paths like "textures/ui".
    m.function("asset_subdirs", |subdir: Ref<str>| -> Vec<String> {
        with_adb(|db| {
            let prefix = if subdir.as_ref().is_empty() {
                String::new()
            } else {
                format!("{}/", subdir.as_ref())
            };
            let mut result: Vec<String> = db.list_dirs().into_iter()
                .filter(|d| {
                    if prefix.is_empty() {
                        !d.contains('/')
                    } else {
                        d.starts_with(&prefix) && !d[prefix.len()..].contains('/')
                    }
                })
                .collect();
            result.sort();
            result
        }).unwrap_or_default()
    }).build()?;

    // asset_list_typed(subdir, type_filter) — like asset_list but filtered by type string.
    // Pass "" as type_filter for all types.
    m.function("asset_list_typed", |subdir: Ref<str>, type_filter: Ref<str>| -> Vec<String> {
        with_adb(|db| {
            db.list_dir_typed(subdir.as_ref(), type_filter.as_ref())
              .into_iter()
              .map(|r| r.path.clone())
              .collect()
        }).unwrap_or_default()
    }).build()?;

    // asset_get_active_dir / asset_set_active_dir — tracks which folder is open in right panel.
    m.function("asset_get_active_dir", || -> String {
        ASSET_ACTIVE_DIR.with(|d| d.borrow().clone())
    }).build()?;
    m.function("asset_set_active_dir", |dir: String| {
        ASSET_ACTIVE_DIR.with(|d| *d.borrow_mut() = dir.clone());
        // Push to nav history (truncate forward stack if navigating mid-history)
        ASSET_NAV_HISTORY.with(|h| {
            ASSET_NAV_POS.with(|p| {
                let mut hist = h.borrow_mut();
                let pos = p.get();
                // Truncate anything after current position
                let new_len = if pos >= 0 { (pos as usize + 1).min(hist.len()) } else { 0 };
                hist.truncate(new_len);
                hist.push(dir);
                p.set((hist.len() as i64) - 1);
            });
        });
    }).build()?;

    // asset_can_nav_back() / asset_can_nav_fwd() — whether back/forward are available.
    m.function("asset_can_nav_back", || -> bool {
        ASSET_NAV_POS.with(|p| p.get() > 0)
    }).build()?;
    m.function("asset_can_nav_fwd", || -> bool {
        ASSET_NAV_HISTORY.with(|h| {
            ASSET_NAV_POS.with(|p| {
                let pos = p.get();
                pos >= 0 && (pos as usize) < h.borrow().len().saturating_sub(1)
            })
        })
    }).build()?;

    // asset_nav_back() → String  — navigate back, return new active dir.
    m.function("asset_nav_back", || -> String {
        ASSET_NAV_HISTORY.with(|h| {
            ASSET_NAV_POS.with(|p| {
                let pos = p.get();
                if pos > 0 {
                    let new_pos = pos - 1;
                    p.set(new_pos);
                    let dir = h.borrow().get(new_pos as usize).cloned().unwrap_or_default();
                    ASSET_ACTIVE_DIR.with(|d| *d.borrow_mut() = dir.clone());
                    dir
                } else {
                    ASSET_ACTIVE_DIR.with(|d| d.borrow().clone())
                }
            })
        })
    }).build()?;

    // asset_nav_fwd() → String  — navigate forward, return new active dir.
    m.function("asset_nav_fwd", || -> String {
        ASSET_NAV_HISTORY.with(|h| {
            ASSET_NAV_POS.with(|p| {
                let pos = p.get();
                let max = h.borrow().len().saturating_sub(1) as i64;
                if pos < max {
                    let new_pos = pos + 1;
                    p.set(new_pos);
                    let dir = h.borrow().get(new_pos as usize).cloned().unwrap_or_default();
                    ASSET_ACTIVE_DIR.with(|d| *d.borrow_mut() = dir.clone());
                    dir
                } else {
                    ASSET_ACTIVE_DIR.with(|d| d.borrow().clone())
                }
            })
        })
    }).build()?;

    // asset_nav_up() → String  — navigate to parent dir, return new active dir.
    m.function("asset_nav_up", || -> String {
        let current = ASSET_ACTIVE_DIR.with(|d| d.borrow().clone());
        let parent = current.rfind('/').map(|i| current[..i].to_string()).unwrap_or_default();
        ASSET_ACTIVE_DIR.with(|d| *d.borrow_mut() = parent.clone());
        // Also push to history
        ASSET_NAV_HISTORY.with(|h| {
            ASSET_NAV_POS.with(|p| {
                let mut hist = h.borrow_mut();
                let pos = p.get();
                let new_len = if pos >= 0 { (pos as usize + 1).min(hist.len()) } else { 0 };
                hist.truncate(new_len);
                hist.push(parent.clone());
                p.set((hist.len() as i64) - 1);
            });
        });
        parent
    }).build()?;

    // asset_show_in_explorer(path) — opens the OS file manager at the given asset path.
    m.function("asset_show_in_explorer", |path: Ref<str>| {
        let abs = PROJECT_ROOT.with(|r| r.borrow().join(path.as_ref()));
        let target = if abs.is_file() {
            abs.parent().map(|p| p.to_path_buf()).unwrap_or(abs)
        } else { abs };
        #[cfg(target_os = "windows")]
        { let _ = std::process::Command::new("explorer").arg(target).spawn(); }
        #[cfg(target_os = "macos")]
        { let _ = std::process::Command::new("open").arg(target).spawn(); }
        #[cfg(target_os = "linux")]
        { let _ = std::process::Command::new("xdg-open").arg(target).spawn(); }
    }).build()?;

    // open_in_external_editor(path) — opens a script/text file in the configured external editor.
    // Reads the "script_editor" pref: "vscode" | "vscodium" | "default".
    m.function("open_in_external_editor", |path: String| {
        let abs = PROJECT_ROOT.with(|r| {
            let root = r.borrow().clone();
            if root == std::path::PathBuf::new() { return std::path::PathBuf::from(&path); }
            root.join("assets").join(&path)
        });
        let editor = crate::rune_bindings::settings_module::get_script_editor();
        match editor.as_str() {
            "vscode" => {
                if std::process::Command::new("code").arg(&abs).spawn().is_err() {
                    log::warn!("[Editor] 'code' not found in PATH, falling back to default");
                    open_with_default(&abs);
                }
            }
            "vscodium" => {
                if std::process::Command::new("codium").arg(&abs).spawn().is_err() {
                    log::warn!("[Editor] 'codium' not found in PATH, falling back to default");
                    open_with_default(&abs);
                }
            }
            _ => open_with_default(&abs),
        }
    }).build()?;

    // asset_get_type_filter / asset_set_type_filter — "" = All.
    m.function("asset_get_type_filter", || -> String {
        ASSET_TYPE_FILTER.with(|f| f.borrow().clone())
    }).build()?;
    m.function("asset_set_type_filter", |filter: String| {
        ASSET_TYPE_FILTER.with(|f| *f.borrow_mut() = filter);
    }).build()?;

    // asset_get_zoom / asset_set_zoom — tile size multiplier (0.5 – 2.0).
    m.function("asset_get_zoom", || -> f64 {
        ASSET_ZOOM.with(|z| z.get())
    }).build()?;
    m.function("asset_set_zoom", |z: f64| {
        ASSET_ZOOM.with(|zoom| zoom.set(z.clamp(0.5, 2.0)));
    }).build()?;

    // asset_get_view_mode / asset_set_view_mode — "tile" | "list".
    m.function("asset_get_view_mode", || -> String {
        ASSET_VIEW_MODE.with(|v| v.borrow().clone())
    }).build()?;
    m.function("asset_set_view_mode", |mode: String| {
        let clamped = if mode == "list" { "list" } else { "tile" };
        ASSET_VIEW_MODE.with(|v| *v.borrow_mut() = clamped.to_string());
    }).build()?;

    // asset_guid(path) — stable GUID from .fluxmeta sidecar.
    m.function("asset_guid", |path: Ref<str>| -> String {
        with_adb(|db| {
            db.get_by_path(path.as_ref()).map(|r| r.guid.clone())
        }).flatten().unwrap_or_default()
    }).build()?;

    // asset_type(path) — "texture" | "model" | "audio" | "script" | … | "unknown"
    m.function("asset_type", |path: Ref<str>| -> String {
        with_adb(|db| {
            db.get_by_path(path.as_ref()).map(|r| r.type_str().to_string())
        }).flatten().unwrap_or_else(|| "unknown".to_string())
    }).build()?;

    // asset_size(path) — file size in bytes.
    m.function("asset_size", |path: Ref<str>| -> i64 {
        with_adb(|db| {
            db.get_by_path(path.as_ref()).map(|r| r.file_size as i64)
        }).flatten().unwrap_or(0)
    }).build()?;

    // asset_size_display(path) — human-readable size ("1.2 MB").
    m.function("asset_size_display", |path: Ref<str>| -> String {
        with_adb(|db| {
            db.get_by_path(path.as_ref()).map(|r| r.size_display())
        }).flatten().unwrap_or_default()
    }).build()?;

    // asset_tags(path) — user tags from .fluxmeta.
    m.function("asset_tags", |path: Ref<str>| -> Vec<String> {
        with_adb(|db| {
            db.get_by_path(path.as_ref()).map(|r| r.tags.clone())
        }).flatten().unwrap_or_default()
    }).build()?;

    // asset_get_import(path) → [[key, value, input_type], …]
    // Returns import settings for an asset, merged with per-type defaults.
    // input_type is "bool", "f32", "i32", or "enum:a|b|c".
    m.function("asset_get_import", |path: Ref<str>| -> Vec<Vec<String>> {
        let t = with_adb(|db| {
            db.get_by_path(path.as_ref()).map(|r| r.type_str().to_string())
        }).flatten().unwrap_or_default();

        let defaults: &[(&str, &str, &str)] = match t.as_str() {
            "texture" => &[
                ("srgb",         "true",    "bool"),
                ("gen_mipmaps",  "true",    "bool"),
                ("max_size",     "2048",    "i32"),
                ("filter",       "Linear",  "enum:Linear|Nearest|Trilinear"),
                ("compression",  "BC3",     "enum:None|BC1|BC3|BC7"),
            ],
            "model" => &[
                ("scale",              "1.0",  "f32"),
                ("import_normals",     "true", "bool"),
                ("import_tangents",    "true", "bool"),
                ("import_animations",  "true", "bool"),
                ("merge_meshes",       "false","bool"),
            ],
            "audio" => &[
                ("mono",       "false", "bool"),
                ("normalize",  "false", "bool"),
                ("streaming",  "false", "bool"),
            ],
            _ => &[],
        };

        defaults.iter().map(|(key, default_val, input_type)| {
            let stored = with_adb(|db| db.get_import_setting(path.as_ref(), key))
                .flatten();
            let value = stored.unwrap_or_else(|| default_val.to_string());
            vec![key.to_string(), value, input_type.to_string()]
        }).collect()
    }).build()?;

    // asset_set_import(path, key, value) — persist one import setting to .fluxmeta.
    m.function("asset_set_import", |path: String, key: String, value: String| {
        let root = PROJECT_ROOT.with(|r| r.borrow().clone());
        with_adb_mut(|db| db.set_import_setting(&path, &key, &value, &root));
    }).build()?;

    // asset_search(query) — name search or "type:texture" / "name:sky" syntax.
    m.function("asset_search", |query: Ref<str>| -> Vec<String> {
        with_adb(|db| {
            db.find(query.as_ref())
              .into_iter()
              .map(|r| r.path.clone())
              .collect()
        }).unwrap_or_default()
    }).build()?;

    // asset_rescan() — signal main.rs to re-run AssetDatabase::scan.
    m.function("asset_rescan", || {
        ACTION_SIGNALS.with(|s| s.borrow_mut().push("rescan_assets".to_string()));
    }).build()?;

    // get/set_asset_search — persistent search query across frames.
    m.function("get_asset_search", || -> String {
        ASSET_SEARCH_QUERY.with(|q| q.borrow().clone())
    }).build()?;

    m.function("set_asset_search", |query: String| {
        ASSET_SEARCH_QUERY.with(|q| *q.borrow_mut() = query);
    }).build()?;

    // asset_filename(path) — "models/cube.glb" → "cube.glb" (last path segment).
    m.function("asset_filename", |path: Ref<str>| -> String {
        path.as_ref().rsplit('/').next().unwrap_or(path.as_ref()).to_string()
    }).build()?;

    // asset_basename(path) — "models/cube.glb" → "cube" (no extension).
    m.function("asset_basename", |path: Ref<str>| -> String {
        let filename = path.as_ref().rsplit('/').next().unwrap_or(path.as_ref());
        match filename.rfind('.') {
            Some(dot) => filename[..dot].to_string(),
            None      => filename.to_string(),
        }
    }).build()?;

    // ── Asset selection ───────────────────────────────────────────────────────

    // get_selected_asset() → project-relative path, or "" when nothing selected.
    m.function("get_selected_asset", || -> String {
        SELECTED_ASSET_PATH.with(|s| s.borrow().clone())
    }).build()?;

    // set_selected_asset(path) — select an asset and deselect any entity.
    m.function("set_selected_asset", |path: String| {
        SELECTED_ASSET_PATH.with(|s| *s.borrow_mut() = path);
        // Deselect entity so the inspector switches to asset view.
        SELECTED.with(|s| *s.borrow_mut() = None);
    }).build()?;

    // clear_selected_asset() — deselect the current asset.
    m.function("clear_selected_asset", || {
        SELECTED_ASSET_PATH.with(|s| s.borrow_mut().clear());
    }).build()?;

    // load_scene(path) — request main.rs to load a scene by project-relative path.
    // Path should include the "assets/" prefix if stored there, e.g. "assets/myscene.fluxscene".
    m.function("load_scene", |path: String| {
        ACTION_SIGNALS.with(|s| s.borrow_mut().push(format!("load_scene:{path}")));
    }).build()?;

    // ── Asset panel creation / deletion ─────────────────────────────────────────

    // get/set_asset_create_mode — track inline creation form state ("" | "dir" | "file").
    m.function("get_asset_create_mode", || -> String {
        ASSET_CREATE_MODE.with(|m| m.borrow().clone())
    }).build()?;
    m.function("set_asset_create_mode", |mode: String| {
        ASSET_CREATE_MODE.with(|m| *m.borrow_mut() = mode);
    }).build()?;

    // get/set_asset_create_input — the name being typed in the creation form.
    m.function("get_asset_create_input", || -> String {
        ASSET_CREATE_INPUT.with(|i| i.borrow().clone())
    }).build()?;
    m.function("set_asset_create_input", |text: String| {
        ASSET_CREATE_INPUT.with(|i| *i.borrow_mut() = text);
    }).build()?;

    // asset_create_dir(name) — create a directory under {project_root}/assets/{name}.
    // Returns true on success.  Signals a rescan.
    m.function("asset_create_dir", |name: String| -> bool {
        let root = PROJECT_ROOT.with(|r| r.borrow().clone());
        if root == std::path::PathBuf::new() { return false; }
        let dir = root.join("assets").join(&name);
        let ok = std::fs::create_dir_all(&dir).is_ok();
        if ok {
            ACTION_SIGNALS.with(|s| s.borrow_mut().push("rescan_assets".to_string()));
            log::info!("[Assets] Created directory: {}", dir.display());
        } else {
            log::error!("[Assets] Failed to create directory: {}", dir.display());
        }
        ok
    }).build()?;

    // asset_create_file(path, content) — create a file at {project_root}/assets/{path}.
    // Parent directories are created automatically.  Signals a rescan.
    m.function("asset_create_file", |path: String, content: String| -> bool {
        let root = PROJECT_ROOT.with(|r| r.borrow().clone());
        if root == std::path::PathBuf::new() { return false; }
        let full = root.join("assets").join(&path);
        if let Some(parent) = full.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let ok = std::fs::write(&full, content.as_bytes()).is_ok();
        if ok {
            ACTION_SIGNALS.with(|s| s.borrow_mut().push("rescan_assets".to_string()));
            log::info!("[Assets] Created file: {}", full.display());
        } else {
            log::error!("[Assets] Failed to create file: {}", full.display());
        }
        ok
    }).build()?;

    // asset_create_scene(path) — create a blank scene file at {project_root}/assets/{path}.
    // The file receives a minimal valid scene JSON (version 2, no entities).
    // Signals a rescan.  Returns true on success.
    m.function("asset_create_scene", |path: String| -> bool {
        let root = PROJECT_ROOT.with(|r| r.borrow().clone());
        if root == std::path::PathBuf::new() { return false; }
        let full = root.join("assets").join(&path);
        if let Some(parent) = full.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let scene_name = full.file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("new_scene")
            .to_string();
        let content = format!(
            "{{\n  \"name\": \"{}\",\n  \"version\": 2,\n  \"settings\": {{\n    \"ambientColor\": [0.2, 0.2, 0.3],\n    \"ambientIntensity\": 0.3,\n    \"fogEnabled\": false,\n    \"fogColor\": [0.5, 0.6, 0.7],\n    \"fogDensity\": 0.01,\n    \"skybox\": null,\n    \"physicsGravity\": [0.0, -9.81, 0.0]\n  }},\n  \"entities\": []\n}}",
            scene_name
        );
        let ok = std::fs::write(&full, content.as_bytes()).is_ok();
        if ok {
            ACTION_SIGNALS.with(|s| s.borrow_mut().push("rescan_assets".to_string()));
            log::info!("[Assets] Created scene: {}", full.display());
        } else {
            log::error!("[Assets] Failed to create scene: {}", full.display());
        }
        ok
    }).build()?;

    // asset_rename(old_path, new_path) — rename/move a file within assets/.
    // Also renames the .fluxmeta sidecar if present.  Signals a rescan.
    m.function("asset_rename", |old_path: String, new_path: String| -> bool {
        let root = PROJECT_ROOT.with(|r| r.borrow().clone());
        if root == std::path::PathBuf::new() { return false; }
        let assets_root = root.join("assets");
        let old_full = assets_root.join(&old_path);
        let new_full = assets_root.join(&new_path);
        if let Some(parent) = new_full.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let ok = std::fs::rename(&old_full, &new_full).is_ok();
        if ok {
            let old_meta = std::path::PathBuf::from(format!("{}.fluxmeta", old_full.display()));
            let new_meta = std::path::PathBuf::from(format!("{}.fluxmeta", new_full.display()));
            if old_meta.is_file() {
                let _ = std::fs::rename(&old_meta, &new_meta);
            }
            ACTION_SIGNALS.with(|s| {
                let mut v = s.borrow_mut();
                v.push(format!("update_asset_paths:{old_path}\t{new_path}"));
                v.push("rescan_assets".to_string());
            });
            log::info!("[Assets] Renamed: {} → {}", old_full.display(), new_full.display());
        } else {
            log::error!("[Assets] Failed to rename: {}", old_full.display());
        }
        ok
    }).build()?;

    // asset_duplicate(path) — copy a file to {stem}_copy.{ext} in the same directory.
    // Avoids overwriting: appends _2, _3, … if needed.  Signals a rescan.
    m.function("asset_duplicate", |path: String| -> bool {
        let root = PROJECT_ROOT.with(|r| r.borrow().clone());
        if root == std::path::PathBuf::new() { return false; }
        let src = root.join("assets").join(&path);
        if !src.is_file() { return false; }
        let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("file").to_string();
        let ext  = src.extension().and_then(|e| e.to_str()).unwrap_or("").to_string();
        let parent = src.parent().unwrap_or(src.as_path());
        let make_name = |suffix: &str| -> std::path::PathBuf {
            if ext.is_empty() {
                parent.join(format!("{stem}{suffix}"))
            } else {
                parent.join(format!("{stem}{suffix}.{ext}"))
            }
        };
        let mut dest = make_name("_copy");
        let mut n = 2u32;
        while dest.exists() {
            dest = make_name(&format!("_copy_{n}"));
            n += 1;
        }
        let ok = std::fs::copy(&src, &dest).is_ok();
        if ok {
            ACTION_SIGNALS.with(|s| s.borrow_mut().push("rescan_assets".to_string()));
            log::info!("[Assets] Duplicated: {} → {}", src.display(), dest.display());
        } else {
            log::error!("[Assets] Failed to duplicate: {}", src.display());
        }
        ok
    }).build()?;

    // asset_delete(path) — delete a file at {project_root}/assets/{path}.  Signals a rescan.
    m.function("asset_delete", |path: String| -> bool {
        let root = PROJECT_ROOT.with(|r| r.borrow().clone());
        if root == std::path::PathBuf::new() { return false; }
        let full = root.join("assets").join(&path);
        // Get GUID before deletion so we can clear refs afterwards.
        let guid = with_adb(|db| db.get_by_path(&path).map(|r| r.guid.clone())).flatten();
        let ok = if full.is_dir() {
            std::fs::remove_dir_all(&full).is_ok()
        } else {
            // Also remove the .fluxmeta sidecar if present.
            let meta = full.with_extension(
                format!("{}.fluxmeta",
                    full.extension().and_then(|e| e.to_str()).unwrap_or(""))
            );
            let _ = std::fs::remove_file(&meta);
            std::fs::remove_file(&full).is_ok()
        };
        if ok {
            ACTION_SIGNALS.with(|s| {
                let mut v = s.borrow_mut();
                if let Some(g) = guid {
                    v.push(format!("clear_asset_refs:{g}"));
                }
                v.push("rescan_assets".to_string());
            });
            log::info!("[Assets] Deleted: {}", full.display());
        } else {
            log::error!("[Assets] Failed to delete: {}", full.display());
        }
        ok
    }).build()?;

    // asset_dependencies(path) → Vec<String> — assets that `path` directly references.
    m.function("asset_dependencies", |path: Ref<str>| -> Vec<String> {
        with_adb(|db| db.dependencies_of(path.as_ref())).unwrap_or_default()
    }).build()?;

    // asset_references(path) → Vec<String> — assets that reference (use) `path`.
    m.function("asset_references", |path: Ref<str>| -> Vec<String> {
        with_adb(|db| db.references_to(path.as_ref())).unwrap_or_default()
    }).build()?;

    // asset_import_file(src_abs_path, dest_subdir) → String
    // Copy an OS file into {project}/assets/{dest_subdir}/.
    // Returns the resulting project-relative path on success, "" on failure.
    // Signals a rescan.
    m.function("asset_import_file", |src_path: Ref<str>, dest_subdir: Ref<str>| -> String {
        let src = std::path::PathBuf::from(src_path.as_ref());
        let result = with_adb_mut(|db| db.import_file(&src, dest_subdir.as_ref()));
        match result {
            Some(Ok(rel_path)) => {
                ACTION_SIGNALS.with(|s| s.borrow_mut().push("rescan_assets".to_string()));
                rel_path
            }
            Some(Err(e)) => {
                log::error!("[Assets] import_file failed: {e}");
                String::new()
            }
            None => String::new(),
        }
    }).build()?;

    // ── Component add / remove ────────────────────────────────────────────────

    m.function("available_components", || -> Vec<String> {
        with_ctx(|_, registry| {
            registry.reflected_type_names()
                .into_iter()
                .map(|s| s.to_string())
                .collect()
        }).unwrap_or_default()
    }).build()?;

    // available_scripts() → Vec<Vec<String>>: [[display_name, path], ...] for all .rn assets in the DB.
    // display_name is PascalCase derived from the filename stem.
    m.function("available_scripts", || -> Vec<Vec<String>> {
        with_adb(|db| {
            db.find("type:script")
                .into_iter()
                .filter(|r| r.extension == "rn")
                .map(|r| {
                    let name = fluxion_core::derive_script_name(&r.path);
                    vec![name, r.path.clone()]
                })
                .collect::<Vec<_>>()
        }).unwrap_or_default()
    }).build()?;

    // component_icon(name) → String: Lucide icon name for a component type ("" if none registered)
    m.function("component_icon", |name: String| -> String {
        with_ctx(|_, registry| {
            registry.component_icon(&name).to_string()
        }).unwrap_or_default()
    }).build()?;

    m.function("add_component", |id: i64, type_name: Ref<str>| {
        with_ctx(|world, _| {
            if let Some(entity) = id_to_entity(world, id) {
                PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                    entity,
                    component: "__add_comp__".to_string(),
                    field:     type_name.as_ref().to_string(),
                    value:     fluxion_core::reflect::ReflectValue::Bool(true),
                }));
            }
        });
    }).build()?;

    m.function("remove_component", |id: i64, type_name: Ref<str>| {
        with_ctx(|world, _| {
            if let Some(entity) = id_to_entity(world, id) {
                PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                    entity,
                    component: "__remove_comp__".to_string(),
                    field:     type_name.as_ref().to_string(),
                    value:     fluxion_core::reflect::ReflectValue::Bool(true),
                }));
            }
        });
    }).build()?;

    // ── Hierarchy / parenting ─────────────────────────────────────────────────

    m.function("root_entities", || -> Vec<i64> {
        with_ctx(|world, _| {
            world.root_entities()
                .map(entity_to_id)
                .collect::<Vec<_>>()
        }).unwrap_or_default()
    }).build()?;

    m.function("entity_parent", |id: i64| -> i64 {
        with_ctx(|world, _| {
            id_to_entity(world, id)
                .and_then(|e| world.get_parent(e))
                .map(entity_to_id)
                .unwrap_or(-1)
        }).unwrap_or(-1)
    }).build()?;

    m.function("entity_children", |id: i64| -> Vec<i64> {
        with_ctx(|world, _| {
            if let Some(entity) = id_to_entity(world, id) {
                world.get_children(entity)
                    .map(entity_to_id)
                    .collect()
            } else {
                vec![]
            }
        }).unwrap_or_default()
    }).build()?;

    m.function("set_parent", |child_id: i64, parent_id: i64| {
        with_ctx(|world, _| {
            let child = id_to_entity(world, child_id)?;
            let parent = if parent_id < 0 { None } else { id_to_entity(world, parent_id) };
            PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                entity:    child,
                component: "__set_parent__".to_string(),
                field:     parent_id.to_string(),
                value:     fluxion_core::reflect::ReflectValue::Bool(parent.is_some()),
            }));
            Some(())
        });
    }).build()?;

    // ── Console log access ────────────────────────────────────────────────────

    m.function("log", |line: Ref<str>| {
        push_log(format!("[INFO] {}", line.as_ref()));
    }).build()?;

    m.function("clear_log", || {
        LOG_ENTRIES.with(|l| l.borrow_mut().clear());
        LOG_SELECTED.with(|c| c.set(-1));
        LOG_GENERATION.with(|g| g.set(g.get().wrapping_add(1)));
    }).build()?;

    m.function("log_generation", || -> i64 {
        LOG_GENERATION.with(|g| g.get() as i64)
    }).build()?;

    m.function("log_with_level", |level: String, msg: String| {
        let prefix = match level.to_lowercase().as_str() {
            "warn" | "warning" => "[WARN] ",
            "error"            => "[ERROR] ",
            "info"             => "[INFO] ",
            _                  => "[LOG] ",
        };
        push_log(format!("{prefix}{msg}"));
    }).build()?;

    // Backward-compat: rebuild string list from entries.
    m.function("log_lines", || -> Vec<String> {
        LOG_ENTRIES.with(|l| {
            l.borrow().iter().map(|e| {
                format!("[{}] {}", e.level.as_str().to_uppercase(), e.message)
            }).collect()
        })
    }).build()?;

    m.function("log_lines_tail", |n: i64| -> Vec<String> {
        LOG_ENTRIES.with(|l| {
            let entries = l.borrow();
            let n = (n.max(0) as usize).min(entries.len());
            let start = entries.len() - n;
            entries[start..].iter().map(|e| {
                format!("[{}] {}", e.level.as_str().to_uppercase(), e.message)
            }).collect()
        })
    }).build()?;

    // ── Structured log bindings (Unity-style console) ─────────────────────────

    m.function("log_counts", || -> Vec<i64> {
        LOG_ENTRIES.with(|l| {
            let (mut info, mut warn, mut error) = (0i64, 0i64, 0i64);
            for e in l.borrow().iter() {
                match e.level {
                    LogLevel::Info  => info  += 1,
                    LogLevel::Warn  => warn  += 1,
                    LogLevel::Error => error += 1,
                }
            }
            vec![info, warn, error]
        })
    }).build()?;

    m.function("log_get_total", || -> i64 {
        LOG_ENTRIES.with(|l| l.borrow().len() as i64)
    }).build()?;

    // Returns filtered (and optionally collapsed) entries.
    // Each inner vec: [level_str, message, count_str, time_str, global_idx_str]
    m.function("log_entries_filtered",
        |show_info: bool, show_warn: bool, show_error: bool, search: String, collapse: bool|
        -> Vec<Vec<String>>
    {
        LOG_ENTRIES.with(|l| {
            let entries = l.borrow();
            let search_lc = search.to_lowercase();

            let filtered: Vec<(usize, &LogEntry)> = entries.iter().enumerate()
                .filter(|(_, e)| {
                    let level_ok = match e.level {
                        LogLevel::Info  => show_info,
                        LogLevel::Warn  => show_warn,
                        LogLevel::Error => show_error,
                    };
                    let search_ok = search_lc.is_empty()
                        || e.message.to_lowercase().contains(&search_lc);
                    level_ok && search_ok
                })
                .collect();

            if !collapse {
                filtered.iter().map(|(idx, e)| vec![
                    e.level.as_str().to_string(),
                    e.message.clone(),
                    "1".to_string(),
                    format_log_time(e.time_ms),
                    idx.to_string(),
                ]).collect()
            } else {
                // Group identical (level, message) pairs; keep last timestamp + idx.
                let mut groups: Vec<(LogLevel, String, u32, u64, usize)> = Vec::new();
                for (idx, e) in &filtered {
                    if let Some(g) = groups.iter_mut()
                        .find(|g| g.0 == e.level && g.1 == e.message)
                    {
                        g.2 += 1;
                        g.3 = e.time_ms;
                        g.4 = *idx;
                    } else {
                        groups.push((e.level.clone(), e.message.clone(), 1, e.time_ms, *idx));
                    }
                }
                groups.iter().map(|(lv, msg, cnt, ms, idx)| vec![
                    lv.as_str().to_string(),
                    msg.clone(),
                    cnt.to_string(),
                    format_log_time(*ms),
                    idx.to_string(),
                ]).collect()
            }
        })
    }).build()?;

    m.function("log_select", |idx: i64| {
        LOG_SELECTED.with(|c| c.set(idx));
    }).build()?;

    m.function("log_selected", || -> i64 {
        LOG_SELECTED.with(|c| c.get())
    }).build()?;

    m.function("log_get_entry", |idx: i64| -> Vec<String> {
        LOG_ENTRIES.with(|l| {
            let entries = l.borrow();
            if idx < 0 || idx as usize >= entries.len() {
                return vec![];
            }
            let e = &entries[idx as usize];
            vec![
                e.level.as_str().to_string(),
                e.message.clone(),
                "1".to_string(),
                format_log_time(e.time_ms),
            ]
        })
    }).build()?;

    // ── Console UI state bindings ─────────────────────────────────────────────

    m.function("get_console_show_info",   || -> bool { CONSOLE_SHOW_INFO.with(|c| c.get()) }).build()?;
    m.function("set_console_show_info",   |v: bool| { CONSOLE_SHOW_INFO.with(|c| c.set(v)); }).build()?;
    m.function("get_console_show_warn",   || -> bool { CONSOLE_SHOW_WARN.with(|c| c.get()) }).build()?;
    m.function("set_console_show_warn",   |v: bool| { CONSOLE_SHOW_WARN.with(|c| c.set(v)); }).build()?;
    m.function("get_console_show_error",  || -> bool { CONSOLE_SHOW_ERROR.with(|c| c.get()) }).build()?;
    m.function("set_console_show_error",  |v: bool| { CONSOLE_SHOW_ERROR.with(|c| c.set(v)); }).build()?;
    m.function("get_console_collapse",    || -> bool { CONSOLE_COLLAPSE.with(|c| c.get()) }).build()?;
    m.function("set_console_collapse",    |v: bool| { CONSOLE_COLLAPSE.with(|c| c.set(v)); }).build()?;
    m.function("get_console_auto_scroll", || -> bool { CONSOLE_AUTO_SCROLL.with(|c| c.get()) }).build()?;
    m.function("set_console_auto_scroll", |v: bool| { CONSOLE_AUTO_SCROLL.with(|c| c.set(v)); }).build()?;
    m.function("get_console_search", || -> String { CONSOLE_SEARCH.with(|c| c.borrow().clone()) }).build()?;
    m.function("set_console_search", |v: String| { CONSOLE_SEARCH.with(|c| *c.borrow_mut() = v); }).build()?;
    m.function("get_console_cmd", || -> String { CONSOLE_CMD_BUF.with(|c| c.borrow().clone()) }).build()?;
    m.function("set_console_cmd", |v: String| { CONSOLE_CMD_BUF.with(|c| *c.borrow_mut() = v); }).build()?;

    // ── Entity rename / duplicate ─────────────────────────────────────────────

    m.function("rename_entity", |id: i64, name: Ref<str>| {
        with_ctx(|world, _| {
            if let Some(entity) = id_to_entity(world, id) {
                PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                    entity,
                    component: "__rename__".to_string(),
                    field:     name.as_ref().to_string(),
                    value:     fluxion_core::reflect::ReflectValue::Bool(true),
                }));
            }
        });
    }).build()?;

    m.function("duplicate_entity", |id: i64| {
        with_ctx(|world, _| {
            if let Some(entity) = id_to_entity(world, id) {
                PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                    entity,
                    component: "__duplicate__".to_string(),
                    field:     String::new(),
                    value:     fluxion_core::reflect::ReflectValue::Bool(true),
                }));
            }
        });
    }).build()?;

    // ── Transform shorthand ───────────────────────────────────────────────────

    m.function("get_world_position", |id: i64| -> Vec<f64> {
        with_ctx(|world, registry| {
            let entity = id_to_entity(world, id)?;
            let reflect = registry.get_reflect("Transform", world, entity)?;
            match reflect.get_field("world_position")? {
                ReflectValue::Vec3([x, y, z]) => Some(vec![x as f64, y as f64, z as f64]),
                _ => None,
            }
        }).flatten().unwrap_or_else(|| vec![0.0, 0.0, 0.0])
    }).build()?;

    // ── Transform fast-path (Unity API parity) ───────────────────────────────
    // These bypass the generic get_f32/set_f32 reflection path for common ops.

    m.function("get_position", |id: i64| -> Vec<f64> {
        with_ctx(|world, _| {
            let entity = id_to_entity(world, id)?;
            let t = world.get_component::<fluxion_core::transform::Transform>(entity)?;
            Some(vec![t.position.x as f64, t.position.y as f64, t.position.z as f64])
        }).flatten().unwrap_or_else(|| vec![0.0, 0.0, 0.0])
    }).build()?;

    m.function("set_position", |id: i64, vals: Vec<f64>| {
        if vals.len() >= 3 {
            if let Some(e) = with_ctx(|world, _| id_to_entity(world, id)).flatten() {
                queue_edit(e, "Transform".to_string(), "position".to_string(),
                    ReflectValue::Vec3([vals[0] as f32, vals[1] as f32, vals[2] as f32]));
            }
        }
    }).build()?;

    m.function("get_scale", |id: i64| -> Vec<f64> {
        with_ctx(|world, _| {
            let entity = id_to_entity(world, id)?;
            let t = world.get_component::<fluxion_core::transform::Transform>(entity)?;
            Some(vec![t.scale.x as f64, t.scale.y as f64, t.scale.z as f64])
        }).flatten().unwrap_or_else(|| vec![1.0, 1.0, 1.0])
    }).build()?;

    m.function("set_scale", |id: i64, vals: Vec<f64>| {
        if vals.len() >= 3 {
            if let Some(e) = with_ctx(|world, _| id_to_entity(world, id)).flatten() {
                queue_edit(e, "Transform".to_string(), "scale".to_string(),
                    ReflectValue::Vec3([vals[0] as f32, vals[1] as f32, vals[2] as f32]));
            }
        }
    }).build()?;

    // get_rotation_euler / set_rotation_euler — degrees XYZ (Unity-style)
    m.function("get_rotation_euler", |id: i64| -> Vec<f64> {
        with_ctx(|world, _| {
            let entity = id_to_entity(world, id)?;
            let t = world.get_component::<fluxion_core::transform::Transform>(entity)?;
            let (rx, ry, rz) = t.rotation.to_euler(glam::EulerRot::XYZ);
            Some(vec![rx.to_degrees() as f64, ry.to_degrees() as f64, rz.to_degrees() as f64])
        }).flatten().unwrap_or_else(|| vec![0.0, 0.0, 0.0])
    }).build()?;

    m.function("set_rotation_euler", |id: i64, vals: Vec<f64>| {
        if vals.len() >= 3 {
            if let Some(e) = with_ctx(|world, _| id_to_entity(world, id)).flatten() {
                let rx = (vals[0] as f32).to_radians();
                let ry = (vals[1] as f32).to_radians();
                let rz = (vals[2] as f32).to_radians();
                let q = glam::Quat::from_euler(EulerRot::XYZ, rx, ry, rz);
                queue_edit(e, "Transform".to_string(), "rotation".to_string(),
                    ReflectValue::Quat([q.x, q.y, q.z, q.w]));
            }
        }
    }).build()?;

    // ── Light fast-path ───────────────────────────────────────────────────────

    m.function("get_light_color", |id: i64| -> Vec<f64> {
        with_ctx(|world, _| {
            let entity = id_to_entity(world, id)?;
            let l = world.get_component::<fluxion_core::components::Light>(entity)?;
            Some(vec![l.color[0] as f64, l.color[1] as f64, l.color[2] as f64])
        }).flatten().unwrap_or_else(|| vec![1.0, 1.0, 1.0])
    }).build()?;

    m.function("set_light_color", |id: i64, vals: Vec<f64>| {
        if vals.len() >= 3 {
            if let Some(e) = with_ctx(|world, _| id_to_entity(world, id)).flatten() {
                queue_edit(e, "Light".to_string(), "color".to_string(),
                    ReflectValue::Color3([vals[0] as f32, vals[1] as f32, vals[2] as f32]));
            }
        }
    }).build()?;

    m.function("get_light_intensity", |id: i64| -> f64 {
        with_ctx(|world, _| {
            let entity = id_to_entity(world, id)?;
            let l = world.get_component::<fluxion_core::components::Light>(entity)?;
            Some(l.intensity as f64)
        }).flatten().unwrap_or(1.0)
    }).build()?;

    m.function("set_light_intensity", |id: i64, val: f64| {
        if let Some(e) = with_ctx(|world, _| id_to_entity(world, id)).flatten() {
            queue_edit(e, "Light".to_string(), "intensity".to_string(),
                ReflectValue::F32(val as f32));
        }
    }).build()?;

    // ── GameObject.Find parity ────────────────────────────────────────────────

    m.function("find_entity_by_name", |name: Ref<str>| -> i64 {
        with_ctx(|world, _| {
            world.find_by_name(&name).map(entity_to_id)
        }).flatten().unwrap_or(-1)
    }).build()?;

    // ── Time helpers (Unity parity: Time.deltaTime / Time.time) ──────────────
    // FRAME_TIME_MS is set each frame from main.rs (milliseconds).

    m.function("time_delta", || -> f64 {
        FRAME_TIME_MS.with(|c| c.get()) / 1000.0
    }).build()?;

    m.function("time_elapsed", || -> f64 {
        TIME_ELAPSED.with(|c| c.get())
    }).build()?;

    // ── Stats / debugger ─────────────────────────────────────────────────────

    m.function("entity_count", || -> i64 {
        with_ctx(|world, _| world.entity_count() as i64).unwrap_or(0)
    }).build()?;

    m.function("component_count", || -> i64 {
        with_ctx(|world, _| {
            world.all_entities().map(|e| world.component_names(e).len() as i64).sum::<i64>()
        }).unwrap_or(0)
    }).build()?;

    m.function("frame_time_ms", || -> f64 {
        FRAME_TIME_MS.with(|c| c.get())
    }).build()?;

    m.function("log_error_count", || -> i64 {
        LOG_ENTRIES.with(|l| {
            l.borrow().iter().filter(|e| e.level == LogLevel::Error).count() as i64
        })
    }).build()?;

    // ── Undo/redo state ───────────────────────────────────────────────────────

    m.function("can_undo", || -> bool {
        UNDO_STATE.with(|c| c.get().0)
    }).build()?;

    m.function("can_redo", || -> bool {
        UNDO_STATE.with(|c| c.get().1)
    }).build()?;

    // trigger_undo() / trigger_redo() — queue action signals consumed by main.rs
    m.function("trigger_undo", || {
        ACTION_SIGNALS.with(|s| s.borrow_mut().push("do_undo".to_string()));
    }).build()?;

    m.function("trigger_redo", || {
        ACTION_SIGNALS.with(|s| s.borrow_mut().push("do_redo".to_string()));
    }).build()?;

    // ── Euler angle helpers (degrees) ─────────────────────────────────────────

    m.function("get_euler", |id: i64, component: Ref<str>, field: Ref<str>| -> Vec<f64> {
        with_ctx(|world, registry| {
            let entity = id_to_entity(world, id)?;
            let reflect = registry.get_reflect(&component, world, entity)?;
            match reflect.get_field(&field)? {
                ReflectValue::Quat([x, y, z, w]) => {
                    let q = glam::Quat::from_xyzw(x, y, z, w);
                    let (rx, ry, rz) = q.to_euler(EulerRot::XYZ);
                    Some(vec![
                        rx.to_degrees() as f64,
                        ry.to_degrees() as f64,
                        rz.to_degrees() as f64,
                    ])
                }
                _ => None,
            }
        }).flatten().unwrap_or_else(|| vec![0.0, 0.0, 0.0])
    }).build()?;

    m.function("set_euler", |id: i64, component: String, field: String, vals: Vec<f64>| {
        if vals.len() >= 3 {
            if let Some(e) = with_ctx(|world, _| id_to_entity(world, id)).flatten() {
                let rx = (vals[0] as f32).to_radians();
                let ry = (vals[1] as f32).to_radians();
                let rz = (vals[2] as f32).to_radians();
                let q = glam::Quat::from_euler(EulerRot::XYZ, rx, ry, rz);
                queue_edit(e, component, field,
                    ReflectValue::Quat([q.x, q.y, q.z, q.w]));
            }
        }
    }).build()?;

    // ── Editor shell state (for toolbar.rn / menubar.rn) ─────────────────────

    m.function("get_editor_mode", || -> String {
        EDITOR_MODE.with(|c| c.borrow().clone())
    }).build()?;

    m.function("set_editor_mode", |mode: Ref<str>| {
        EDITOR_MODE.with(|c| *c.borrow_mut() = mode.as_ref().to_string());
    }).build()?;

    m.function("get_transform_tool", || -> String {
        TRANSFORM_TOOL.with(|c| c.borrow().clone())
    }).build()?;

    m.function("set_transform_tool", |tool: Ref<str>| {
        TRANSFORM_TOOL.with(|c| *c.borrow_mut() = tool.as_ref().to_string());
    }).build()?;

    m.function("get_project_name", || -> String {
        PROJECT_NAME.with(|c| c.borrow().clone())
    }).build()?;

    m.function("get_scene_name", || -> String {
        SCENE_NAME.with(|c| c.borrow().clone())
    }).build()?;

    m.function("do_new_scene", || {
        ACTION_SIGNALS.with(|s| s.borrow_mut().push("new_scene".to_string()));
    }).build()?;

    m.function("do_open_scene", || {
        ACTION_SIGNALS.with(|s| s.borrow_mut().push("open_scene".to_string()));
    }).build()?;

    m.function("do_save_scene", || {
        ACTION_SIGNALS.with(|s| s.borrow_mut().push("save_scene".to_string()));
    }).build()?;

    m.function("exit_app", || {
        ACTION_SIGNALS.with(|s| s.borrow_mut().push("exit".to_string()));
    }).build()?;

    m.function("push_action", |signal: String| {
        ACTION_SIGNALS.with(|s| s.borrow_mut().push(signal));
    }).build()?;

    m.function("lsp_running", || -> bool {
        crate::lsp_manager::LSP_RUNNING.load(std::sync::atomic::Ordering::Relaxed)
    }).build()?;

    // ── Editor camera state (read/write by editor_camera.rn) ─────────────────

    m.function("get_editor_cam_pos", || -> Vec<f64> {
        EDITOR_CAM.with(|c| c.borrow().pos.to_vec())
    }).build()?;

    m.function("set_editor_cam_pos", |vals: Vec<f64>| {
        if vals.len() >= 3 {
            EDITOR_CAM.with(|c| c.borrow_mut().pos = [vals[0], vals[1], vals[2]]);
            EDITOR_CAM_DIRTY.with(|c| c.set(true));
        }
    }).build()?;

    m.function("get_editor_cam_yaw", || -> f64 {
        EDITOR_CAM.with(|c| c.borrow().yaw)
    }).build()?;

    m.function("set_editor_cam_yaw", |v: f64| {
        EDITOR_CAM.with(|c| c.borrow_mut().yaw = v);
        EDITOR_CAM_DIRTY.with(|c| c.set(true));
    }).build()?;

    m.function("get_editor_cam_pitch", || -> f64 {
        EDITOR_CAM.with(|c| c.borrow().pitch)
    }).build()?;

    m.function("set_editor_cam_pitch", |v: f64| {
        EDITOR_CAM.with(|c| c.borrow_mut().pitch = v);
        EDITOR_CAM_DIRTY.with(|c| c.set(true));
    }).build()?;

    m.function("get_editor_cam_target", || -> Vec<f64> {
        EDITOR_CAM.with(|c| c.borrow().target.to_vec())
    }).build()?;

    m.function("set_editor_cam_target", |vals: Vec<f64>| {
        if vals.len() >= 3 {
            EDITOR_CAM.with(|c| c.borrow_mut().target = [vals[0], vals[1], vals[2]]);
            EDITOR_CAM_DIRTY.with(|c| c.set(true));
        }
    }).build()?;

    m.function("get_editor_cam_speed", || -> f64 {
        EDITOR_CAM.with(|c| c.borrow().speed)
    }).build()?;

    m.function("set_editor_cam_speed", |v: f64| {
        EDITOR_CAM.with(|c| c.borrow_mut().speed = v.max(0.1));
    }).build()?;

    m.function("get_editor_cam_fov", || -> f64 {
        EDITOR_CAM.with(|c| c.borrow().fov)
    }).build()?;

    m.function("set_editor_cam_fov", |v: f64| {
        EDITOR_CAM.with(|c| c.borrow_mut().fov = v.clamp(1.0, 170.0));
    }).build()?;

    m.function("get_editor_cam_near", || -> f64 {
        EDITOR_CAM.with(|c| c.borrow().near)
    }).build()?;

    m.function("set_editor_cam_near", |v: f64| {
        EDITOR_CAM.with(|c| c.borrow_mut().near = v.max(0.001));
    }).build()?;

    m.function("get_editor_cam_far", || -> f64 {
        EDITOR_CAM.with(|c| c.borrow().far)
    }).build()?;

    m.function("set_editor_cam_far", |v: f64| {
        EDITOR_CAM.with(|c| c.borrow_mut().far = v.max(1.0));
    }).build()?;

    // ── Gameplay script helpers (used by inspector.rn / assets.rn) ────────────

    // script_error(entity_id, script_name) → String
    m.function("script_error", |id: i64, name: String| -> String {
        if id < 0 { return String::new(); }
        crate::rune_bindings::gameplay_module::get_script_error(id as u64, &name)
    }).build()?;

    // script_entries(entity_id) → Vec of [name, path, enabled_str]
    m.function("script_entries", |entity_id: i64| -> Vec<Vec<String>> {
        if entity_id < 0 { return Vec::new(); }
        ENTITY_CACHE.with(|cache| {
            let map = cache.borrow();
            if let Some(&eid) = map.get(&(entity_id as u64)) {
                WORLD_PTR.with(|w| {
                    let ptr = w.get()?;
                    let world = unsafe { ptr.as_ref() };
                    let bundle = world.get_component::<fluxion_core::ScriptBundle>(eid)?;
                    Some(bundle.scripts.iter().map(|e| {
                        vec![e.name.clone(), e.path.clone(), if e.enabled { "true".to_string() } else { "false".to_string() }]
                    }).collect::<Vec<_>>())
                }).unwrap_or_default()
            } else {
                Vec::new()
            }
        })
    }).build()?;

    // add_script(entity_id, rel_path) — adds ScriptEntry to ScriptBundle
    m.function("add_script", |entity_id: i64, path: String| {
        use fluxion_core::reflect::ReflectValue;
        ENTITY_CACHE.with(|cache| {
            let map = cache.borrow();
            if let Some(&eid) = map.get(&(entity_id as u64)) {
                PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                    entity:    eid,
                    component: "__add_script__".to_string(),
                    field:     path,
                    value:     ReflectValue::Bool(true),
                }));
            }
        });
    }).build()?;

    // remove_script(entity_id, script_name) — removes entry by name from ScriptBundle
    m.function("remove_script", |entity_id: i64, name: String| {
        use fluxion_core::reflect::ReflectValue;
        ENTITY_CACHE.with(|cache| {
            let map = cache.borrow();
            if let Some(&eid) = map.get(&(entity_id as u64)) {
                PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                    entity:    eid,
                    component: "__remove_script__".to_string(),
                    field:     name,
                    value:     ReflectValue::Bool(true),
                }));
            }
        });
    }).build()?;

    // script_compile_summary() → [total, errors] or [] if not compiled yet
    m.function("script_compile_summary", || -> Vec<i64> {
        let (total, errors) = crate::rune_bindings::gameplay_module::get_compile_summary();
        if total < 0 { vec![] } else { vec![total, errors] }
    }).build()?;

    // script_fields(entity_id, script_name) →
    //   [[name, value_json, hint, label, min, max, tooltip, hidden, readonly], …]
    // Columns 0-1 are the field name and JSON value.
    // Columns 2-8 are ScriptFieldMeta: hint, label, min, max, tooltip, hidden("1"/"0"), readonly("1"/"0").
    m.function("script_fields", |entity_id: i64, script_name: String| -> Vec<Vec<String>> {
        if entity_id < 0 { return Vec::new(); }
        ENTITY_CACHE.with(|cache| {
            let map = cache.borrow();
            if let Some(&eid) = map.get(&(entity_id as u64)) {
                WORLD_PTR.with(|w| {
                    let ptr = w.get()?;
                    let world = unsafe { ptr.as_ref() };
                    let bundle = world.get_component::<fluxion_core::ScriptBundle>(eid)?;
                    let entry = bundle.scripts.iter().find(|e| e.name == script_name)?;
                    Some(entry.fields.iter().map(|f| {
                        vec![
                            f.name.clone(),
                            f.value.to_string(),
                            f.meta.hint.clone(),
                            f.meta.label.clone(),
                            f.meta.min.to_string(),
                            f.meta.max.to_string(),
                            f.meta.tooltip.clone(),
                            if f.meta.hidden    { "1".into() } else { "0".into() },
                            if f.meta.read_only { "1".into() } else { "0".into() },
                        ]
                    }).collect::<Vec<_>>())
                }).unwrap_or_default()
            } else {
                Vec::new()
            }
        })
    }).build()?;

    // str_strip_prefix(s, prefix) → String: strips prefix from s if present, else returns s unchanged.
    m.function("str_strip_prefix", |s: String, prefix: String| -> String {
        s.strip_prefix(prefix.as_str()).unwrap_or(&s).to_string()
    }).build()?;

    // parse_f64(s) → [ok, value]: ok=1.0 if s is a valid f64, 0.0 otherwise.
    // Used by inspector.rn to decide between drag-widget and text-input for script fields.
    m.function("parse_f64", |s: String| -> Vec<f64> {
        match s.parse::<f64>() {
            Ok(v) => vec![1.0, v],
            Err(_) => vec![0.0, 0.0],
        }
    }).build()?;

    // script_field_decl(script_name) → [[name, type_str, hint, min_str, max_str], …]
    // Returns declared field metadata registered by `fluxion::script::declare_field`.
    // The inspector uses this to choose the correct widget per script field.
    m.function("script_field_decl", |script_name: String| -> Vec<Vec<String>> {
        crate::rune_bindings::gameplay_module::get_field_decls(&script_name)
    }).build()?;

    // ── Asset info bindings (Phase F) ─────────────────────────────────────────

    // texture_info(path) → [width_str, height_str, format_str]
    // Reads image dimensions from the file header without full decode.
    // Returns ["?", "?", ext] if unreadable.
    m.function("texture_info", |path: String| -> Vec<String> {
        let full = format!("assets/{}", path);
        let ext = std::path::Path::new(&full)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let format_str = match ext.as_str() {
            "png"  => "PNG",
            "jpg" | "jpeg" => "JPEG",
            "webp" => "WebP",
            "bmp"  => "BMP",
            "tga"  => "TGA",
            "hdr"  => "HDR",
            "exr"  => "EXR",
            "ktx"  => "KTX",
            "dds"  => "DDS",
            _      => "Unknown",
        };
        match image::image_dimensions(&full) {
            Ok((w, h)) => vec![w.to_string(), h.to_string(), format_str.to_string()],
            Err(_)     => vec!["?".to_string(), "?".to_string(), format_str.to_string()],
        }
    }).build()?;

    // model_info(path) → [format_str, mesh_count_str]
    // For glTF/GLB: counts mesh primitives from JSON.
    // For other formats: returns extension + "1".
    m.function("model_info", |path: String| -> Vec<String> {
        let full = format!("assets/{}", path);
        let ext = std::path::Path::new(&full)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let format_str = match ext.as_str() {
            "glb"  => "GLB (Binary glTF)",
            "gltf" => "glTF",
            "obj"  => "Wavefront OBJ",
            "fbx"  => "FBX",
            _      => "Unknown",
        };
        // Count meshes from glTF JSON: read file, find "meshes":[...] and count objects
        let mesh_count = (|| -> Option<usize> {
            let content = std::fs::read(&full).ok()?;
            // For GLB: JSON starts at byte 20, length at bytes 12-16
            let json_str = if ext == "glb" {
                if content.len() < 20 { return None; }
                let json_len = u32::from_le_bytes([content[12], content[13], content[14], content[15]]) as usize;
                if content.len() < 20 + json_len { return None; }
                String::from_utf8(content[20..20 + json_len].to_vec()).ok()?
            } else if ext == "gltf" {
                String::from_utf8(content).ok()?
            } else {
                return None;
            };
            // Quick count: number of "primitives" arrays → proxy for submesh count
            Some(json_str.matches("\"primitives\"").count().max(1))
        })().unwrap_or(1);
        vec![format_str.to_string(), mesh_count.to_string()]
    }).build()?;

    // ── Viewport stats ────────────────────────────────────────────────────────
    // frame_stats() → [draw_calls, entity_count, frame_ms]
    m.function("frame_stats", || -> Vec<f64> {
        let (dc, ec) = FRAME_STATS.with(|c| c.get());
        let ms = FRAME_TIME_MS.with(|c| c.get());
        vec![dc as f64, ec as f64, ms]
    }).build()?;

    // ── Snap settings ─────────────────────────────────────────────────────────
    m.function("get_snap_enabled",   || -> bool { SNAP_ENABLED  .with(|c| c.get()) }).build()?;
    m.function("set_snap_enabled",   |v: bool|   { SNAP_ENABLED  .with(|c| c.set(v)); }).build()?;
    m.function("get_snap_translate", || -> f64   { SNAP_TRANSLATE.with(|c| c.get()) }).build()?;
    m.function("set_snap_translate", |v: f64|    { SNAP_TRANSLATE.with(|c| c.set(v.max(0.001))); }).build()?;
    m.function("get_snap_rotate",    || -> f64   { SNAP_ROTATE   .with(|c| c.get()) }).build()?;
    m.function("set_snap_rotate",    |v: f64|    { SNAP_ROTATE   .with(|c| c.set(v.max(0.1))); }).build()?;
    m.function("get_snap_scale",     || -> f64   { SNAP_SCALE    .with(|c| c.get()) }).build()?;
    m.function("set_snap_scale",     |v: f64|    { SNAP_SCALE    .with(|c| c.set(v.max(0.001))); }).build()?;

    // ── Multi-selection ───────────────────────────────────────────────────────
    m.function("get_multi_selected", || -> Vec<i64> {
        SELECTED_MULTI.with(|s| s.borrow().iter().map(|e| entity_to_id(*e)).collect())
    }).build()?;

    m.function("add_to_selection", |id: i64| {
        if is_protected_editor_cam(id) { return; }
        with_ctx(|world, _| {
            if let Some(entity) = id_to_entity(world, id) {
                SELECTED_MULTI.with(|s| {
                    let mut v = s.borrow_mut();
                    if !v.contains(&entity) { v.push(entity); }
                });
                SELECTED.with(|s| *s.borrow_mut() = Some(entity));
            }
        });
    }).build()?;

    m.function("remove_from_selection", |id: i64| {
        with_ctx(|world, _| {
            if let Some(entity) = id_to_entity(world, id) {
                SELECTED_MULTI.with(|s| s.borrow_mut().retain(|e| *e != entity));
            }
        });
    }).build()?;

    m.function("clear_multi_selection", || {
        SELECTED_MULTI.with(|s| s.borrow_mut().clear());
    }).build()?;

    m.function("is_multi_selected", |id: i64| -> bool {
        with_ctx(|world, _| {
            id_to_entity(world, id).map(|entity| {
                SELECTED_MULTI.with(|s| s.borrow().contains(&entity))
            }).unwrap_or(false)
        }).unwrap_or(false)
    }).build()?;

    // ── Prefab creation ───────────────────────────────────────────────────────
    // create_prefab(entity_id, rel_path) — saves entity as prefab .scene file
    m.function("create_prefab", |entity_id: i64, path: String| {
        use fluxion_core::reflect::ReflectValue;
        ENTITY_CACHE.with(|cache| {
            let map = cache.borrow();
            if let Some(&eid) = map.get(&(entity_id as u64)) {
                PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                    entity:    eid,
                    component: "__create_prefab__".to_string(),
                    field:     path,
                    value:     ReflectValue::Bool(true),
                }));
            }
        });
    }).build()?;

    // set_prefab_pending(entity_id) — remember which entity is waiting for a prefab path.
    m.function("set_prefab_pending", |id: i64| {
        PREFAB_PENDING.with(|c| c.set(id));
    }).build()?;

    // get_prefab_pending() → entity id (-1 if none), also clears it.
    m.function("get_prefab_pending", || -> i64 {
        PREFAB_PENDING.with(|c| { let v = c.get(); c.set(-1); v })
    }).build()?;

    // ── CSG box gizmo ─────────────────────────────────────────────────────────

    // selected_has_csg() → bool  — true when the selected entity has a CsgShape component.
    m.function("selected_has_csg", || -> bool {
        use fluxion_core::components::CsgShape;
        let sel = SELECTED.with(|s| *s.borrow());
        let eid = match sel { Some(e) => e, None => return false };
        with_world(|w| w.has_component::<CsgShape>(eid)).unwrap_or(false)
    }).build()?;

    // get_box_gizmo_mode() → "" | "face" | "axis"
    m.function("get_box_gizmo_mode", || -> String {
        match BOX_GIZMO_MODE.with(|c| c.get()) {
            1 => "face".to_string(),
            2 => "axis".to_string(),
            _ => String::new(),
        }
    }).build()?;

    // set_box_gizmo_mode(mode: String)  — "" | "face" | "axis"
    m.function("set_box_gizmo_mode", |mode: String| {
        let v = match mode.as_str() {
            "face" => 1,
            "axis" => 2,
            _      => 0,
        };
        BOX_GIZMO_MODE.with(|c| c.set(v));
    }).build()?;

    // set_script_field(entity_id, script_name, field_name, value_str)
    // Packs as "script_name\x00field_name\x00value_str" in the PendingEdit field.
    m.function("set_script_field", |entity_id: i64, script_name: String, field_name: String, value_str: String| {
        use fluxion_core::reflect::ReflectValue;
        ENTITY_CACHE.with(|cache| {
            let map = cache.borrow();
            if let Some(&eid) = map.get(&(entity_id as u64)) {
                let packed = format!("{}\x00{}\x00{}", script_name, field_name, value_str);
                PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                    entity:    eid,
                    component: "__set_script_field__".to_string(),
                    field:     packed,
                    value:     ReflectValue::Bool(true),
                }));
            }
        });
    }).build()?;

    // ── Utility: split string by whitespace (for console command parsing) ────────
    m.function("cmd_split", |s: Ref<str>| -> Vec<String> {
        s.as_ref()
            .split_whitespace()
            .map(|p| p.to_string())
            .collect()
    }).build()?;

    // ── Collision layers on RigidBody ─────────────────────────────────────────

    m.function("get_collision_layer", |entity_id: i64| -> i64 {
        if entity_id < 0 { return 1; }
        ENTITY_CACHE.with(|cache| {
            let map = cache.borrow();
            if let Some(&eid) = map.get(&(entity_id as u64)) {
                WORLD_PTR.with(|w| {
                    let ptr = w.get()?;
                    let world = unsafe { ptr.as_ref() };
                    let rb = world.get_component::<fluxion_core::RigidBody>(eid)?;
                    Some(rb.collision_layer as i64)
                }).unwrap_or(1)
            } else { 1 }
        })
    }).build()?;

    m.function("set_collision_layer", |entity_id: i64, layer: i64| {
        if entity_id < 0 { return; }
        ENTITY_CACHE.with(|cache| {
            if let Some(&eid) = cache.borrow().get(&(entity_id as u64)) {
                PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                    entity:    eid,
                    component: "__set_collision_layer__".to_string(),
                    field:     layer.to_string(),
                    value:     ReflectValue::Bool(true),
                }));
            }
        });
    }).build()?;

    m.function("get_collision_mask", |entity_id: i64| -> i64 {
        if entity_id < 0 { return -1; }
        ENTITY_CACHE.with(|cache| {
            let map = cache.borrow();
            if let Some(&eid) = map.get(&(entity_id as u64)) {
                WORLD_PTR.with(|w| {
                    let ptr = w.get()?;
                    let world = unsafe { ptr.as_ref() };
                    let rb = world.get_component::<fluxion_core::RigidBody>(eid)?;
                    Some(rb.collision_mask as i64)
                }).unwrap_or(-1)
            } else { -1 }
        })
    }).build()?;

    m.function("set_collision_mask", |entity_id: i64, mask: i64| {
        if entity_id < 0 { return; }
        ENTITY_CACHE.with(|cache| {
            if let Some(&eid) = cache.borrow().get(&(entity_id as u64)) {
                PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                    entity:    eid,
                    component: "__set_collision_mask__".to_string(),
                    field:     mask.to_string(),
                    value:     ReflectValue::Bool(true),
                }));
            }
        });
    }).build()?;

    m.function("get_physics_material", |entity_id: i64| -> String {
        if entity_id < 0 { return String::new(); }
        ENTITY_CACHE.with(|cache| {
            let map = cache.borrow();
            if let Some(&eid) = map.get(&(entity_id as u64)) {
                WORLD_PTR.with(|w| {
                    let ptr = w.get()?;
                    let world = unsafe { ptr.as_ref() };
                    let rb = world.get_component::<fluxion_core::RigidBody>(eid)?;
                    Some(rb.physics_material_path.clone())
                }).unwrap_or_default()
            } else { String::new() }
        })
    }).build()?;

    m.function("set_physics_material", |entity_id: i64, path: String| {
        if entity_id < 0 { return; }
        ENTITY_CACHE.with(|cache| {
            if let Some(&eid) = cache.borrow().get(&(entity_id as u64)) {
                PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                    entity:    eid,
                    component: "__set_physics_material__".to_string(),
                    field:     path,
                    value:     ReflectValue::Bool(true),
                }));
            }
        });
    }).build()?;

    // ── Material asset bindings ───────────────────────────────────────────────

    m.function("read_material", |path: String| -> String {
        PROJECT_ROOT.with(|root| {
            let full = root.borrow().join(&path);
            std::fs::read_to_string(&full)
                .or_else(|_| std::fs::read_to_string(&path))
                .unwrap_or_default()
        })
    }).build()?;

    m.function("write_material", |path: String, json: String| {
        PENDING.with(|p| p.borrow_mut().push(PendingEdit {
            entity:    fluxion_core::EntityId::INVALID,
            component: "__write_material__".to_string(),
            field:     path,
            value:     ReflectValue::Str(json),
        }));
    }).build()?;

    m.function("create_material", |path: String| {
        let root = PROJECT_ROOT.with(|r| r.borrow().clone());
        if root == std::path::PathBuf::new() {
            log::warn!("[create_material] PROJECT_ROOT not set");
            return;
        }
        let full = root.join("assets").join(&path);
        if full.exists() {
            log::warn!("[create_material] already exists: {}", full.display());
            return;
        }
        if let Some(parent) = full.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let default_asset = fluxion_renderer::material::MaterialAsset::default();
        match serde_json::to_vec_pretty(&default_asset) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(&full, &bytes) {
                    log::error!("[create_material] failed to write {}: {e}", full.display());
                } else {
                    log::info!("[create_material] created: {}", full.display());
                    ACTION_SIGNALS.with(|s| s.borrow_mut().push("rescan_assets".to_string()));
                }
            }
            Err(e) => log::error!("[create_material] serialize failed: {e}"),
        }
    }).build()?;

    m.function("material_slots", |entity_id: i64| -> Vec<Vec<String>> {
        if entity_id < 0 { return Vec::new(); }
        ENTITY_CACHE.with(|cache| {
            let map = cache.borrow();
            if let Some(&eid) = map.get(&(entity_id as u64)) {
                WORLD_PTR.with(|w| {
                    let ptr = w.get()?;
                    let world = unsafe { ptr.as_ref() };
                    let mr = world.get_component::<fluxion_core::MeshRenderer>(eid)?;
                    let result: Vec<Vec<String>> = mr.material_slots.iter().map(|s| {
                        vec![
                            s.slot_index.to_string(),
                            s.name.clone(),
                            s.material_path.clone().unwrap_or_default(),
                        ]
                    }).collect();
                    Some(result)
                }).unwrap_or_default()
            } else { Vec::new() }
        })
    }).build()?;

    m.function("set_material_slot", |entity_id: i64, slot_idx: i64, path: String| {
        if entity_id < 0 { return; }
        ENTITY_CACHE.with(|cache| {
            if let Some(&eid) = cache.borrow().get(&(entity_id as u64)) {
                PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                    entity:    eid,
                    component: "__set_material_slot__".to_string(),
                    field:     slot_idx.to_string(),
                    value:     ReflectValue::Str(path),
                }));
            }
        });
    }).build()?;

    m.function("clear_material_slot", |entity_id: i64, slot_idx: i64| {
        if entity_id < 0 { return; }
        ENTITY_CACHE.with(|cache| {
            if let Some(&eid) = cache.borrow().get(&(entity_id as u64)) {
                PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                    entity:    eid,
                    component: "__set_material_slot__".to_string(),
                    field:     slot_idx.to_string(),
                    value:     ReflectValue::Str(String::new()),
                }));
            }
        });
    }).build()?;

    m.function("get_material_path", |entity_id: i64| -> String {
        if entity_id < 0 { return String::new(); }
        ENTITY_CACHE.with(|cache| {
            let map = cache.borrow();
            if let Some(&eid) = map.get(&(entity_id as u64)) {
                WORLD_PTR.with(|w| {
                    let ptr = w.get()?;
                    let world = unsafe { ptr.as_ref() };
                    let mr = world.get_component::<fluxion_core::MeshRenderer>(eid)?;
                    Some(mr.material_path.clone().unwrap_or_default())
                }).unwrap_or_default()
            } else { String::new() }
        })
    }).build()?;

    m.function("set_material_path", |entity_id: i64, path: String| {
        if entity_id < 0 { return; }
        ENTITY_CACHE.with(|cache| {
            if let Some(&eid) = cache.borrow().get(&(entity_id as u64)) {
                PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                    entity:    eid,
                    component: "__set_material_path__".to_string(),
                    field:     path,
                    value:     ReflectValue::Bool(true),
                }));
            }
        });
    }).build()?;

    // ── Material editor (in-memory JSON cache) ────────────────────────────────

    m.function("mat_load", |path: String| -> bool {
        // If already cached, do NOT reload from disk — that would discard unsaved edits.
        let already = MATERIAL_CACHE.with(|c| c.borrow().contains_key(&path));
        if already { return true; }
        let json = PROJECT_ROOT.with(|root| {
            let full = root.borrow().join(&path);
            std::fs::read_to_string(&full)
                .or_else(|_| {
                    let with_assets = root.borrow().join("assets").join(&path);
                    std::fs::read_to_string(&with_assets)
                })
                .or_else(|_| std::fs::read_to_string(&path))
                .ok()
        });
        if let Some(text) = json {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                MATERIAL_CACHE.with(|c| c.borrow_mut().insert(path, val));
                return true;
            }
        }
        false
    }).build()?;

    // mat_unload(path) — evict a material from the cache (forces reload on next mat_load).
    m.function("mat_unload", |path: String| {
        MATERIAL_CACHE.with(|c| c.borrow_mut().remove(&path));
    }).build()?;

    m.function("mat_f32", |path: String, field: String| -> f64 {
        MATERIAL_CACHE.with(|c| {
            c.borrow().get(&path)
                .and_then(|v| v.get(&field))
                .and_then(|f| f.as_f64())
                .unwrap_or(0.0)
        })
    }).build()?;

    m.function("mat_bool", |path: String, field: String| -> bool {
        MATERIAL_CACHE.with(|c| {
            c.borrow().get(&path)
                .and_then(|v| v.get(&field))
                .and_then(|f| f.as_bool())
                .unwrap_or(false)
        })
    }).build()?;

    m.function("mat_str", |path: String, field: String| -> String {
        MATERIAL_CACHE.with(|c| {
            c.borrow().get(&path)
                .and_then(|v| v.get(&field))
                .and_then(|f| f.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default()
        })
    }).build()?;

    m.function("mat_color3", |path: String, field: String| -> Vec<f64> {
        MATERIAL_CACHE.with(|c| {
            c.borrow().get(&path)
                .and_then(|v| v.get(&field))
                .and_then(|a| a.as_array())
                .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect())
                .unwrap_or_else(|| vec![0.0, 0.0, 0.0])
        })
    }).build()?;

    m.function("mat_color4", |path: String, field: String| -> Vec<f64> {
        MATERIAL_CACHE.with(|c| {
            c.borrow().get(&path)
                .and_then(|v| v.get(&field))
                .and_then(|a| a.as_array())
                .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect())
                .unwrap_or_else(|| vec![1.0, 1.0, 1.0, 1.0])
        })
    }).build()?;

    m.function("mat_set_f32", |path: String, field: String, val: f64| {
        MATERIAL_CACHE.with(|c| {
            if let Some(obj) = c.borrow_mut().get_mut(&path) {
                if let Some(map) = obj.as_object_mut() {
                    map.insert(field, serde_json::json!(val));
                }
            }
        });
    }).build()?;

    m.function("mat_set_bool", |path: String, field: String, val: bool| {
        MATERIAL_CACHE.with(|c| {
            if let Some(obj) = c.borrow_mut().get_mut(&path) {
                if let Some(map) = obj.as_object_mut() {
                    map.insert(field, serde_json::json!(val));
                }
            }
        });
    }).build()?;

    m.function("mat_set_str", |path: String, field: String, val: String| {
        MATERIAL_CACHE.with(|c| {
            if let Some(obj) = c.borrow_mut().get_mut(&path) {
                if let Some(map) = obj.as_object_mut() {
                    if val.is_empty() {
                        map.insert(field, serde_json::Value::Null);
                    } else {
                        map.insert(field, serde_json::json!(val));
                    }
                }
            }
        });
    }).build()?;

    m.function("mat_set_color3", |path: String, field: String, r: f64, g: f64, b: f64| {
        MATERIAL_CACHE.with(|c| {
            if let Some(obj) = c.borrow_mut().get_mut(&path) {
                if let Some(map) = obj.as_object_mut() {
                    map.insert(field, serde_json::json!([r, g, b]));
                }
            }
        });
    }).build()?;

    m.function("mat_set_color4", |path: String, field: String, vals: Vec<f64>| {
        MATERIAL_CACHE.with(|c| {
            if let Some(obj) = c.borrow_mut().get_mut(&path) {
                if let Some(map) = obj.as_object_mut() {
                    map.insert(field, serde_json::json!(vals));
                }
            }
        });
    }).build()?;

    // parse_entity_id(s) → i64  — parses a decimal string entity id, or -1 on failure.
    // Used by hierarchy.rn to decode the reparent: DnD action string.
    m.function("parse_entity_id", |s: Ref<str>| -> i64 {
        s.as_ref().parse::<i64>().unwrap_or(-1)
    }).build()?;

    // is_ancestor_of(ancestor_id, entity_id) → bool
    // Returns true if ancestor_id is a direct or indirect parent of entity_id.
    // Used by hierarchy.rn to prevent dropping an entity onto one of its own descendants.
    m.function("is_ancestor_of", |ancestor_id: i64, entity_id: i64| -> bool {
        with_ctx(|world, _| {
            let ancestor = id_to_entity(world, ancestor_id)?;
            let entity   = id_to_entity(world, entity_id)?;
            Some(world.is_ancestor_of(ancestor, entity))
        }).flatten().unwrap_or(false)
    }).build()?;

    // editor_camera_id() → i64  — returns bits of the protected editor fly-cam entity, or -1
    m.function("editor_camera_id", || -> i64 {
        EDITOR_CAM_ID.with(|c| c.get().map(|e| e.to_bits() as i64).unwrap_or(-1))
    }).build()?;

    // script_hotreload_pending() → bool — true once after a .rn file changed; consumed on read.
    m.function("script_hotreload_pending", || -> bool {
        let pending = crate::rune_bindings::gameplay_module::take_script_hotreload_pending();
        if pending {
            HOTRELOAD_TOAST_TIMER.with(|t| t.set(3.0));
        }
        pending
    }).build()?;

    // hotreload_toast_visible() → bool — true while the 3-second toast countdown is active.
    m.function("hotreload_toast_visible", || -> bool {
        HOTRELOAD_TOAST_TIMER.with(|t| t.get() > 0.0)
    }).build()?;

    // mat_alpha_mode(path) → String: "opaque" | "blend" | "mask:<cutoff>"
    m.function("mat_alpha_mode", |path: String| -> String {
        MATERIAL_CACHE.with(|c| {
            let cache = c.borrow();
            let Some(obj) = cache.get(&path) else { return "opaque".to_string() };
            let v = obj.get("alphaMode").cloned().unwrap_or(serde_json::Value::Null);
            match &v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Object(m) => {
                    if let Some(cutoff) = m.get("mask").and_then(|x| x.as_f64()) {
                        format!("mask:{}", cutoff)
                    } else {
                        "opaque".to_string()
                    }
                }
                _ => "opaque".to_string(),
            }
        })
    }).build()?;

    // mat_set_alpha_mode(path, mode) — mode: "opaque" | "blend" | "mask:<cutoff>"
    m.function("mat_set_alpha_mode", |path: String, mode: String| {
        MATERIAL_CACHE.with(|c| {
            if let Some(obj) = c.borrow_mut().get_mut(&path) {
                if let Some(map) = obj.as_object_mut() {
                    let val = if mode == "opaque" {
                        serde_json::Value::String("opaque".into())
                    } else if mode == "blend" {
                        serde_json::Value::String("blend".into())
                    } else if let Some(rest) = mode.strip_prefix("mask:") {
                        let cutoff: f64 = rest.parse().unwrap_or(0.5);
                        let mut m2 = serde_json::Map::new();
                        m2.insert("mask".into(), serde_json::json!(cutoff));
                        serde_json::Value::Object(m2)
                    } else {
                        serde_json::Value::String("opaque".into())
                    };
                    map.insert("alphaMode".into(), val);
                }
            }
        });
    }).build()?;

    // mat_uv_transform(path, slot) → [scale_x, scale_y, offset_x, offset_y]
    // slot: "albedo" | "normal" | etc.
    m.function("mat_uv_transform", |path: String, slot: String| -> Vec<f64> {
        MATERIAL_CACHE.with(|c| {
            c.borrow().get(&path)
                .and_then(|obj| obj.get("uvTransforms"))
                .and_then(|uv| uv.get(&slot))
                .map(|t| {
                    let sx = t.get("scale").and_then(|s| s.get(0)).and_then(|v| v.as_f64()).unwrap_or(1.0);
                    let sy = t.get("scale").and_then(|s| s.get(1)).and_then(|v| v.as_f64()).unwrap_or(1.0);
                    let ox = t.get("offset").and_then(|s| s.get(0)).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let oy = t.get("offset").and_then(|s| s.get(1)).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    vec![sx, sy, ox, oy]
                })
                .unwrap_or_else(|| vec![1.0, 1.0, 0.0, 0.0])
        })
    }).build()?;

    // mat_set_uv_transform(path, slot, vals) — vals = [scale_x, scale_y, offset_x, offset_y]
    m.function("mat_set_uv_transform", |path: String, slot: String, vals: Vec<f64>| {
        let sx = vals.get(0).copied().unwrap_or(1.0);
        let sy = vals.get(1).copied().unwrap_or(1.0);
        let ox = vals.get(2).copied().unwrap_or(0.0);
        let oy = vals.get(3).copied().unwrap_or(0.0);
        MATERIAL_CACHE.with(|c| {
            if let Some(obj) = c.borrow_mut().get_mut(&path) {
                if let Some(map) = obj.as_object_mut() {
                    let uv_map = map
                        .entry("uvTransforms")
                        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                    if let Some(slots) = uv_map.as_object_mut() {
                        slots.insert(slot, serde_json::json!({
                            "scale":  [sx, sy],
                            "offset": [ox, oy]
                        }));
                    }
                }
            }
        });
    }).build()?;

    // mat_apply_preset(path, preset) — overwrite multiple fields in one call.
    // preset: "metal" | "plastic" | "glass" | "matte" | "emissive"
    m.function("mat_apply_preset", |path: String, preset: Ref<str>| {
        MATERIAL_CACHE.with(|c| {
            if let Some(obj) = c.borrow_mut().get_mut(&path) {
                if let Some(map) = obj.as_object_mut() {
                    match preset.as_ref() {
                        "metal" => {
                            map.insert("roughness".into(), serde_json::json!(0.15));
                            map.insert("metalness".into(), serde_json::json!(1.0));
                            map.insert("alphaMode".into(), serde_json::json!("opaque"));
                        }
                        "plastic" => {
                            map.insert("roughness".into(), serde_json::json!(0.4));
                            map.insert("metalness".into(), serde_json::json!(0.0));
                            map.insert("alphaMode".into(), serde_json::json!("opaque"));
                        }
                        "glass" => {
                            map.insert("roughness".into(), serde_json::json!(0.05));
                            map.insert("metalness".into(), serde_json::json!(0.0));
                            map.insert("alphaMode".into(), serde_json::json!("blend"));
                            map.insert("color".into(), serde_json::json!([0.9, 0.95, 1.0, 0.3]));
                            map.insert("doubleSided".into(), serde_json::json!(true));
                        }
                        "matte" => {
                            map.insert("roughness".into(), serde_json::json!(0.9));
                            map.insert("metalness".into(), serde_json::json!(0.0));
                            map.insert("alphaMode".into(), serde_json::json!("opaque"));
                        }
                        "emissive" => {
                            map.insert("roughness".into(), serde_json::json!(0.5));
                            map.insert("metalness".into(), serde_json::json!(0.0));
                            map.insert("emissive".into(), serde_json::json!([1.0, 0.9, 0.3]));
                            map.insert("emissiveIntensity".into(), serde_json::json!(5.0));
                        }
                        _ => {}
                    }
                }
            }
        });
    }).build()?;

    // ── Asset reference query bindings ────────────────────────────────────────

    // asset_ref_count(path) → i64 — number of entity component fields referencing this asset.
    m.function("asset_ref_count", |path: Ref<str>| -> i64 {
        let path_str = path.as_ref();
        let guid = with_adb(|db| db.get_by_path(path_str).map(|r| r.guid.clone())).flatten();
        ASSET_REF_CACHE.with(|c| {
            let cache = c.borrow();
            if let Some(g) = &guid {
                if let Some((_, refs)) = cache.get(g.as_str()) {
                    return refs.len() as i64;
                }
            }
            let fallback = format!("path:{path_str}");
            cache.get(fallback.as_str()).map(|(_, refs)| refs.len() as i64).unwrap_or(0)
        })
    }).build()?;

    // asset_entity_refs(path) → Vec<Vec<String>> — list of [entity_id, name, component, field].
    m.function("asset_entity_refs", |path: Ref<str>| -> Vec<Vec<String>> {
        let path_str = path.as_ref();
        let guid = with_adb(|db| db.get_by_path(path_str).map(|r| r.guid.clone())).flatten();
        ASSET_REF_CACHE.with(|c| {
            let cache = c.borrow();
            if let Some(g) = &guid {
                if let Some((_, refs)) = cache.get(g.as_str()) {
                    return refs.clone();
                }
            }
            let fallback = format!("path:{path_str}");
            cache.get(fallback.as_str()).map(|(_, refs)| refs.clone()).unwrap_or_default()
        })
    }).build()?;

    // asset_guid_for_path(path) → String — stable GUID for the given asset path.
    m.function("asset_guid_for_path", |path: Ref<str>| -> String {
        with_adb(|db| db.get_by_path(path.as_ref()).map(|r| r.guid.clone()))
            .flatten()
            .unwrap_or_default()
    }).build()?;

    m.function("mat_save", |path: String| {
        MATERIAL_CACHE.with(|c| {
            if let Some(obj) = c.borrow().get(&path) {
                if let Ok(json) = serde_json::to_string_pretty(obj) {
                    PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                        entity:    fluxion_core::EntityId::INVALID,
                        component: "__write_material__".to_string(),
                        field:     path,
                        value:     ReflectValue::Str(json),
                    }));
                }
            }
        });
    }).build()?;

    Ok(m)
}

thread_local! {
    static MATERIAL_CACHE: RefCell<HashMap<String, serde_json::Value>> = RefCell::new(HashMap::new());
}
