// ============================================================
// gameplay_module.rs — Rune modules for gameplay scripts
//
// Provides two Rune modules used exclusively by per-entity
// gameplay scripts (ScriptBehaviour).  These are NOT installed
// into the editor VM (no egui dependency).
//
//   fluxion::script  — identity of the current script owner
//   fluxion::entity  — transform get/set, find, spawn, destroy
//
// Usage in a gameplay script (assets/scripts/spinner.rn):
//
//   pub fn update(dt) {
//       let id  = fluxion::script::self_entity();
//       let rot = fluxion::entity::get_rotation_euler(id);
//       fluxion::entity::set_rotation_euler(id, [rot[0], rot[1] + dt, rot[2]]);
//   }
// ============================================================

use std::cell::Cell;
use rune::Module;
use fluxion_core::transform::Transform;
use glam::{EulerRot, Quat, Vec3};

use super::world_module::with_world;

// ── Thread-local: current script owner entity ─────────────────────────────────

thread_local! {
    /// Entity ID (as i64) of the gameplay script currently being ticked.
    /// Set by host.rs before each `RuneBehaviour::tick()`, cleared after.
    static SELF_ENTITY_ID: Cell<i64> = Cell::new(-1);
    /// Name of the script currently being ticked (e.g. "Spinner").
    static SELF_SCRIPT_NAME: RefCell<String> = RefCell::new(String::new());
}

/// Called by host.rs before ticking a gameplay script.
pub fn set_self_entity(id: i64) {
    SELF_ENTITY_ID.with(|c| c.set(id));
}

/// Called by host.rs after ticking a gameplay script.
pub fn clear_self_entity() {
    SELF_ENTITY_ID.with(|c| c.set(-1));
}

/// Called by host.rs before ticking, to set the active script name.
pub fn set_self_script(name: &str) {
    SELF_SCRIPT_NAME.with(|c| *c.borrow_mut() = name.to_string());
}

/// Called by host.rs after ticking.
pub fn clear_self_script() {
    SELF_SCRIPT_NAME.with(|c| c.borrow_mut().clear());
}

// ── Script error store ────────────────────────────────────────────────────────

use std::collections::HashMap;
use std::cell::RefCell;
use std::sync::Mutex;

thread_local! {
    /// Maps (entity_bits, script_name) → last compile/runtime error string.
    static SCRIPT_ERRORS: RefCell<HashMap<(u64, String), String>> = RefCell::new(HashMap::new());
}

// ── Compile summary ───────────────────────────────────────────────────────────

/// (total_scripts, error_count).  `(-1, 0)` = not compiled yet.
static COMPILE_SUMMARY: Mutex<(i64, i64)> = Mutex::new((-1, 0));

/// Set after `rebuild_gameplay_scripts` finishes.
pub fn set_compile_summary(total: usize, errors: usize) {
    if let Ok(mut g) = COMPILE_SUMMARY.lock() {
        *g = (total as i64, errors as i64);
    }
}

/// Returns `(-1, 0)` if scripts have not been compiled yet this session.
pub fn get_compile_summary() -> (i64, i64) {
    COMPILE_SUMMARY.lock().map(|g| *g).unwrap_or((-1, 0))
}

/// Store a compile error for a given (entity, script_name) pair.
pub fn set_script_error(entity_bits: u64, script_name: impl Into<String>, msg: impl Into<String>) {
    SCRIPT_ERRORS.with(|m| { m.borrow_mut().insert((entity_bits, script_name.into()), msg.into()); });
}

/// Clear the error for a given (entity, script_name) pair.
pub fn clear_script_error(entity_bits: u64, script_name: &str) {
    SCRIPT_ERRORS.with(|m| { m.borrow_mut().remove(&(entity_bits, script_name.to_string())); });
}

/// Get the last error string for (entity, script_name); empty = no error.
pub fn get_script_error(entity_bits: u64, script_name: &str) -> String {
    SCRIPT_ERRORS.with(|m| m.borrow().get(&(entity_bits, script_name.to_string())).cloned().unwrap_or_default())
}

// ── Pending spawns/destroys ───────────────────────────────────────────────────

thread_local! {
    static PENDING_DESTROYS: RefCell<Vec<u64>> = RefCell::new(Vec::new());
    static PENDING_SPAWNS:   RefCell<Vec<String>> = RefCell::new(Vec::new());
}

/// Drain entities queued for destruction by gameplay scripts.
pub fn drain_pending_destroys() -> Vec<u64> {
    PENDING_DESTROYS.with(|v| std::mem::take(&mut *v.borrow_mut()))
}

/// Drain entity names queued for spawning by gameplay scripts.
pub fn drain_pending_spawns() -> Vec<String> {
    PENDING_SPAWNS.with(|v| std::mem::take(&mut *v.borrow_mut()))
}

// ── Field declarations (declare_field API) ───────────────────────────────────
//
// Gameplay scripts call `fluxion::script::declare_field(name, type, hint, min, max)`
// during tick() to tell the inspector how to render each field.  Declarations are
// stored per script name in a global map and are persistent across frames.

static FIELD_DECL_STORE: std::sync::OnceLock<Mutex<HashMap<String, Vec<Vec<String>>>>> = std::sync::OnceLock::new();

fn field_decl_store() -> &'static Mutex<HashMap<String, Vec<Vec<String>>>> {
    FIELD_DECL_STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Called by the `declare_field` Rune binding to register field metadata.
fn store_field_decl(script_name: &str, decl: Vec<String>) {
    if let Ok(mut map) = field_decl_store().lock() {
        let entry = map.entry(script_name.to_string()).or_default();
        // Replace existing declaration with the same field name, or append.
        if let Some(existing) = entry.iter_mut().find(|d| d.first().map(|n| n == &decl[0]).unwrap_or(false)) {
            *existing = decl;
        } else {
            entry.push(decl);
        }
    }
}

/// Returns declared field metadata for a script by name.
/// Each inner Vec is [name, type_str, hint, min_str, max_str].
pub fn get_field_decls(script_name: &str) -> Vec<Vec<String>> {
    field_decl_store().lock()
        .map(|map| map.get(script_name).cloned().unwrap_or_default())
        .unwrap_or_default()
}

// ── Script fields (injected around each tick) ────────────────────────────────

thread_local! {
    /// Current script's mutable field store. Loaded before tick, drained after.
    static SCRIPT_FIELDS: RefCell<Vec<(String, serde_json::Value)>> = RefCell::new(Vec::new());
}

/// Called by host.rs before each `RuneBehaviour::tick()` to inject field values.
pub fn set_script_fields(fields: Vec<(String, serde_json::Value)>) {
    SCRIPT_FIELDS.with(|f| *f.borrow_mut() = fields);
}

/// Called by host.rs after each `RuneBehaviour::tick()` to drain the (possibly mutated) fields.
pub fn drain_script_fields() -> Vec<(String, serde_json::Value)> {
    SCRIPT_FIELDS.with(|f| std::mem::take(&mut *f.borrow_mut()))
}

// ── Helper: entity ID ↔ bits ──────────────────────────────────────────────────

fn id_from_i64(raw: i64) -> Option<fluxion_core::EntityId> {
    if raw < 0 { return None; }
    with_world(|world| {
        world.all_entities().find(|e| e.to_bits() == raw as u64)
    }).flatten()
}

// ── fluxion::script module ────────────────────────────────────────────────────

pub fn build_script_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["script"])?;

    // self_entity() → i64: entity ID of the script's owner (-1 = none)
    m.function("self_entity", || -> i64 {
        SELF_ENTITY_ID.with(|c| c.get())
    }).build()?;

    // self_name() → String: display name of the owner entity
    m.function("self_name", || -> String {
        let id = SELF_ENTITY_ID.with(|c| c.get());
        if id < 0 { return String::new(); }
        with_world(|world| {
            world.all_entities()
                .find(|e| e.to_bits() == id as u64)
                .map(|e| world.get_name(e).to_string())
                .unwrap_or_default()
        }).unwrap_or_default()
    }).build()?;

    // field(name) → f64: numeric field value (0.0 if missing or non-numeric).
    m.function("field", |name: String| -> f64 {
        SCRIPT_FIELDS.with(|f| {
            let fields = f.borrow();
            fields.iter().find(|(n, _)| *n == name)
                .and_then(|(_, v)| v.as_f64())
                .unwrap_or(0.0)
        })
    }).build()?;

    // field_str(name) → String: string field value ("" if missing or non-string).
    m.function("field_str", |name: String| -> String {
        SCRIPT_FIELDS.with(|f| {
            let fields = f.borrow();
            fields.iter().find(|(n, _)| *n == name)
                .and_then(|(_, v)| v.as_str().map(|s| s.to_string())
                    .or_else(|| v.as_f64().map(|n| n.to_string())))
                .unwrap_or_default()
        })
    }).build()?;

    // field_bool(name) → bool: boolean field value (false if missing or non-bool).
    m.function("field_bool", |name: String| -> bool {
        SCRIPT_FIELDS.with(|f| {
            let fields = f.borrow();
            fields.iter().find(|(n, _)| *n == name)
                .and_then(|(_, v)| v.as_bool())
                .unwrap_or(false)
        })
    }).build()?;

    // set_field(name, value: f64) — write a numeric field back.
    m.function("set_field", |name: String, value: f64| {
        SCRIPT_FIELDS.with(|f| {
            let mut fields = f.borrow_mut();
            let json_val = serde_json::json!(value);
            if let Some(entry) = fields.iter_mut().find(|(n, _)| *n == name) {
                entry.1 = json_val;
            } else {
                fields.push((name, json_val));
            }
        });
    }).build()?;

    // set_field_str(name, value: String) — write a string field back.
    m.function("set_field_str", |name: String, value: String| {
        SCRIPT_FIELDS.with(|f| {
            let mut fields = f.borrow_mut();
            let json_val = serde_json::Value::String(value);
            if let Some(entry) = fields.iter_mut().find(|(n, _)| *n == name) {
                entry.1 = json_val;
            } else {
                fields.push((name, json_val));
            }
        });
    }).build()?;

    // set_field_bool(name, value: bool) — write a boolean field back.
    m.function("set_field_bool", |name: String, value: bool| {
        SCRIPT_FIELDS.with(|f| {
            let mut fields = f.borrow_mut();
            let json_val = serde_json::Value::Bool(value);
            if let Some(entry) = fields.iter_mut().find(|(n, _)| *n == name) {
                entry.1 = json_val;
            } else {
                fields.push((name, json_val));
            }
        });
    }).build()?;

    // declare_field(name, type_str, hint, min, max)
    // Registers inspector metadata for a script field.
    // type_str: "f32"|"bool"|"str"|"material"|"mesh"|"audio"|"scene"|"entity_ref"
    // hint:     ""|"slider"|"uniform_scale"
    // min/max:  numeric range (0,0 = no range)
    // Example:  fluxion::script::declare_field("speed", "f32", "slider", 0.0, 100.0)
    m.function("declare_field", |name: String, type_str: String, hint: String, min: f64, max: f64| {
        let script_name = SELF_SCRIPT_NAME.with(|s| s.borrow().clone());
        if !script_name.is_empty() {
            store_field_decl(&script_name, vec![
                name,
                type_str,
                hint,
                min.to_string(),
                max.to_string(),
            ]);
        }
    }).build()?;

    Ok(m)
}

// ── fluxion::entity module ────────────────────────────────────────────────────

pub fn build_entity_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["entity"])?;

    // ── Transform reads ──────────────────────────────────────────────────────

    // get_position(id) → [x, y, z]
    m.function("get_position", |id: i64| -> Vec<f64> {
        let Some(eid) = id_from_i64(id) else { return vec![0.0, 0.0, 0.0]; };
        with_world(|world| {
            world.get_component::<Transform>(eid)
                .map(|t| vec![t.position.x as f64, t.position.y as f64, t.position.z as f64])
                .unwrap_or_else(|| vec![0.0, 0.0, 0.0])
        }).unwrap_or_else(|| vec![0.0, 0.0, 0.0])
    }).build()?;

    // get_rotation_euler(id) → [rx, ry, rz] (radians, XYZ order)
    m.function("get_rotation_euler", |id: i64| -> Vec<f64> {
        let Some(eid) = id_from_i64(id) else { return vec![0.0, 0.0, 0.0]; };
        with_world(|world| {
            world.get_component::<Transform>(eid)
                .map(|t| {
                    let (x, y, z) = t.rotation.to_euler(EulerRot::XYZ);
                    vec![x as f64, y as f64, z as f64]
                })
                .unwrap_or_else(|| vec![0.0, 0.0, 0.0])
        }).unwrap_or_else(|| vec![0.0, 0.0, 0.0])
    }).build()?;

    // get_scale(id) → [x, y, z]
    m.function("get_scale", |id: i64| -> Vec<f64> {
        let Some(eid) = id_from_i64(id) else { return vec![1.0, 1.0, 1.0]; };
        with_world(|world| {
            world.get_component::<Transform>(eid)
                .map(|t| vec![t.scale.x as f64, t.scale.y as f64, t.scale.z as f64])
                .unwrap_or_else(|| vec![1.0, 1.0, 1.0])
        }).unwrap_or_else(|| vec![1.0, 1.0, 1.0])
    }).build()?;

    // ── Transform writes ─────────────────────────────────────────────────────

    // set_position(id, [x, y, z])
    m.function("set_position", |id: i64, v: Vec<f64>| {
        let Some(eid) = id_from_i64(id) else { return; };
        with_world(|world| {
            if let Some(mut t) = world.get_component_mut::<Transform>(eid) {
                t.position = Vec3::new(
                    v.first().copied().unwrap_or(0.0) as f32,
                    v.get(1).copied().unwrap_or(0.0) as f32,
                    v.get(2).copied().unwrap_or(0.0) as f32,
                );
                t.dirty = true;
            }
        });
    }).build()?;

    // set_rotation_euler(id, [rx, ry, rz]) — radians, XYZ
    m.function("set_rotation_euler", |id: i64, v: Vec<f64>| {
        let Some(eid) = id_from_i64(id) else { return; };
        with_world(|world| {
            if let Some(mut t) = world.get_component_mut::<Transform>(eid) {
                let rx = v.first().copied().unwrap_or(0.0) as f32;
                let ry = v.get(1).copied().unwrap_or(0.0) as f32;
                let rz = v.get(2).copied().unwrap_or(0.0) as f32;
                t.rotation = Quat::from_euler(EulerRot::XYZ, rx, ry, rz);
                t.dirty = true;
            }
        });
    }).build()?;

    // set_scale(id, [x, y, z])
    m.function("set_scale", |id: i64, v: Vec<f64>| {
        let Some(eid) = id_from_i64(id) else { return; };
        with_world(|world| {
            if let Some(mut t) = world.get_component_mut::<Transform>(eid) {
                t.scale = Vec3::new(
                    v.first().copied().unwrap_or(1.0) as f32,
                    v.get(1).copied().unwrap_or(1.0) as f32,
                    v.get(2).copied().unwrap_or(1.0) as f32,
                );
                t.dirty = true;
            }
        });
    }).build()?;

    // translate(id, [dx, dy, dz]) — move relative to current position
    m.function("translate", |id: i64, v: Vec<f64>| {
        let Some(eid) = id_from_i64(id) else { return; };
        with_world(|world| {
            if let Some(mut t) = world.get_component_mut::<Transform>(eid) {
                t.position += Vec3::new(
                    v.first().copied().unwrap_or(0.0) as f32,
                    v.get(1).copied().unwrap_or(0.0) as f32,
                    v.get(2).copied().unwrap_or(0.0) as f32,
                );
                t.dirty = true;
            }
        });
    }).build()?;

    // ── World queries ────────────────────────────────────────────────────────

    // find(name) → i64 — find entity by display name, -1 if not found
    m.function("find", |name: String| -> i64 {
        with_world(|world| {
            world.all_entities()
                .find(|e| world.get_name(*e) == name)
                .map(|e| e.to_bits() as i64)
                .unwrap_or(-1)
        }).unwrap_or(-1)
    }).build()?;

    // name(id) → String — get entity display name
    m.function("name", |id: i64| -> String {
        let Some(eid) = id_from_i64(id) else { return String::new(); };
        with_world(|world| {
            world.get_name(eid).to_string()
        }).unwrap_or_default()
    }).build()?;

    // ── Entity lifecycle ─────────────────────────────────────────────────────

    // destroy(id) — queues entity for destruction after frame
    m.function("destroy", |id: i64| {
        if id >= 0 {
            PENDING_DESTROYS.with(|v| v.borrow_mut().push(id as u64));
        }
    }).build()?;

    // spawn(name) → i64 — queues entity spawn, returns -1 (ID available next frame)
    // Note: returns -1 because spawn is deferred; query by name next frame.
    m.function("spawn", |name: String| -> i64 {
        PENDING_SPAWNS.with(|v| v.borrow_mut().push(name));
        -1
    }).build()?;

    Ok(m)
}

// ── Factory function ──────────────────────────────────────────────────────────

/// Returns all gameplay-specific Rune modules.
/// Pass this as the `extra_modules_fn` argument to
/// `RuneBehaviour::from_file_with_extra_modules`.
pub fn build_gameplay_modules() -> anyhow::Result<Vec<Module>> {
    Ok(vec![
        build_script_module()?,
        build_entity_module()?,
    ])
}
