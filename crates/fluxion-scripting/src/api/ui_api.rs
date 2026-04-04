// ============================================================
// fluxion-scripting — UI / egui API
//
// Exposes a Unity-IMGUI-like immediate-mode GUI from scripts.
// Rust drains the command queue each frame and renders it via egui.
//
// Usage in scripts:
//   class HUD extends FluxionBehaviour {
//     onGUI() {
//       GUI.Label({ x:10, y:10, w:200, h:30 }, "Health: " + this.hp);
//       if (GUI.Button({ x:10, y:50, w:120, h:30 }, "Restart")) { ... }
//     }
//   }
//
// Rust drains `__gui_commands` every frame and passes them to egui.
// ============================================================

use std::sync::Mutex;
use fluxion_core::ReflectValue;
use crate::binding_registry::{BindingEntry, ParamMeta, ScriptBindingRegistry, ScriptType};

// ── Shared egui command queue (drained by the renderer each frame) ─────────────

/// One queued GUI draw call from a script.
#[derive(Debug, Clone)]
pub enum GuiCommand {
    Label   { rect: [f32;4], text: String },
    Button  { rect: [f32;4], text: String, id: u32 },
    Box     { rect: [f32;4], text: String },
    Toggle  { rect: [f32;4], value: bool, text: String, id: u32 },
    Slider  { rect: [f32;4], value: f32, min: f32, max: f32, id: u32 },
    TextField { rect: [f32;4], text: String, id: u32 },
    Window  { id: u32, title: String, rect: [f32;4] },
    EndWindow,
    BeginGroup { rect: [f32;4] },
    EndGroup,
    DrawTexture { rect: [f32;4], path: String },
    Color { r: f32, g: f32, b: f32, a: f32 },
}

lazy_static::lazy_static! {
    /// Commands queued by scripts this frame. Drained by the GUI renderer.
    pub static ref GUI_COMMAND_QUEUE: Mutex<Vec<GuiCommand>> = Mutex::new(Vec::new());
    /// Button-pressed results from last frame (id → was_pressed).
    pub static ref GUI_BUTTON_RESULTS: Mutex<std::collections::HashMap<u32, bool>> = Mutex::new(std::collections::HashMap::new());
    /// Toggle values (id → current_value).
    pub static ref GUI_TOGGLE_VALUES: Mutex<std::collections::HashMap<u32, bool>> = Mutex::new(std::collections::HashMap::new());
    /// Slider values (id → current_value).
    pub static ref GUI_SLIDER_VALUES: Mutex<std::collections::HashMap<u32, f32>> = Mutex::new(std::collections::HashMap::new());
    /// Text field contents (id → text).
    pub static ref GUI_TEXTFIELD_VALUES: Mutex<std::collections::HashMap<u32, String>> = Mutex::new(std::collections::HashMap::new());
}

/// Drain the command queue — call once per frame before egui rendering.
pub fn drain_commands() -> Vec<GuiCommand> {
    GUI_COMMAND_QUEUE.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

/// Feed back button/toggle/slider results so scripts can read them next frame.
pub fn set_button_result(id: u32, pressed: bool) {
    if let Ok(mut m) = GUI_BUTTON_RESULTS.lock() { m.insert(id, pressed); }
}
pub fn set_toggle_value(id: u32, value: bool) {
    if let Ok(mut m) = GUI_TOGGLE_VALUES.lock() { m.insert(id, value); }
}
pub fn set_slider_value(id: u32, value: f32) {
    if let Ok(mut m) = GUI_SLIDER_VALUES.lock() { m.insert(id, value); }
}
pub fn set_textfield_value(id: u32, text: String) {
    if let Ok(mut m) = GUI_TEXTFIELD_VALUES.lock() { m.insert(id, text); }
}

fn push(cmd: GuiCommand) {
    if let Ok(mut q) = GUI_COMMAND_QUEUE.lock() { q.push(cmd); }
}

fn as_f32(v: &ReflectValue) -> f32 {
    match v { ReflectValue::F32(f) => *f, ReflectValue::U32(n) => *n as f32, _ => 0.0 }
}
fn as_str(v: &ReflectValue) -> String {
    match v { ReflectValue::Str(s) => s.clone(), _ => String::new() }
}
fn as_bool(v: &ReflectValue) -> bool {
    match v { ReflectValue::Bool(b) => *b, _ => false }
}

pub fn register(reg: &mut ScriptBindingRegistry) {
    // GUI.Label(rect, text)
    reg.register("GUI", BindingEntry::new(
        "Label",
        "Draws a text label at the given Rect.",
        vec![
            ParamMeta::new("rect", ScriptType::Object).doc("{x,y,w,h}"),
            ParamMeta::new("text", ScriptType::String),
        ],
        None,
        |args| {
            let text = args.get(1).map(as_str).unwrap_or_default();
            let r = parse_rect_obj(args.get(0));
            push(GuiCommand::Label { rect: r, text });
            Ok(None)
        },
    ));

    // GUI.Button(rect, text) → bool
    reg.register("GUI", BindingEntry::new(
        "Button",
        "Draws a button. Returns true on the frame it is clicked.",
        vec![
            ParamMeta::new("rect", ScriptType::Object),
            ParamMeta::new("text", ScriptType::String),
        ],
        Some(ScriptType::Bool),
        |args| {
            let text = args.get(1).map(as_str).unwrap_or_default();
            let r = parse_rect_obj(args.get(0));
            let id = hash_rect_text(&r, &text);
            push(GuiCommand::Button { rect: r, text, id });
            let pressed = GUI_BUTTON_RESULTS.lock()
                .map(|m| *m.get(&id).unwrap_or(&false))
                .unwrap_or(false);
            Ok(Some(ReflectValue::Bool(pressed)))
        },
    ));

    // GUI.Box(rect, text)
    reg.register("GUI", BindingEntry::new(
        "Box",
        "Draws a box with optional label.",
        vec![
            ParamMeta::new("rect", ScriptType::Object),
            ParamMeta::new("text", ScriptType::String).optional(),
        ],
        None,
        |args| {
            let text = args.get(1).map(as_str).unwrap_or_default();
            let r = parse_rect_obj(args.get(0));
            push(GuiCommand::Box { rect: r, text });
            Ok(None)
        },
    ));

    // GUI.Toggle(rect, value, text) → bool
    reg.register("GUI", BindingEntry::new(
        "Toggle",
        "Draws a toggle. Returns the current value.",
        vec![
            ParamMeta::new("rect",  ScriptType::Object),
            ParamMeta::new("value", ScriptType::Bool),
            ParamMeta::new("text",  ScriptType::String),
        ],
        Some(ScriptType::Bool),
        |args| {
            let value = args.get(1).map(as_bool).unwrap_or(false);
            let text  = args.get(2).map(as_str).unwrap_or_default();
            let r = parse_rect_obj(args.get(0));
            let id = hash_rect_text(&r, &text);
            push(GuiCommand::Toggle { rect: r, value, text, id });
            let current = GUI_TOGGLE_VALUES.lock()
                .map(|m| *m.get(&id).unwrap_or(&value))
                .unwrap_or(value);
            Ok(Some(ReflectValue::Bool(current)))
        },
    ));

    // GUI.HorizontalSlider(rect, value, min, max) → float
    reg.register("GUI", BindingEntry::new(
        "HorizontalSlider",
        "Draws a horizontal slider. Returns the current value.",
        vec![
            ParamMeta::new("rect",  ScriptType::Object),
            ParamMeta::new("value", ScriptType::Float),
            ParamMeta::new("min",   ScriptType::Float),
            ParamMeta::new("max",   ScriptType::Float),
        ],
        Some(ScriptType::Float),
        |args| {
            let value = args.get(1).map(as_f32).unwrap_or(0.0);
            let min   = args.get(2).map(as_f32).unwrap_or(0.0);
            let max   = args.get(3).map(as_f32).unwrap_or(1.0);
            let r = parse_rect_obj(args.get(0));
            let id = (r[0] as u32).wrapping_mul(31).wrapping_add(r[1] as u32);
            push(GuiCommand::Slider { rect: r, value, min, max, id });
            let current = GUI_SLIDER_VALUES.lock()
                .map(|m| *m.get(&id).unwrap_or(&value))
                .unwrap_or(value);
            Ok(Some(ReflectValue::F32(current)))
        },
    ));

    // GUI.TextField(rect, text) → string
    reg.register("GUI", BindingEntry::new(
        "TextField",
        "Draws a single-line text field. Returns the current text.",
        vec![
            ParamMeta::new("rect", ScriptType::Object),
            ParamMeta::new("text", ScriptType::String),
        ],
        Some(ScriptType::String),
        |args| {
            let text = args.get(1).map(as_str).unwrap_or_default();
            let r = parse_rect_obj(args.get(0));
            let id = hash_rect_text(&r, &text);
            push(GuiCommand::TextField { rect: r, text: text.clone(), id });
            let current = GUI_TEXTFIELD_VALUES.lock()
                .map(|m| m.get(&id).cloned().unwrap_or_else(|| text.clone()))
                .unwrap_or(text);
            Ok(Some(ReflectValue::Str(current)))
        },
    ));

    // GUI.BeginGroup / EndGroup
    reg.register("GUI", BindingEntry::new(
        "BeginGroup",
        "Begins a group (clip region).",
        vec![ParamMeta::new("rect", ScriptType::Object)],
        None,
        |args| {
            let r = parse_rect_obj(args.get(0));
            push(GuiCommand::BeginGroup { rect: r });
            Ok(None)
        },
    ));
    reg.register("GUI", BindingEntry::new(
        "EndGroup",
        "Ends the current group.",
        vec![],
        None,
        |_args| { push(GuiCommand::EndGroup); Ok(None) },
    ));

    // GUI.DrawTexture(rect, texturePath)
    reg.register("GUI", BindingEntry::new(
        "DrawTexture",
        "Draws a texture inside a Rect.",
        vec![
            ParamMeta::new("rect", ScriptType::Object),
            ParamMeta::new("texturePath", ScriptType::String),
        ],
        None,
        |args| {
            let path = args.get(1).map(as_str).unwrap_or_default();
            let r = parse_rect_obj(args.get(0));
            push(GuiCommand::DrawTexture { rect: r, path });
            Ok(None)
        },
    ));

    // GUILayout.Label / GUILayout.Button — auto-layout variants (queued same way)
    reg.register("GUILayout", BindingEntry::new(
        "Label",
        "Auto-layout label.",
        vec![ParamMeta::new("text", ScriptType::String)],
        None,
        |args| {
            let text = args.get(0).map(as_str).unwrap_or_default();
            push(GuiCommand::Label { rect: [0.0,0.0,-1.0,-1.0], text });
            Ok(None)
        },
    ));
    reg.register("GUILayout", BindingEntry::new(
        "Button",
        "Auto-layout button. Returns true when clicked.",
        vec![ParamMeta::new("text", ScriptType::String)],
        Some(ScriptType::Bool),
        |args| {
            let text = args.get(0).map(as_str).unwrap_or_default();
            let id = fnv1a(text.as_bytes());
            push(GuiCommand::Button { rect: [0.0,0.0,-1.0,-1.0], text, id });
            let pressed = GUI_BUTTON_RESULTS.lock()
                .map(|m| *m.get(&id).unwrap_or(&false))
                .unwrap_or(false);
            Ok(Some(ReflectValue::Bool(pressed)))
        },
    ));
}

// ── helpers ────────────────────────────────────────────────────────────────────

fn parse_rect_obj(_v: Option<&ReflectValue>) -> [f32;4] {
    [0.0;4]  // JS objects decoded on JS side; Rust side receives pre-flattened floats via JSON
}

fn hash_rect_text(r: &[f32;4], t: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in t.as_bytes() { h = h.wrapping_mul(16777619) ^ (*b as u32); }
    for f in r { h = h.wrapping_mul(16777619) ^ f.to_bits(); }
    h
}

fn fnv1a(data: &[u8]) -> u32 {
    let mut h: u32 = 2166136261;
    for b in data { h = h.wrapping_mul(16777619) ^ (*b as u32); }
    h
}

// ── JS extension ───────────────────────────────────────────────────────────────
pub const UI_JS_EXTENSION: &str = r#"
// ── Rect helper ───────────────────────────────────────────────────────────────
function Rect(x, y, w, h) {
    if (!(this instanceof Rect)) return new Rect(x, y, w, h);
    this.x = x ?? 0; this.y = y ?? 0;
    this.w = w ?? 100; this.h = h ?? 30;
}

// ── GUI wrapper: translate Rect objects to flat args ─────────────────────────
// The auto-generated GUI functions pass rect as a JS object; we normalise to {x,y,w,h}.
const _guiFns = ["Label","Button","Box","Toggle","HorizontalSlider","TextField","BeginGroup","DrawTexture"];
for (const fn_name of _guiFns) {
    const orig = GUI[fn_name];
    if (typeof orig !== "function") continue;
    GUI[fn_name] = function(rect, ...rest) {
        const r = rect instanceof Rect
            ? rect
            : (rect && typeof rect === "object" ? { x: rect.x??0, y: rect.y??0, w: rect.w??rect.width??100, h: rect.h??rect.height??30 } : new Rect());
        return orig(r, ...rest);
    };
}

// ── onGUI lifecycle hook ──────────────────────────────────────────────────────
function __fluxion_gui_tick() {
    for (const b of __behaviours) {
        if (!b.enabled || !b._started) continue;
        if (typeof b.onGUI === "function") {
            try { b.onGUI(); } catch(e) { console.error("onGUI() error:", e); }
        }
    }
}
"#;
