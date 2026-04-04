// ============================================================
// fluxion-scripting — Debug API
//
// Unity-compatible Debug module for scripts.
//
// Unity equivalents:
//   Debug.Log(message)
//   Debug.LogWarning(message)
//   Debug.LogError(message)
//   Debug.DrawLine(start, end, color, duration)
//   Debug.DrawRay(origin, direction, color, duration)
//   Debug.Assert(condition, message)
//   Debug.Break()
// ============================================================

use std::sync::Mutex;
use fluxion_core::ReflectValue;
use crate::binding_registry::{BindingEntry, ParamMeta, ScriptBindingRegistry, ScriptType};

#[derive(Debug, Clone)]
pub struct DebugDrawRequest {
    pub start:    [f32; 3],
    pub end:      [f32; 3],
    pub color:    [f32; 4],
    pub duration: f32,
}

lazy_static::lazy_static! {
    pub static ref DEBUG_DRAW_QUEUE: Mutex<Vec<DebugDrawRequest>> = Mutex::new(Vec::new());
}

pub fn drain_draw_requests() -> Vec<DebugDrawRequest> {
    DEBUG_DRAW_QUEUE.lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}

fn as_str(v: &ReflectValue) -> String {
    match v { ReflectValue::Str(s) => s.clone(), _ => String::new() }
}
fn as_f32(v: &ReflectValue) -> f32 {
    match v { ReflectValue::F32(f) => *f, ReflectValue::U32(n) => *n as f32, _ => 0.0 }
}
fn as_vec3(v: Option<&ReflectValue>) -> [f32; 3] {
    match v { Some(ReflectValue::Vec3(a)) => *a, _ => [0.0; 3] }
}
fn as_color(v: Option<&ReflectValue>) -> [f32; 4] {
    match v {
        Some(ReflectValue::Color4(a)) => *a,
        Some(ReflectValue::Color3(a)) => [a[0], a[1], a[2], 1.0],
        _ => [1.0, 1.0, 1.0, 1.0],
    }
}

pub fn register(reg: &mut ScriptBindingRegistry) {
    reg.register("Debug", BindingEntry::new(
        "Log",
        "Logs a message to the console.",
        vec![
            ParamMeta::new("message", ScriptType::String),
            ParamMeta::new("context", ScriptType::Object).optional(),
        ],
        None,
        |args| {
            let msg = args.first().map(as_str).unwrap_or_default();
            log::info!("[Script] {}", msg);
            Ok(None)
        },
    ));

    reg.register("Debug", BindingEntry::new(
        "LogWarning",
        "Logs a warning message.",
        vec![ParamMeta::new("message", ScriptType::String)],
        None,
        |args| {
            let msg = args.first().map(as_str).unwrap_or_default();
            log::warn!("[Script] {}", msg);
            Ok(None)
        },
    ));

    reg.register("Debug", BindingEntry::new(
        "LogError",
        "Logs an error message.",
        vec![ParamMeta::new("message", ScriptType::String)],
        None,
        |args| {
            let msg = args.first().map(as_str).unwrap_or_default();
            log::error!("[Script] {}", msg);
            Ok(None)
        },
    ));

    reg.register("Debug", BindingEntry::new(
        "Assert",
        "Logs an error if condition is false.",
        vec![
            ParamMeta::new("condition", ScriptType::Bool),
            ParamMeta::new("message",   ScriptType::String).optional(),
        ],
        None,
        |args| {
            let ok  = match args.first() { Some(ReflectValue::Bool(b)) => *b, _ => true };
            let msg = args.get(1).map(as_str).unwrap_or_else(|| "Assertion failed".into());
            if !ok { log::error!("[Script Assert] {}", msg); }
            Ok(None)
        },
    ));

    reg.register("Debug", BindingEntry::new(
        "DrawLine",
        "Draws a line in the scene view between start and end.",
        vec![
            ParamMeta::new("start",    ScriptType::Vec3),
            ParamMeta::new("end",      ScriptType::Vec3),
            ParamMeta::new("color",    ScriptType::Vec4).optional(),
            ParamMeta::new("duration", ScriptType::Float).optional(),
        ],
        None,
        |args| {
            let start    = as_vec3(args.first());
            let end      = as_vec3(args.get(1));
            let color    = as_color(args.get(2));
            let duration = args.get(3).map(as_f32).unwrap_or(0.0);
            if let Ok(mut q) = DEBUG_DRAW_QUEUE.lock() {
                q.push(DebugDrawRequest { start, end, color, duration });
            }
            Ok(None)
        },
    ));

    reg.register("Debug", BindingEntry::new(
        "DrawRay",
        "Draws a ray from origin in direction.",
        vec![
            ParamMeta::new("origin",    ScriptType::Vec3),
            ParamMeta::new("direction", ScriptType::Vec3),
            ParamMeta::new("color",     ScriptType::Vec4).optional(),
            ParamMeta::new("duration",  ScriptType::Float).optional(),
        ],
        None,
        |args| {
            let origin = as_vec3(args.first());
            let dir    = as_vec3(args.get(1));
            let end    = [origin[0]+dir[0], origin[1]+dir[1], origin[2]+dir[2]];
            let color  = as_color(args.get(2));
            let dur    = args.get(3).map(as_f32).unwrap_or(0.0);
            if let Ok(mut q) = DEBUG_DRAW_QUEUE.lock() {
                q.push(DebugDrawRequest { start: origin, end, color, duration: dur });
            }
            Ok(None)
        },
    ));

    reg.register("Debug", BindingEntry::new(
        "Break",
        "Pauses the editor (no-op in standalone builds).",
        vec![],
        None,
        |_| { log::warn!("[Script] Debug.Break()"); Ok(None) },
    ));

    reg.register("Debug", BindingEntry::new(
        "ClearDeveloperConsole",
        "Clears errors from the developer console.",
        vec![],
        None,
        |_| Ok(None),
    ));
}

// ── JS extension ───────────────────────────────────────────────────────────────
pub const DEBUG_JS_EXTENSION: &str = r#"
// ── Debug.Log / LogWarning / LogError also route through console ──────────────
const _debugLog   = Debug.Log.bind(Debug);
const _debugWarn  = Debug.LogWarning.bind(Debug);
const _debugError = Debug.LogError.bind(Debug);

Debug.Log      = function(...args) { const s = args.map(String).join(" "); console.log(s);   _debugLog(s);   };
Debug.LogWarning = function(...args) { const s = args.map(String).join(" "); console.warn(s);  _debugWarn(s);  };
Debug.LogError   = function(...args) { const s = args.map(String).join(" "); console.error(s); _debugError(s); };

// ── Color helper (matches Unity Color(r,g,b,a)) ───────────────────────────────
function Color(r, g, b, a) {
    if (!(this instanceof Color)) return new Color(r, g, b, a);
    this.r = r ?? 0; this.g = g ?? 0; this.b = b ?? 0; this.a = a ?? 1;
}
Color.red     = new Color(1,0,0,1);
Color.green   = new Color(0,1,0,1);
Color.blue    = new Color(0,0,1,1);
Color.white   = new Color(1,1,1,1);
Color.black   = new Color(0,0,0,1);
Color.yellow  = new Color(1,0.92,0.016,1);
Color.cyan    = new Color(0,1,1,1);
Color.magenta = new Color(1,0,1,1);
Color.clear   = new Color(0,0,0,0);

// ── Vector helpers ────────────────────────────────────────────────────────────
function Vector3(x, y, z) {
    if (!(this instanceof Vector3)) return new Vector3(x, y, z);
    this.x = x ?? 0; this.y = y ?? 0; this.z = z ?? 0;
}
Vector3.zero    = new Vector3(0,0,0);
Vector3.one     = new Vector3(1,1,1);
Vector3.up      = new Vector3(0,1,0);
Vector3.down    = new Vector3(0,-1,0);
Vector3.forward = new Vector3(0,0,1);
Vector3.back    = new Vector3(0,0,-1);
Vector3.right   = new Vector3(1,0,0);
Vector3.left    = new Vector3(-1,0,0);
Vector3.distance = function(a, b) {
    const dx=a.x-b.x, dy=a.y-b.y, dz=a.z-b.z;
    return Math.sqrt(dx*dx+dy*dy+dz*dz);
};
Vector3.dot = function(a,b) { return a.x*b.x+a.y*b.y+a.z*b.z; };
Vector3.cross = function(a,b) {
    return new Vector3(a.y*b.z-a.z*b.y, a.z*b.x-a.x*b.z, a.x*b.y-a.y*b.x);
};
Vector3.lerp = function(a,b,t) {
    return new Vector3(a.x+(b.x-a.x)*t, a.y+(b.y-a.y)*t, a.z+(b.z-a.z)*t);
};
Vector3.prototype.magnitude = function() { return Math.sqrt(this.x*this.x+this.y*this.y+this.z*this.z); };
Vector3.prototype.normalized = function() { const m=this.magnitude()||1; return new Vector3(this.x/m,this.y/m,this.z/m); };
Vector3.prototype.add = function(v) { return new Vector3(this.x+v.x,this.y+v.y,this.z+v.z); };
Vector3.prototype.sub = function(v) { return new Vector3(this.x-v.x,this.y-v.y,this.z-v.z); };
Vector3.prototype.scale = function(s) { return new Vector3(this.x*s,this.y*s,this.z*s); };
Vector3.prototype.toString = function() { return `Vector3(${this.x.toFixed(3)}, ${this.y.toFixed(3)}, ${this.z.toFixed(3)})`; };

function Vector2(x, y) {
    if (!(this instanceof Vector2)) return new Vector2(x, y);
    this.x = x ?? 0; this.y = y ?? 0;
}
Vector2.zero  = new Vector2(0,0);
Vector2.one   = new Vector2(1,1);
Vector2.up    = new Vector2(0,1);
Vector2.right = new Vector2(1,0);
Vector2.prototype.magnitude = function() { return Math.sqrt(this.x*this.x+this.y*this.y); };

function Quaternion(x, y, z, w) {
    if (!(this instanceof Quaternion)) return new Quaternion(x,y,z,w);
    this.x = x ?? 0; this.y = y ?? 0; this.z = z ?? 0; this.w = w ?? 1;
}
Quaternion.identity = new Quaternion(0,0,0,1);
Quaternion.Euler = function(x,y,z) {
    const cx=Math.cos(x*Mathf.DEG2RAD*.5), sx=Math.sin(x*Mathf.DEG2RAD*.5);
    const cy=Math.cos(y*Mathf.DEG2RAD*.5), sy=Math.sin(y*Mathf.DEG2RAD*.5);
    const cz=Math.cos(z*Mathf.DEG2RAD*.5), sz=Math.sin(z*Mathf.DEG2RAD*.5);
    return new Quaternion(
        sx*cy*cz+cx*sy*sz,
        cx*sy*cz-sx*cy*sz,
        cx*cy*sz+sx*sy*cz,
        cx*cy*cz-sx*sy*sz,
    );
};
"#;
