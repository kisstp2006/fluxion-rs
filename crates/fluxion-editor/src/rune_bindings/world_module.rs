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

use rune::{Module, runtime::Ref};

use fluxion_core::{
    AssetDatabase,
    ComponentRegistry, ECSWorld, EntityId,
    reflect::{FieldDescriptor, ReflectFieldType, ReflectValue},
};
use glam::EulerRot;

// ── Thread-local context ──────────────────────────────────────────────────────

thread_local! {
    static WORLD_PTR:    Cell<Option<NonNull<ECSWorld>>>          = Cell::new(None);
    static REG_PTR:      Cell<Option<NonNull<ComponentRegistry>>> = Cell::new(None);
    static SELECTED:     RefCell<Option<EntityId>>               = RefCell::new(None);
    static PENDING:      RefCell<Vec<PendingEdit>>               = RefCell::new(Vec::new());
    static LOG_LINES:    RefCell<Vec<String>>                    = RefCell::new(Vec::new());
    /// Per-frame cache: entity bits → EntityId. Rebuilt each frame in set_world_context.
    static ENTITY_CACHE: RefCell<HashMap<u64, EntityId>>         = RefCell::new(HashMap::new());
    /// Monotonic counter incremented on every push_log / clear_log call.
    /// Rune panels compare against their last-seen value to skip redundant clones.
    static LOG_GENERATION: Cell<u64> = Cell::new(0);
    /// Project root directory path — set once at editor startup.
    static PROJECT_ROOT: RefCell<PathBuf> = RefCell::new(PathBuf::new());
    /// (can_undo, can_redo) — pushed each frame by EditorHost.
    static UNDO_STATE: Cell<(bool, bool)> = Cell::new((false, false));
    /// Last frame delta time in milliseconds.
    static FRAME_TIME_MS: Cell<f64> = Cell::new(0.0);
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
    /// Project name for display in toolbar.
    static PROJECT_NAME: RefCell<String> = RefCell::new(String::new());
    /// Scene name for display in toolbar.
    static SCENE_NAME: RefCell<String> = RefCell::new(String::new());
    /// Signals queued by Rune scripts for main.rs to consume.
    static ACTION_SIGNALS: RefCell<Vec<String>> = RefCell::new(Vec::new());

    // ── Editor camera state (persisted between frames, mutated by editor_camera.rn) ──
    static EDITOR_CAM_POS:    RefCell<[f64; 3]> = RefCell::new([0.0, 2.0, 8.0]);
    static EDITOR_CAM_YAW:    Cell<f64>         = Cell::new(0.0);
    static EDITOR_CAM_PITCH:  Cell<f64>         = Cell::new(-0.15);
    static EDITOR_CAM_TARGET: RefCell<[f64; 3]> = RefCell::new([0.0, 0.0, 0.0]);
    static EDITOR_CAM_SPEED:  Cell<f64>         = Cell::new(5.0);
    /// True when the editor camera has been mutated this frame (main.rs reads this to push to Transform).
    static EDITOR_CAM_DIRTY:  Cell<bool>        = Cell::new(false);
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
pub fn clear_world_context() {
    WORLD_PTR   .with(|c| c.set(None));
    REG_PTR     .with(|c| c.set(None));
    ENTITY_CACHE.with(|cache| cache.borrow_mut().clear());
}

/// Drain queued mutations for the host to apply with &mut ECSWorld.
pub fn drain_pending_edits() -> Vec<PendingEdit> {
    PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

/// Append a log line from Rust host code.
/// Caps the log at 10 000 entries; drains the oldest 1 000 when exceeded.
pub fn push_log(line: String) {
    LOG_LINES.with(|l| {
        let mut v = l.borrow_mut();
        if v.len() >= 10_000 {
            v.drain(..1_000);
        }
        v.push(line);
    });
    LOG_GENERATION.with(|g| g.set(g.get().wrapping_add(1)));
}

/// Get the currently selected entity (may be None).
pub fn get_selected_id() -> Option<EntityId> {
    SELECTED.with(|s| *s.borrow())
}

/// Set the project root path so Rune scripts can enumerate assets.
pub fn set_project_root(root: &std::path::Path) {
    PROJECT_ROOT.with(|p| *p.borrow_mut() = root.to_path_buf());
}

/// Update undo/redo state so Rune scripts can query it.
pub fn set_undo_state(can_undo: bool, can_redo: bool) {
    UNDO_STATE.with(|c| c.set((can_undo, can_redo)));
}

/// Push the last frame delta time (milliseconds) for the debugger panel.
pub fn set_frame_time(ms: f64) {
    FRAME_TIME_MS.with(|c| c.set(ms));
}

pub fn set_time_elapsed(secs: f64) {
    TIME_ELAPSED.with(|c| c.set(secs));
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

/// Read the transform tool as set by Rune.
pub fn get_transform_tool_str() -> String {
    TRANSFORM_TOOL.with(|c| c.borrow().clone())
}

// ── Editor camera host API ────────────────────────────────────────────────────

/// Read editor camera position [x,y,z] (for main.rs to push to renderer).
pub fn get_editor_cam_pos() -> [f64; 3] {
    EDITOR_CAM_POS.with(|c| *c.borrow())
}

/// Read editor camera yaw (radians).
pub fn get_editor_cam_yaw() -> f64 {
    EDITOR_CAM_YAW.with(|c| c.get())
}

/// Read editor camera pitch (radians).
pub fn get_editor_cam_pitch() -> f64 {
    EDITOR_CAM_PITCH.with(|c| c.get())
}

/// Initialize editor camera position from the active Camera entity transform (called once at startup).
pub fn init_editor_cam(pos: [f64; 3], yaw: f64, pitch: f64) {
    EDITOR_CAM_POS  .with(|c| *c.borrow_mut() = pos);
    EDITOR_CAM_YAW  .with(|c| c.set(yaw));
    EDITOR_CAM_PITCH.with(|c| c.set(pitch));
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
    let bits = id as u64;
    ENTITY_CACHE.with(|cache| cache.borrow().get(&bits).copied())
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
                    .filter(|&&name| registry.has_reflect(name))
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
                .map(|d| match d.field_type {
                    ReflectFieldType::F32     => "f32",
                    ReflectFieldType::Vec3    => "vec3",
                    ReflectFieldType::Quat    => "quat",
                    ReflectFieldType::Color3  => "color3",
                    ReflectFieldType::Color4  => "color4",
                    ReflectFieldType::Bool    => "bool",
                    ReflectFieldType::U32     => "u32",
                    ReflectFieldType::U8      => "u8",
                    ReflectFieldType::USize   => "usize",
                    ReflectFieldType::Str     => "str",
                    ReflectFieldType::OptionStr => "option_str",
                    ReflectFieldType::Enum    => "enum",
                    ReflectFieldType::Texture => "texture",
                    ReflectFieldType::I32     => "i32",
                    ReflectFieldType::Vec2    => "vec2",
                }.to_string())
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
                ReflectFieldType::OptionStr => ReflectValue::OptionStr(Some(val)),
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

    // ── Component add / remove ────────────────────────────────────────────────

    m.function("available_components", || -> Vec<String> {
        with_ctx(|_, registry| {
            registry.reflected_type_names()
                .into_iter()
                .map(|s| s.to_string())
                .collect()
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

    m.function("log_lines", || -> Vec<String> {
        LOG_LINES.with(|l| l.borrow().clone())
    }).build()?;

    m.function("log", |line: Ref<str>| {
        push_log(line.as_ref().to_string());
    }).build()?;

    m.function("clear_log", || {
        LOG_LINES.with(|l| l.borrow_mut().clear());
        LOG_GENERATION.with(|g| g.set(g.get().wrapping_add(1)));
    }).build()?;

    m.function("log_generation", || -> i64 {
        LOG_GENERATION.with(|g| g.get() as i64)
    }).build()?;

    m.function("log_lines_tail", |n: i64| -> Vec<String> {
        LOG_LINES.with(|l| {
            let lines = l.borrow();
            let n = (n.max(0) as usize).min(lines.len());
            lines[lines.len() - n..].to_vec()
        })
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
        LOG_LINES.with(|l| {
            l.borrow().iter().filter(|s| s.contains("[ERROR]")).count() as i64
        })
    }).build()?;

    // ── Undo/redo state ───────────────────────────────────────────────────────

    m.function("can_undo", || -> bool {
        UNDO_STATE.with(|c| c.get().0)
    }).build()?;

    m.function("can_redo", || -> bool {
        UNDO_STATE.with(|c| c.get().1)
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

    // ── Editor camera state (read/write by editor_camera.rn) ─────────────────

    m.function("get_editor_cam_pos", || -> Vec<f64> {
        EDITOR_CAM_POS.with(|c| c.borrow().to_vec())
    }).build()?;

    m.function("set_editor_cam_pos", |vals: Vec<f64>| {
        if vals.len() >= 3 {
            EDITOR_CAM_POS.with(|c| *c.borrow_mut() = [vals[0], vals[1], vals[2]]);
            EDITOR_CAM_DIRTY.with(|c| c.set(true));
        }
    }).build()?;

    m.function("get_editor_cam_yaw", || -> f64 {
        EDITOR_CAM_YAW.with(|c| c.get())
    }).build()?;

    m.function("set_editor_cam_yaw", |v: f64| {
        EDITOR_CAM_YAW.with(|c| c.set(v));
        EDITOR_CAM_DIRTY.with(|c| c.set(true));
    }).build()?;

    m.function("get_editor_cam_pitch", || -> f64 {
        EDITOR_CAM_PITCH.with(|c| c.get())
    }).build()?;

    m.function("set_editor_cam_pitch", |v: f64| {
        EDITOR_CAM_PITCH.with(|c| c.set(v));
        EDITOR_CAM_DIRTY.with(|c| c.set(true));
    }).build()?;

    m.function("get_editor_cam_target", || -> Vec<f64> {
        EDITOR_CAM_TARGET.with(|c| c.borrow().to_vec())
    }).build()?;

    m.function("set_editor_cam_target", |vals: Vec<f64>| {
        if vals.len() >= 3 {
            EDITOR_CAM_TARGET.with(|c| *c.borrow_mut() = [vals[0], vals[1], vals[2]]);
            EDITOR_CAM_DIRTY.with(|c| c.set(true));
        }
    }).build()?;

    m.function("get_editor_cam_speed", || -> f64 {
        EDITOR_CAM_SPEED.with(|c| c.get())
    }).build()?;

    m.function("set_editor_cam_speed", |v: f64| {
        EDITOR_CAM_SPEED.with(|c| c.set(v.max(0.1)));
    }).build()?;

    Ok(m)
}
