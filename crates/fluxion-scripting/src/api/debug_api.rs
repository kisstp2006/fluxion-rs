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

use fluxion_core::ReflectValue;
use fluxion_core::Color;
use fluxion_core::debug_draw;
use crate::binding_registry::{BindingEntry, ParamMeta, ScriptBindingRegistry, ScriptType};

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
            let [sx, sy, sz] = as_vec3(args.first());
            let [ex, ey, ez] = as_vec3(args.get(1));
            let [r, g, b, a] = as_color(args.get(2));
            debug_draw::draw_line(
                glam::Vec3::new(sx, sy, sz),
                glam::Vec3::new(ex, ey, ez),
                Color::Custom(r, g, b, a),
            );
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
            let [ox, oy, oz] = as_vec3(args.first());
            let [dx, dy, dz] = as_vec3(args.get(1));
            let [r, g, b, a] = as_color(args.get(2));
            debug_draw::draw_ray(
                glam::Vec3::new(ox, oy, oz),
                glam::Vec3::new(dx, dy, dz),
                Color::Custom(r, g, b, a),
            );
            Ok(None)
        },
    ));

    reg.register("Debug", BindingEntry::new(
        "DrawSphere",
        "Draws a wireframe sphere in the scene view.",
        vec![
            ParamMeta::new("center", ScriptType::Vec3),
            ParamMeta::new("radius", ScriptType::Float),
            ParamMeta::new("color",  ScriptType::Vec4).optional(),
        ],
        None,
        |args| {
            let [cx, cy, cz] = as_vec3(args.first());
            let radius       = args.get(1).map(as_f32).unwrap_or(1.0);
            let [r, g, b, a] = as_color(args.get(2));
            debug_draw::draw_sphere(
                glam::Vec3::new(cx, cy, cz),
                radius,
                Color::Custom(r, g, b, a),
            );
            Ok(None)
        },
    ));

    reg.register("Debug", BindingEntry::new(
        "DrawBox",
        "Draws a wireframe axis-aligned box in the scene view.",
        vec![
            ParamMeta::new("center",      ScriptType::Vec3),
            ParamMeta::new("halfExtents", ScriptType::Vec3),
            ParamMeta::new("color",       ScriptType::Vec4).optional(),
        ],
        None,
        |args| {
            let [cx, cy, cz] = as_vec3(args.first());
            let [hx, hy, hz] = as_vec3(args.get(1));
            let [r, g, b, a] = as_color(args.get(2));
            let center = glam::Vec3::new(cx, cy, cz);
            let half   = glam::Vec3::new(hx, hy, hz);
            debug_draw::draw_aabb(center - half, center + half, Color::Custom(r, g, b, a));
            Ok(None)
        },
    ));

    reg.register("Debug", BindingEntry::new(
        "DrawCross",
        "Draws a cross (3 axis lines) at the given position.",
        vec![
            ParamMeta::new("position", ScriptType::Vec3),
            ParamMeta::new("size",     ScriptType::Float).optional(),
            ParamMeta::new("color",    ScriptType::Vec4).optional(),
        ],
        None,
        |args| {
            let [px, py, pz] = as_vec3(args.first());
            let size         = args.get(1).map(as_f32).unwrap_or(1.0);
            let [r, g, b, a] = as_color(args.get(2));
            debug_draw::draw_cross(
                glam::Vec3::new(px, py, pz),
                size,
                Color::Custom(r, g, b, a),
            );
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
