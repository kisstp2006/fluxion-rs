// ============================================================
// fluxion-scripting — Physics API
//
// Unity-compatible Physics module.
// Handlers that need live ECS/physics data are stubs here;
// the sandbox wires them up by replacing handlers via
// registry.register() after the default register() call.
//
// Unity equivalents:
//   Physics.Raycast(origin, direction, maxDist?)    → RaycastHit
//   Physics.gravity                                  → Vector3
//   Physics.OverlapSphere(center, radius)            → colliders[]
// ============================================================

use fluxion_core::ReflectValue;
use crate::binding_registry::{BindingEntry, ParamMeta, ScriptBindingRegistry, ScriptType};

pub fn register(reg: &mut ScriptBindingRegistry) {
    // Physics.Raycast(origin, direction, maxDistance?)
    reg.register("Physics", BindingEntry::new(
        "Raycast",
        "Casts a ray and returns the first hit. Returns null if nothing was hit.",
        vec![
            ParamMeta::new("origin",      ScriptType::Vec3).doc("World-space ray origin"),
            ParamMeta::new("direction",   ScriptType::Vec3).doc("World-space ray direction (normalised)"),
            ParamMeta::new("maxDistance", ScriptType::Float).optional().doc("Maximum distance (default: Infinity)"),
        ],
        Some(ScriptType::Object),
        |_args| {
            // Stub — real implementation supplied by PhysicsEcsWorld integration
            Ok(Some(ReflectValue::Str("null".into())))
        },
    ));

    // Physics.RaycastAll
    reg.register("Physics", BindingEntry::new(
        "RaycastAll",
        "Casts a ray and returns ALL hits sorted by distance.",
        vec![
            ParamMeta::new("origin",      ScriptType::Vec3),
            ParamMeta::new("direction",   ScriptType::Vec3),
            ParamMeta::new("maxDistance", ScriptType::Float).optional(),
        ],
        Some(ScriptType::Array),
        |_args| Ok(Some(ReflectValue::Str("[]".into()))),
    ));

    // Physics.OverlapSphere
    reg.register("Physics", BindingEntry::new(
        "OverlapSphere",
        "Returns an array of colliders whose bounding volumes overlap the given sphere.",
        vec![
            ParamMeta::new("center", ScriptType::Vec3),
            ParamMeta::new("radius", ScriptType::Float),
        ],
        Some(ScriptType::Array),
        |_args| Ok(Some(ReflectValue::Str("[]".into()))),
    ));

    // Physics.OverlapBox
    reg.register("Physics", BindingEntry::new(
        "OverlapBox",
        "Returns colliders inside an axis-aligned box.",
        vec![
            ParamMeta::new("center",     ScriptType::Vec3),
            ParamMeta::new("halfExtents",ScriptType::Vec3),
        ],
        Some(ScriptType::Array),
        |_args| Ok(Some(ReflectValue::Str("[]".into()))),
    ));

    // Physics.gravity (get)
    reg.register("Physics", BindingEntry::new(
        "getGravity",
        "Returns the gravity vector used by the physics simulation.",
        vec![],
        Some(ScriptType::Vec3),
        |_args| Ok(Some(ReflectValue::Vec3([0.0, -9.81, 0.0]))),
    ));

    // Physics.gravity (set)
    reg.register("Physics", BindingEntry::new(
        "setGravity",
        "Sets the gravity vector for the physics simulation.",
        vec![ParamMeta::new("gravity", ScriptType::Vec3)],
        None,
        |_args| Ok(None),
    ));

    // Physics.Simulate — manual step (for deterministic tests)
    reg.register("Physics", BindingEntry::new(
        "Simulate",
        "Simulates physics by the given step size. Use carefully in FixedUpdate only.",
        vec![ParamMeta::new("step", ScriptType::Float)],
        None,
        |_args| Ok(None),
    ));

    // Physics.IgnoreCollision
    reg.register("Physics", BindingEntry::new(
        "IgnoreCollision",
        "Marks two colliders to ignore collisions with each other.",
        vec![
            ParamMeta::new("colliderA", ScriptType::Object),
            ParamMeta::new("colliderB", ScriptType::Object),
            ParamMeta::new("ignore",    ScriptType::Bool).optional(),
        ],
        None,
        |_args| Ok(None),
    ));
}

// ── JS extension: Rigidbody component helper + Physics namespace properties ────
pub const PHYSICS_JS_EXTENSION: &str = r#"
// ── Physics namespace property shims ─────────────────────────────────────────
Object.defineProperty(Physics, "gravity", {
    get() { return Physics.getGravity(); },
    set(v) { Physics.setGravity(v); },
    configurable: true,
});

// ── RaycastHit helper class ───────────────────────────────────────────────────
class RaycastHit {
    constructor(data) {
        this.point      = data ? data.point      : { x:0, y:0, z:0 };
        this.normal     = data ? data.normal     : { x:0, y:1, z:0 };
        this.distance   = data ? data.distance   : 0;
        this.entityId   = data ? data.entityId   : null;
        this.entityName = data ? data.entityName : null;
    }
    get collider() { return this; }
}

// Wrap Physics.Raycast so it returns a RaycastHit or null
const _rawRaycast = Physics.Raycast.bind(Physics);
Physics.Raycast = function(origin, direction, maxDistance) {
    const raw = _rawRaycast(origin, direction, maxDistance ?? Infinity);
    if (!raw || raw === "null") return null;
    try {
        const parsed = typeof raw === "string" ? JSON.parse(raw) : raw;
        return parsed ? new RaycastHit(parsed) : null;
    } catch { return null; }
};

const _rawRaycastAll = Physics.RaycastAll.bind(Physics);
Physics.RaycastAll = function(origin, direction, maxDistance) {
    const raw = _rawRaycastAll(origin, direction, maxDistance ?? Infinity);
    try {
        const arr = typeof raw === "string" ? JSON.parse(raw) : (Array.isArray(raw) ? raw : []);
        return arr.map(d => new RaycastHit(d));
    } catch { return []; }
};

// ── Rigidbody component wrapper ───────────────────────────────────────────────
// Usage in scripts:
//   const rb = new Rigidbody(this.entity.id);
//   rb.AddForce({ x:0, y:10, z:0 });
class Rigidbody {
    constructor(entityId) {
        this._id = entityId;
    }

    AddForce(force, forceMode)     { return __native_invoke("Rigidbody.AddForce",     this._id, force, forceMode ?? "Force"); }
    AddTorque(torque, forceMode)   { return __native_invoke("Rigidbody.AddTorque",    this._id, torque, forceMode ?? "Force"); }
    AddRelativeForce(force)        { return __native_invoke("Rigidbody.AddRelativeForce", this._id, force); }
    MovePosition(pos)              { return __native_invoke("Rigidbody.MovePosition", this._id, pos); }
    MoveRotation(rot)              { return __native_invoke("Rigidbody.MoveRotation", this._id, rot); }
    Sleep()                        { return __native_invoke("Rigidbody.Sleep",        this._id); }
    WakeUp()                       { return __native_invoke("Rigidbody.WakeUp",       this._id); }

    get velocity()      { return __native_invoke("Rigidbody.getVelocity",      this._id); }
    set velocity(v)     {        __native_invoke("Rigidbody.setVelocity",      this._id, v); }
    get angularVelocity(){ return __native_invoke("Rigidbody.getAngularVelocity", this._id); }
    set angularVelocity(v){       __native_invoke("Rigidbody.setAngularVelocity", this._id, v); }
    get mass()          { return __native_invoke("Rigidbody.getMass",          this._id); }
    set mass(v)         {        __native_invoke("Rigidbody.setMass",          this._id, v); }
    get isKinematic()   { return __native_invoke("Rigidbody.getIsKinematic",   this._id); }
    set isKinematic(v)  {        __native_invoke("Rigidbody.setIsKinematic",   this._id, v); }
    get useGravity()    { return __native_invoke("Rigidbody.getUseGravity",    this._id); }
    set useGravity(v)   {        __native_invoke("Rigidbody.setUseGravity",    this._id, v); }
    get drag()          { return __native_invoke("Rigidbody.getDrag",          this._id); }
    set drag(v)         {        __native_invoke("Rigidbody.setDrag",          this._id, v); }
    get angularDrag()   { return __native_invoke("Rigidbody.getAngularDrag",   this._id); }
    set angularDrag(v)  {        __native_invoke("Rigidbody.setAngularDrag",   this._id, v); }
}

// Register Rigidbody per-entity handlers into __native_invoke dispatch
// (these get routed by the "Rigidbody.*" prefix)
const _rbStubs = [
    "AddForce","AddTorque","AddRelativeForce","MovePosition","MoveRotation",
    "Sleep","WakeUp",
    "getVelocity","setVelocity","getAngularVelocity","setAngularVelocity",
    "getMass","setMass","getIsKinematic","setIsKinematic",
    "getUseGravity","setUseGravity","getDrag","setDrag","getAngularDrag","setAngularDrag",
];
"#;
