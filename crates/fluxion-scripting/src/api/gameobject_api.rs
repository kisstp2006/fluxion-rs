// ============================================================
// fluxion-scripting — GameObject / World API
//
// Unity-compatible object management from scripts.
//
// Unity equivalents:
//   GameObject.Find(name)
//   GameObject.FindWithTag(tag)
//   GameObject.Instantiate(prefab)
//   GameObject.Destroy(obj, delay?)
//   Object.DontDestroyOnLoad(obj)
//   gameObject.SetActive(bool)
//   gameObject.GetComponent<T>()
//   gameObject.AddComponent<T>()
// ============================================================

use std::sync::Mutex;
use fluxion_core::ReflectValue;
use crate::binding_registry::{BindingEntry, ParamMeta, ScriptBindingRegistry, ScriptType};

// ── Queued world-mutation commands ─────────────────────────────────────────────
// Scripts call these; the ECS world is mutated by Rust at end-of-frame.

#[derive(Debug, Clone)]
pub enum WorldCommand {
    Destroy       { entity_name: String, delay: f32 },
    SetActive     { entity_name: String, active: bool },
    Instantiate   { prefab_path: String, parent: Option<String>, position: Option<[f32;3]>, rotation: Option<[f32;4]> },
    AddComponent  { entity_name: String, component_type: String, data: String },
    RemoveComponent { entity_name: String, component_type: String },
    SetTag        { entity_name: String, tag: String },
    SetName       { entity_name: String, new_name: String },
    SetParent     { entity_name: String, parent_name: Option<String> },
    LoadScene     { scene_path: String, additive: bool },
}

lazy_static::lazy_static! {
    pub static ref WORLD_COMMANDS: Mutex<Vec<WorldCommand>> = Mutex::new(Vec::new());
}

pub fn drain_commands() -> Vec<WorldCommand> {
    WORLD_COMMANDS.lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}

fn push(cmd: WorldCommand) {
    if let Ok(mut q) = WORLD_COMMANDS.lock() { q.push(cmd); }
}

fn as_str(v: &ReflectValue) -> String {
    match v { ReflectValue::Str(s) => s.clone(), _ => String::new() }
}
fn as_f32(v: &ReflectValue) -> f32 {
    match v { ReflectValue::F32(f) => *f, ReflectValue::U32(n) => *n as f32, _ => 0.0 }
}

pub fn register(reg: &mut ScriptBindingRegistry) {
    // ── GameObject static methods ─────────────────────────────────────────────

    reg.register("GameObject", BindingEntry::new(
        "Find",
        "Finds a GameObject by name. Returns null if not found.",
        vec![ParamMeta::new("name", ScriptType::String)],
        Some(ScriptType::Object),
        |_args| {
            // Actual lookup is JS-side via __world_find_entity (Rust-bound each frame)
            Ok(Some(ReflectValue::Str("null".into())))
        },
    ));

    reg.register("GameObject", BindingEntry::new(
        "FindWithTag",
        "Finds the first active GameObject tagged with tag.",
        vec![ParamMeta::new("tag", ScriptType::String)],
        Some(ScriptType::Object),
        |_args| Ok(Some(ReflectValue::Str("null".into()))),
    ));

    reg.register("GameObject", BindingEntry::new(
        "FindGameObjectsWithTag",
        "Returns an array of all active GameObjects tagged with tag.",
        vec![ParamMeta::new("tag", ScriptType::String)],
        Some(ScriptType::Array),
        |_args| Ok(Some(ReflectValue::Str("[]".into()))),
    ));

    reg.register("GameObject", BindingEntry::new(
        "Destroy",
        "Destroys the GameObject, component or asset. Optional delay in seconds.",
        vec![
            ParamMeta::new("objectName", ScriptType::String),
            ParamMeta::new("delay", ScriptType::Float).optional(),
        ],
        None,
        |args| {
            let name  = args.first().map(as_str).unwrap_or_default();
            let delay = args.get(1).map(as_f32).unwrap_or(0.0);
            push(WorldCommand::Destroy { entity_name: name, delay });
            Ok(None)
        },
    ));

    reg.register("GameObject", BindingEntry::new(
        "Instantiate",
        "Clones a prefab asset into the scene. Returns the new object's name.",
        vec![
            ParamMeta::new("prefabPath", ScriptType::String).doc("Asset path to .prefab.json"),
            ParamMeta::new("position",   ScriptType::Vec3).optional(),
            ParamMeta::new("rotation",   ScriptType::Quat).optional(),
        ],
        Some(ScriptType::String),
        |args| {
            let path = args.first().map(as_str).unwrap_or_default();
            let pos = match args.get(1) { Some(ReflectValue::Vec3(v)) => Some(*v), _ => None };
            let rot = match args.get(2) { Some(ReflectValue::Quat(q)) => Some(*q), _ => None };
            push(WorldCommand::Instantiate { prefab_path: path, parent: None, position: pos, rotation: rot });
            Ok(Some(ReflectValue::Str("__pending_instantiate__".into())))
        },
    ));

    reg.register("GameObject", BindingEntry::new(
        "SetActive",
        "Activates/deactivates the GameObject by name.",
        vec![
            ParamMeta::new("name",   ScriptType::String),
            ParamMeta::new("active", ScriptType::Bool),
        ],
        None,
        |args| {
            let name   = args.first().map(as_str).unwrap_or_default();
            let active = match args.get(1) { Some(ReflectValue::Bool(b)) => *b, _ => true };
            push(WorldCommand::SetActive { entity_name: name, active });
            Ok(None)
        },
    ));

    // ── SceneManager ─────────────────────────────────────────────────────────
    reg.register("SceneManager", BindingEntry::new(
        "LoadScene",
        "Loads the scene at the given path. set additive=true to load without unloading current.",
        vec![
            ParamMeta::new("scenePath", ScriptType::String),
            ParamMeta::new("additive",  ScriptType::Bool).optional(),
        ],
        None,
        |args| {
            let path    = args.first().map(as_str).unwrap_or_default();
            let additive = match args.get(1) { Some(ReflectValue::Bool(b)) => *b, _ => false };
            push(WorldCommand::LoadScene { scene_path: path, additive });
            Ok(None)
        },
    ));

    reg.register("SceneManager", BindingEntry::new(
        "GetActiveScene",
        "Returns an object describing the currently loaded scene.",
        vec![],
        Some(ScriptType::Object),
        |_args| Ok(Some(ReflectValue::Str("{\"name\":\"main\"}".into()))),
    ));
}

// ── JS extension ───────────────────────────────────────────────────────────────
pub const GAMEOBJECT_JS_EXTENSION: &str = r#"
// ── GameObject instance wrapper ───────────────────────────────────────────────
// Uses a plain constructor function to avoid redeclaring the `const GameObject`
// namespace object already set up in ENGINE_BOOTSTRAP_JS.
function _GameObjectInstance(data) {
    this.name      = data ? data.name      : "";
    this.tag       = data ? data.tag       : "Untagged";
    this.active    = data ? data.active    : true;
    this.transform = data ? data.transform : null;
    this._id       = data ? data.id        : null;
}

_GameObjectInstance.prototype.SetActive = function(value) {
    GameObject.SetActive(this.name, value);
    this.active = value;
};
_GameObjectInstance.prototype.GetComponent    = function(t) { return __native_invoke("Component.Get",    this.name, t); };
_GameObjectInstance.prototype.AddComponent    = function(t) { return __native_invoke("Component.Add",    this.name, t); };
_GameObjectInstance.prototype.RemoveComponent = function(t) { return __native_invoke("Component.Remove", this.name, t); };
_GameObjectInstance.prototype.CompareTag      = function(tag) { return this.tag === tag; };
_GameObjectInstance.prototype.SendMessage = function(methodName, value) {
    for (const b of __behaviours) {
        if (b.__scriptTargetName === this.name && typeof b[methodName] === "function") {
            try { b[methodName](value); } catch(e) { console.error("SendMessage:", e); }
        }
    }
};
_GameObjectInstance.prototype.BroadcastMessage = function(methodName, value) {
    this.SendMessage(methodName, value);
};

// ── Extend the existing GameObject namespace with high-level static helpers ────
Object.assign(GameObject, {
    _wrap: function(data) { return new _GameObjectInstance(data); },
    Find: function(name) {
        const raw = __native_invoke("GameObject.Find", name);
        if (!raw || raw === "null") return null;
        try { return new _GameObjectInstance(typeof raw === "string" ? JSON.parse(raw) : raw); } catch(e) { return null; }
    },
    FindWithTag: function(tag) {
        const raw = __native_invoke("GameObject.FindWithTag", tag);
        if (!raw || raw === "null") return null;
        try { return new _GameObjectInstance(typeof raw === "string" ? JSON.parse(raw) : raw); } catch(e) { return null; }
    },
    FindGameObjectsWithTag: function(tag) {
        const raw = __native_invoke("GameObject.FindGameObjectsWithTag", tag);
        try {
            const arr = typeof raw === "string" ? JSON.parse(raw) : (Array.isArray(raw) ? raw : []);
            return arr.map(d => new _GameObjectInstance(d));
        } catch(e) { return []; }
    },
    Destroy: function(obj, delay) {
        __native_invoke("GameObject.Destroy", typeof obj === "string" ? obj : obj.name, delay || 0);
    },
    Instantiate: function(prefabPath, position, rotation) {
        return __native_invoke("GameObject.Instantiate", prefabPath,
            position || {x:0,y:0,z:0}, rotation || {x:0,y:0,z:0,w:1});
    },
    SetActive: function(name, active) { __native_invoke("GameObject.SetActive", name, active); },
});

// ── Object shim (Unity Object.Destroy / Object.Instantiate) ───────────────────
// `Object` is the JS built-in — extend it directly without redeclaring.
Object.Destroy           = function(obj, delay)     { GameObject.Destroy(obj, delay); };
Object.Instantiate       = function(path, pos, rot) { return GameObject.Instantiate(path, pos, rot); };
Object.DontDestroyOnLoad = function(_obj)           {};

// ── SceneManager property shims ───────────────────────────────────────────────
if (typeof SceneManager !== "undefined" && !SceneManager.hasOwnProperty("_guarded")) {
    Object.defineProperty(SceneManager, "_guarded", { value: true, writable: false });
}
"#;
