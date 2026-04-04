// ============================================================
// fluxion-scripting — Input API
//
// Unity-compatible Input module exposed to scripts.
// The actual input state is pushed each frame by `auto_binding::push_input_snapshot`.
// These handlers read from the JS-side `Input._keys` / `_mouseButtons` / etc.
// snapshots via the registry's ReflectValue args.
//
// Unity equivalents:
//   Input.GetKey(key)          → isKeyDown(key)
//   Input.GetKeyDown(key)      → (handled JS-side via _justPressed set)
//   Input.GetMouseButton(i)    → getMouseButton(i)
//   Input.GetAxis("Horizontal") → horizontal / vertical properties
//   Input.mousePosition        → _mousePos snapshot
//   Input.GetJoystickNames()   → gamepad connected state
// ============================================================

use fluxion_core::ReflectValue;
use crate::binding_registry::{BindingEntry, ParamMeta, ScriptBindingRegistry, ScriptType};

pub fn register(reg: &mut ScriptBindingRegistry) {
    // GetKey — checked against the live _keys snapshot (held this frame)
    reg.register("Input", BindingEntry::new(
        "GetKey",
        "Returns true while the user holds down the key identified by keyCode.",
        vec![ParamMeta::new("keyCode", ScriptType::String).doc("JS KeyboardEvent.code, e.g. 'KeyW', 'Space'")],
        Some(ScriptType::Bool),
        |_args| {
            // Runtime: JS-side reads Input._keys directly for perf.
            // This handler exists for Rune and for documentation / dts generation.
            // In JS the generated wrapper calls __native_invoke but the JS Input object
            // also has its own isKeyDown(key) getter that is faster (no crossing).
            Ok(Some(ReflectValue::Bool(false)))
        },
    ));

    // GetKeyDown — true only on the frame the key was first pressed
    reg.register("Input", BindingEntry::new(
        "GetKeyDown",
        "Returns true during the frame the user starts pressing down the key.",
        vec![ParamMeta::new("keyCode", ScriptType::String)],
        Some(ScriptType::Bool),
        |_args| Ok(Some(ReflectValue::Bool(false))),
    ));

    // GetKeyUp
    reg.register("Input", BindingEntry::new(
        "GetKeyUp",
        "Returns true during the frame the user releases the key.",
        vec![ParamMeta::new("keyCode", ScriptType::String)],
        Some(ScriptType::Bool),
        |_args| Ok(Some(ReflectValue::Bool(false))),
    ));

    // GetMouseButton(0/1/2)
    reg.register("Input", BindingEntry::new(
        "GetMouseButton",
        "Returns whether the given mouse button is held. 0=left, 1=right, 2=middle.",
        vec![ParamMeta::new("button", ScriptType::Int)],
        Some(ScriptType::Bool),
        |_args| Ok(Some(ReflectValue::Bool(false))),
    ));

    // GetMouseButtonDown
    reg.register("Input", BindingEntry::new(
        "GetMouseButtonDown",
        "Returns true during the frame the user pressed the mouse button.",
        vec![ParamMeta::new("button", ScriptType::Int)],
        Some(ScriptType::Bool),
        |_args| Ok(Some(ReflectValue::Bool(false))),
    ));

    // GetMouseButtonUp
    reg.register("Input", BindingEntry::new(
        "GetMouseButtonUp",
        "Returns true during the frame the user released the mouse button.",
        vec![ParamMeta::new("button", ScriptType::Int)],
        Some(ScriptType::Bool),
        |_args| Ok(Some(ReflectValue::Bool(false))),
    ));

    // GetAxis — named axis shorthand
    reg.register("Input", BindingEntry::new(
        "GetAxis",
        "Returns the value of the virtual axis identified by axisName (-1..1). Supports 'Horizontal', 'Vertical', 'Mouse X', 'Mouse Y'.",
        vec![ParamMeta::new("axisName", ScriptType::String)],
        Some(ScriptType::Float),
        |_args| Ok(Some(ReflectValue::F32(0.0))),
    ));

    // GetAxisRaw — no smoothing
    reg.register("Input", BindingEntry::new(
        "GetAxisRaw",
        "Returns the value of the virtual axis with no smoothing applied.",
        vec![ParamMeta::new("axisName", ScriptType::String)],
        Some(ScriptType::Float),
        |_args| Ok(Some(ReflectValue::F32(0.0))),
    ));

    // GetJoystickNames
    reg.register("Input", BindingEntry::new(
        "GetJoystickNames",
        "Returns an array of strings describing connected joysticks.",
        vec![],
        Some(ScriptType::Array),
        |_args| Ok(Some(ReflectValue::Str("[]".into()))),
    ));
}

// ── JS-side Input object extension (injected alongside auto-generated wrappers) ──
//
// The auto-generated module provides the __native_invoke-backed functions above.
// We extend it with JS-only convenience getters that read the snapshot directly
// (avoiding a Rust round-trip for hot-path per-frame reads).
pub const INPUT_JS_EXTENSION: &str = r#"
// ── Input extension: snapshot-direct getters and frame-edge tracking ──────────
// The auto-generated Input object is already defined at this point.
// We add fast snapshot-direct accessors and frame-edge (GetKeyDown/Up) tracking.

Object.assign(Input, {
    // ── snapshot-direct (no __native_invoke round trip) ──────────────────────
    isKeyDown(key)     { return !!Input._keys[key]; },
    isKeyUp(key)       { return !Input._keys[key]; },

    get mousePosition(){ return Input._mousePos; },
    get mouseDelta()   { return Input._mouseDelta; },
    get scrollDelta()  { return Input._scrollDelta; },

    getMouseButton(i) {
        if (i === 0) return Input._mouseButtons.left;
        if (i === 1) return Input._mouseButtons.right;
        if (i === 2) return Input._mouseButtons.middle;
        return false;
    },

    get horizontal()   { return (Input._keys["KeyD"] ? 1 : 0) - (Input._keys["KeyA"] ? 1 : 0); },
    get vertical()     { return (Input._keys["KeyW"] ? 1 : 0) - (Input._keys["KeyS"] ? 1 : 0); },

    // Named axis helper (Unity-style)
    GetAxis(name) {
        switch (name) {
            case "Horizontal": case "X": return Input.horizontal;
            case "Vertical":   case "Y": return Input.vertical;
            case "Mouse X": return Input._mouseDelta.x;
            case "Mouse Y": return Input._mouseDelta.y;
            case "Mouse ScrollWheel": return Input._scrollDelta.y;
            default: return 0;
        }
    },
    GetAxisRaw(name) { return Input.GetAxis(name); },

    // ── Gamepad ───────────────────────────────────────────────────────────────
    get gamepadConnected()   { return Input._gamepad.connected; },
    get gamepadLeftStick()   { return { x: Input._gamepad.lx, y: Input._gamepad.ly }; },
    get gamepadRightStick()  { return { x: Input._gamepad.rx, y: Input._gamepad.ry }; },
    get gamepadLeftTrigger() { return Input._gamepad.lt; },
    get gamepadRightTrigger(){ return Input._gamepad.rt; },
    get gamepadButtons()     { return Input._gamepad.buttons; },
    GetGamepadButton(bit)    { return !!(Input._gamepad.buttons & (1 << bit)); },

    // ── Frame-edge tracking (GetKeyDown / GetKeyUp / GetMouseButtonDown) ──────
    // Populated by __fluxion_input_begin_frame() called before each tick.
    _prevKeys:        {},
    _prevMouseBtns:   { left: false, middle: false, right: false },
    _justPressedKeys: {},
    _justReleasedKeys:{},
    _mouseDownThisFrame: { left: false, middle: false, right: false },
    _mouseUpThisFrame:   { left: false, middle: false, right: false },

    GetKey(keyCode)       { return !!Input._keys[keyCode]; },
    GetKeyDown(keyCode)   { return !!Input._justPressedKeys[keyCode]; },
    GetKeyUp(keyCode)     { return !!Input._justReleasedKeys[keyCode]; },
    GetMouseButtonDown(i) {
        if (i === 0) return Input._mouseDownThisFrame.left;
        if (i === 1) return Input._mouseDownThisFrame.right;
        if (i === 2) return Input._mouseDownThisFrame.middle;
        return false;
    },
    GetMouseButtonUp(i) {
        if (i === 0) return Input._mouseUpThisFrame.left;
        if (i === 1) return Input._mouseUpThisFrame.right;
        if (i === 2) return Input._mouseUpThisFrame.middle;
        return false;
    },

    GetJoystickNames() { return Input._gamepad.connected ? ["Gamepad 0"] : []; },
});

// Called by Rust before each script tick (after snapshot is pushed).
function __fluxion_input_begin_frame() {
    const prev = Input._prevKeys;
    const curr = Input._keys;
    Input._justPressedKeys  = {};
    Input._justReleasedKeys = {};
    for (const k in curr) {
        if (!prev[k]) Input._justPressedKeys[k] = true;
    }
    for (const k in prev) {
        if (!curr[k]) Input._justReleasedKeys[k] = true;
    }
    Input._prevKeys = Object.assign({}, curr);

    // Mouse button edge
    const pb = Input._prevMouseBtns;
    const cb = Input._mouseButtons;
    const btnNames = ["left", "right", "middle"];
    for (const b of btnNames) {
        Input._mouseDownThisFrame[b] = cb[b] && !pb[b];
        Input._mouseUpThisFrame[b]   = !cb[b] && pb[b];
    }
    Input._prevMouseBtns = Object.assign({}, cb);
}
"#;
