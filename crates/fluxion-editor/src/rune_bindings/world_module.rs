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
use std::ptr::NonNull;

use rune::Module;

use fluxion_core::{
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
}

/// A deferred field mutation queued by Rune, applied after the panel call.
pub struct PendingEdit {
    pub entity:    EntityId,
    pub component: String,
    pub field:     String,
    pub value:     ReflectValue,
}

// ── Public host API ────────────────────────────────────────────────────────────

/// Set world + registry pointers before a Rune panel call.
/// Also rebuilds the entity ID cache for O(1) lookups this frame.
/// # Safety: pointers must remain valid until `clear_world_context()` is called.
pub fn set_world_context(world: &ECSWorld, registry: &ComponentRegistry) {
    WORLD_PTR.with(|c| c.set(Some(NonNull::from(world))));
    REG_PTR  .with(|c| c.set(Some(NonNull::from(registry))));
    ENTITY_CACHE.with(|cache| {
        let mut map = cache.borrow_mut();
        map.clear();
        for e in world.all_entities() {
            map.insert(e.to_bits(), e);
        }
    });
}

/// Clear world + registry pointers after the Rune panel call.
pub fn clear_world_context() {
    WORLD_PTR.with(|c| c.set(None));
    REG_PTR  .with(|c| c.set(None));
    ENTITY_CACHE.with(|cache| cache.borrow_mut().clear());
}

/// Drain queued mutations for the host to apply with &mut ECSWorld.
pub fn drain_pending_edits() -> Vec<PendingEdit> {
    PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

/// Append a log line from Rust host code.
pub fn push_log(line: String) {
    LOG_LINES.with(|l| l.borrow_mut().push(line));
    LOG_GENERATION.with(|g| g.set(g.get().wrapping_add(1)));
}

/// Get the currently selected entity (may be None).
pub fn get_selected_id() -> Option<EntityId> {
    SELECTED.with(|s| *s.borrow())
}

// ── Internal helpers ─────────────────────────────────────────────────────────

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

    m.function("fields", |id: i64, component: String| -> Vec<String> {
        let _ = id;
        with_ctx(|_, registry| {
            registry.component_fields(&component)
                .map(|fields| fields.iter().map(|f| f.name.to_string()).collect())
                .unwrap_or_default()
        }).unwrap_or_default()
    }).build()?;

    m.function("field_type", |id: i64, component: String, field: String| -> String {
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
                }.to_string())
                .unwrap_or_default()
        }).unwrap_or_default()
    }).build()?;

    m.function("field_range", |id: i64, component: String, field: String| -> Vec<f64> {
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

    m.function("field_readonly", |id: i64, component: String, field: String| -> bool {
        let _ = id;
        with_ctx(|_, registry| {
            get_descriptor(registry, &component, &field)
                .map(|d| d.read_only)
                .unwrap_or(false)
        }).unwrap_or(false)
    }).build()?;

    // ── Typed getters ─────────────────────────────────────────────────────────

    m.function("get_f32", |id: i64, component: String, field: String| -> f64 {
        with_ctx(|world, registry| {
            let entity = id_to_entity(world, id)?;
            let reflect = registry.get_reflect(&component, world, entity)?;
            match reflect.get_field(&field)? {
                ReflectValue::F32(v) => Some(v as f64),
                _ => None,
            }
        }).flatten().unwrap_or(0.0)
    }).build()?;

    m.function("get_bool", |id: i64, component: String, field: String| -> bool {
        with_ctx(|world, registry| {
            let entity = id_to_entity(world, id)?;
            let reflect = registry.get_reflect(&component, world, entity)?;
            match reflect.get_field(&field)? {
                ReflectValue::Bool(v) => Some(v),
                _ => None,
            }
        }).flatten().unwrap_or(false)
    }).build()?;

    m.function("get_str", |id: i64, component: String, field: String| -> String {
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

    m.function("get_vec3", |id: i64, component: String, field: String| -> Vec<f64> {
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

    m.function("get_quat", |id: i64, component: String, field: String| -> Vec<f64> {
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

    m.function("get_color3", |id: i64, component: String, field: String| -> Vec<f64> {
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

    m.function("get_color4", |id: i64, component: String, field: String| -> Vec<f64> {
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

    m.function("get_u32", |id: i64, component: String, field: String| -> i64 {
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

    m.function("create_entity", |name: String| {
        PENDING.with(|p| p.borrow_mut().push(PendingEdit {
            entity:    fluxion_core::EntityId::INVALID,
            component: "__spawn__".to_string(),
            field:     name,
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

    // ── Console log access ────────────────────────────────────────────────────

    m.function("log_lines", || -> Vec<String> {
        LOG_LINES.with(|l| l.borrow().clone())
    }).build()?;

    m.function("log", |line: String| {
        push_log(line);
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

    m.function("rename_entity", |id: i64, name: String| {
        with_ctx(|world, _| {
            if let Some(entity) = id_to_entity(world, id) {
                PENDING.with(|p| p.borrow_mut().push(PendingEdit {
                    entity,
                    component: "__rename__".to_string(),
                    field:     name,
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

    // ── Euler angle helpers (degrees) ─────────────────────────────────────────

    m.function("get_euler", |id: i64, component: String, field: String| -> Vec<f64> {
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

    Ok(m)
}
