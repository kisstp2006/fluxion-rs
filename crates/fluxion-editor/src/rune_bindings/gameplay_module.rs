// ============================================================
// gameplay_module.rs — Rune modules for gameplay scripts
//
// Provides Unity-style native modules used exclusively by
// per-entity gameplay scripts.  NOT installed into editor VM.
//
//   fluxion::native::transform  — Transform position/rotation/scale/helpers
//   fluxion::native::gameobject — find/spawn/destroy/name
//   fluxion::native::script     — hotreload_pending()
//
// The Rune prelude (auto-injected) wraps these into the
// user-facing OOP structs: Transform, GameObject, Vec3, etc.
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

// ── Script hot-reload pending flag ────────────────────────────────────────────

static SCRIPT_HOTRELOAD_PENDING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Set by the file watcher in main.rs when a `.rn` file changes.
pub fn set_script_hotreload_pending(v: bool) {
    SCRIPT_HOTRELOAD_PENDING.store(v, std::sync::atomic::Ordering::Relaxed);
}

/// Read (and consume) the flag — returns true once per hot-reload event.
pub fn take_script_hotreload_pending() -> bool {
    SCRIPT_HOTRELOAD_PENDING.swap(false, std::sync::atomic::Ordering::Relaxed)
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

// ── fluxion::native::transform module ────────────────────────────────────────
//
// All position/rotation/scale operations on entities.
// Euler angles are in DEGREES (matches Unity convention).
// Quaternion xyzw is also available for math-intensive scripts.

pub fn build_native_transform_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["native", "transform"])?;

    // ── self_id() → i64: entity ID of the current script owner ───────────────
    m.function("self_id", || -> i64 {
        SELF_ENTITY_ID.with(|c| c.get())
    }).build()?;

    // ── Position ─────────────────────────────────────────────────────────────

    // get_position(id) → [x, y, z] (world space)
    m.function("get_position", |id: i64| -> Vec<f64> {
        let Some(eid) = id_from_i64(id) else { return vec![0.0, 0.0, 0.0]; };
        with_world(|world| {
            world.get_component::<Transform>(eid)
                .map(|t| vec![t.position.x as f64, t.position.y as f64, t.position.z as f64])
                .unwrap_or_else(|| vec![0.0, 0.0, 0.0])
        }).unwrap_or_else(|| vec![0.0, 0.0, 0.0])
    }).build()?;

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

    // ── Rotation — Euler (degrees) ────────────────────────────────────────────

    // get_euler(id) → [x, y, z] DEGREES
    m.function("get_euler", |id: i64| -> Vec<f64> {
        let Some(eid) = id_from_i64(id) else { return vec![0.0, 0.0, 0.0]; };
        with_world(|world| {
            world.get_component::<Transform>(eid)
                .map(|t| {
                    let (x, y, z) = t.rotation.to_euler(EulerRot::XYZ);
                    let deg = std::f32::consts::PI / 180.0;
                    vec![(x / deg) as f64, (y / deg) as f64, (z / deg) as f64]
                })
                .unwrap_or_else(|| vec![0.0, 0.0, 0.0])
        }).unwrap_or_else(|| vec![0.0, 0.0, 0.0])
    }).build()?;

    // set_euler(id, [x, y, z]) — DEGREES XYZ
    m.function("set_euler", |id: i64, v: Vec<f64>| {
        let Some(eid) = id_from_i64(id) else { return; };
        with_world(|world| {
            if let Some(mut t) = world.get_component_mut::<Transform>(eid) {
                let deg = std::f32::consts::PI / 180.0;
                let rx = (v.first().copied().unwrap_or(0.0) as f32) * deg;
                let ry = (v.get(1).copied().unwrap_or(0.0) as f32) * deg;
                let rz = (v.get(2).copied().unwrap_or(0.0) as f32) * deg;
                t.rotation = Quat::from_euler(EulerRot::XYZ, rx, ry, rz);
                t.dirty = true;
            }
        });
    }).build()?;

    // rotate(id, dx, dy, dz) — add DEGREES delta to current Euler angles
    m.function("rotate", |id: i64, dx: f64, dy: f64, dz: f64| {
        let Some(eid) = id_from_i64(id) else { return; };
        with_world(|world| {
            if let Some(mut t) = world.get_component_mut::<Transform>(eid) {
                let (cx, cy, cz) = t.rotation.to_euler(EulerRot::XYZ);
                let deg = std::f32::consts::PI / 180.0;
                let nx = cx + (dx as f32) * deg;
                let ny = cy + (dy as f32) * deg;
                let nz = cz + (dz as f32) * deg;
                t.rotation = Quat::from_euler(EulerRot::XYZ, nx, ny, nz);
                t.dirty = true;
            }
        });
    }).build()?;

    // ── Rotation — Quaternion (xyzw) ──────────────────────────────────────────

    // get_rotation_quat(id) → [x, y, z, w]
    m.function("get_rotation_quat", |id: i64| -> Vec<f64> {
        let Some(eid) = id_from_i64(id) else { return vec![0.0, 0.0, 0.0, 1.0]; };
        with_world(|world| {
            world.get_component::<Transform>(eid)
                .map(|t| vec![t.rotation.x as f64, t.rotation.y as f64,
                               t.rotation.z as f64, t.rotation.w as f64])
                .unwrap_or_else(|| vec![0.0, 0.0, 0.0, 1.0])
        }).unwrap_or_else(|| vec![0.0, 0.0, 0.0, 1.0])
    }).build()?;

    // set_rotation_quat(id, [x, y, z, w])
    m.function("set_rotation_quat", |id: i64, v: Vec<f64>| {
        let Some(eid) = id_from_i64(id) else { return; };
        with_world(|world| {
            if let Some(mut t) = world.get_component_mut::<Transform>(eid) {
                t.rotation = Quat::from_xyzw(
                    v.first().copied().unwrap_or(0.0) as f32,
                    v.get(1).copied().unwrap_or(0.0) as f32,
                    v.get(2).copied().unwrap_or(0.0) as f32,
                    v.get(3).copied().unwrap_or(1.0) as f32,
                ).normalize();
                t.dirty = true;
            }
        });
    }).build()?;

    // quat_from_euler(x, y, z) → [qx, qy, qz, qw] — helper for Quaternion::euler
    m.function("quat_from_euler", |x: f64, y: f64, z: f64| -> Vec<f64> {
        let deg = std::f32::consts::PI / 180.0;
        let q = Quat::from_euler(EulerRot::XYZ, (x as f32) * deg, (y as f32) * deg, (z as f32) * deg);
        vec![q.x as f64, q.y as f64, q.z as f64, q.w as f64]
    }).build()?;

    // quat_to_euler(x, y, z, w) → [dx, dy, dz] degrees
    m.function("quat_to_euler", |x: f64, y: f64, z: f64, w: f64| -> Vec<f64> {
        let q = Quat::from_xyzw(x as f32, y as f32, z as f32, w as f32).normalize();
        let (ex, ey, ez) = q.to_euler(EulerRot::XYZ);
        let deg = std::f32::consts::PI / 180.0;
        vec![(ex / deg) as f64, (ey / deg) as f64, (ez / deg) as f64]
    }).build()?;

    // quat_mul(ax,ay,az,aw, bx,by,bz,bw) → [x,y,z,w] — compose two quaternions
    // Rune 0.14 arity limit = 5, so we pass as two Vec<f64>
    m.function("quat_mul", |a: Vec<f64>, b: Vec<f64>| -> Vec<f64> {
        let qa = Quat::from_xyzw(a[0] as f32, a[1] as f32, a[2] as f32, a[3] as f32).normalize();
        let qb = Quat::from_xyzw(b[0] as f32, b[1] as f32, b[2] as f32, b[3] as f32).normalize();
        let qr = qa * qb;
        vec![qr.x as f64, qr.y as f64, qr.z as f64, qr.w as f64]
    }).build()?;

    // quat_mul_vec3(quat: [x,y,z,w], v: [x,y,z]) → [x,y,z] — rotate a point
    m.function("quat_mul_vec3", |q: Vec<f64>, v: Vec<f64>| -> Vec<f64> {
        let quat = Quat::from_xyzw(q[0] as f32, q[1] as f32, q[2] as f32, q[3] as f32).normalize();
        let vec  = Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32);
        let out  = quat * vec;
        vec![out.x as f64, out.y as f64, out.z as f64]
    }).build()?;

    // ── Scale ─────────────────────────────────────────────────────────────────

    // get_scale(id) → [x, y, z]
    m.function("get_scale", |id: i64| -> Vec<f64> {
        let Some(eid) = id_from_i64(id) else { return vec![1.0, 1.0, 1.0]; };
        with_world(|world| {
            world.get_component::<Transform>(eid)
                .map(|t| vec![t.scale.x as f64, t.scale.y as f64, t.scale.z as f64])
                .unwrap_or_else(|| vec![1.0, 1.0, 1.0])
        }).unwrap_or_else(|| vec![1.0, 1.0, 1.0])
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

    // ── LookAt ────────────────────────────────────────────────────────────────

    // look_at(id, [tx, ty, tz]) — rotate to face world-space target position
    m.function("look_at", |id: i64, target: Vec<f64>| {
        let Some(eid) = id_from_i64(id) else { return; };
        with_world(|world| {
            if let Some(mut t) = world.get_component_mut::<Transform>(eid) {
                let target_pos = Vec3::new(
                    target.first().copied().unwrap_or(0.0) as f32,
                    target.get(1).copied().unwrap_or(0.0) as f32,
                    target.get(2).copied().unwrap_or(0.0) as f32,
                );
                let dir = (target_pos - t.position).normalize_or_zero();
                if dir.length_squared() > 1e-6 {
                    t.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, dir);
                    t.dirty = true;
                }
            }
        });
    }).build()?;

    // ── Hierarchy ────────────────────────────────────────────────────────────

    // get_parent(id) → i64 (-1 if none)
    // NOTE: Transform has no parent field in this engine; stub returns -1.
    m.function("get_parent", |_id: i64| -> i64 { -1 }).build()?;

    // set_parent(child_id, parent_id) — stub (no hierarchy system yet)
    m.function("set_parent", |_child_id: i64, _parent_id: i64| {}).build()?;

    // child_count(id) → i64 — stub (no hierarchy system yet)
    m.function("child_count", |_id: i64| -> i64 { 0 }).build()?;

    // get_child(parent_id, index) → i64 — stub
    m.function("get_child", |_parent_id: i64, _index: i64| -> i64 { -1 }).build()?;

    Ok(m)
}

// ── fluxion::native::gameobject module ───────────────────────────────────────

pub fn build_native_gameobject_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["native", "gameobject"])?;

    // self_id() → i64: entity ID of the current script owner
    m.function("self_id", || -> i64 {
        SELF_ENTITY_ID.with(|c| c.get())
    }).build()?;

    // get_name(id) → String
    m.function("get_name", |id: i64| -> String {
        let Some(eid) = id_from_i64(id) else { return String::new(); };
        with_world(|world| world.get_name(eid).to_string()).unwrap_or_default()
    }).build()?;

    // set_name(id, name)
    m.function("set_name", |id: i64, name: String| {
        let Some(eid) = id_from_i64(id) else { return; };
        super::world_module::with_world_mut(|world| { world.set_name(eid, &name); });
    }).build()?;

    // find_by_name(name) → i64 (-1 if not found)
    m.function("find_by_name", |name: String| -> i64 {
        with_world(|world| {
            world.all_entities()
                .find(|e| world.get_name(*e) == name)
                .map(|e| e.to_bits() as i64)
                .unwrap_or(-1)
        }).unwrap_or(-1)
    }).build()?;

    // destroy(id) — queues entity for destruction after frame
    m.function("destroy", |id: i64| {
        if id >= 0 {
            PENDING_DESTROYS.with(|v| v.borrow_mut().push(id as u64));
        }
    }).build()?;

    // spawn(name) → i64 — queues entity spawn; returns -1 (ID available next frame)
    m.function("spawn", |name: String| -> i64 {
        PENDING_SPAWNS.with(|v| v.borrow_mut().push(name));
        -1
    }).build()?;

    Ok(m)
}

// ── fluxion::native::script module ───────────────────────────────────────────

pub fn build_native_script_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["native", "script"])?;

    // hotreload_pending() → bool — true once after a .rn file change.
    m.function("hotreload_pending", || -> bool {
        take_script_hotreload_pending()
    }).build()?;

    // field(name) → f64: numeric field value
    m.function("field", |name: String| -> f64 {
        SCRIPT_FIELDS.with(|f| {
            let fields = f.borrow();
            fields.iter().find(|(n, _)| *n == name)
                .and_then(|(_, v)| v.as_f64())
                .unwrap_or(0.0)
        })
    }).build()?;

    // field_str(name) → String
    m.function("field_str", |name: String| -> String {
        SCRIPT_FIELDS.with(|f| {
            let fields = f.borrow();
            fields.iter().find(|(n, _)| *n == name)
                .and_then(|(_, v)| v.as_str().map(|s| s.to_string())
                    .or_else(|| v.as_f64().map(|n| n.to_string())))
                .unwrap_or_default()
        })
    }).build()?;

    // field_bool(name) → bool
    m.function("field_bool", |name: String| -> bool {
        SCRIPT_FIELDS.with(|f| {
            let fields = f.borrow();
            fields.iter().find(|(n, _)| *n == name)
                .and_then(|(_, v)| v.as_bool())
                .unwrap_or(false)
        })
    }).build()?;

    // set_field(name, value: f64)
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

    // set_field_str(name, value: String)
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

    // set_field_bool(name, value: bool)
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

    Ok(m)
}

// ── Factory function ──────────────────────────────────────────────────────────

/// Returns all gameplay-specific Rune modules.
/// Passed as `extra_modules_fn` to `RuneBehaviour::from_file_with_extra_modules`.
/// The embedded prelude (see `behaviour.rs`) wraps these into user-facing
/// OOP structs: Transform, GameObject, Vec3, Input, Time, Debug, Mathf, Key.
pub fn build_gameplay_modules() -> anyhow::Result<Vec<Module>> {
    Ok(vec![
        build_native_transform_module()?,
        build_native_gameobject_module()?,
        build_native_script_module()?,
        // Full input API backed by thread-local InputState set in tick_gameplay_scripts.
        super::input_module::build_input_module()
            .map_err(|e| anyhow::anyhow!("input module: {e}"))?,
    ])
}
