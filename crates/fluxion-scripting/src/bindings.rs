// ============================================================
// fluxion-scripting — Engine bindings
//
// Injects engine state into the JS global scope so scripts can
// access the world, time, input, etc.
//
// Binding strategy: we use a data-snapshot approach rather than
// live Rust references. Each frame, we snapshot mutable state (transforms,
// time, etc.) into JS globals before calling script update functions.
// Scripts write back via queued commands that Rust applies after the tick.
//
// This avoids the borrow-checker complexity of giving JS live &mut references
// into Rust data structures, and matches how the TypeScript engine worked
// (everything was already single-threaded JS with no lifetime issues).
//
// Injected globals:
//   Engine        — { renderGraph, pause(), resume() }
//   Time          — { dt, elapsed, fixedDt, frameCount }
//   console       — { log, warn, error, info }  (re-routed to Rust log crate)
// ============================================================

use crate::vm::JsVm;
use crate::behaviour::inject_base_classes;
use crate::render_pass::RENDER_GRAPH_JS;

/// The JavaScript engine bootstrap code.
/// Sets up the Engine global and the console override.
const ENGINE_BOOTSTRAP_JS: &str = r#"
// ── console → Rust log ────────────────────────────────────────────────────────
// The QuickJS default console is no-op. We route it through Rust's log crate
// via __log_info, __log_warn, __log_error which are bound in Rust.

const console = {
    log(...args)   { if (typeof __log_info  === "function") __log_info(...args.map(String));  },
    warn(...args)  { if (typeof __log_warn  === "function") __log_warn(...args.map(String));  },
    error(...args) { if (typeof __log_error === "function") __log_error(...args.map(String)); },
    info(...args)  { if (typeof __log_info  === "function") __log_info(...args.map(String));  },
};

// ── Engine global ─────────────────────────────────────────────────────────────
// Scripts access engine subsystems through this object.

const Engine = {
    renderGraph: null,  // set by render_pass.js
    paused: false,

    pause()  { Engine.paused = true;  if (typeof __engine_pause  === "function") __engine_pause();  },
    resume() { Engine.paused = false; if (typeof __engine_resume === "function") __engine_resume(); },

    // Debug lines for native egui panel (Rust drains `lines` each frame after scripts).
    ui: {
        lines: [],
        pushLine(s) { Engine.ui.lines.push(String(s)); },
        clearLines() { Engine.ui.lines.length = 0; },
    },
};

// ── Time global ───────────────────────────────────────────────────────────────
// Updated each frame by Rust before calling __fluxion_tick.

const Time = {
    dt:         0.016,
    elapsed:    0.0,
    fixedDt:    0.016667,
    frameCount: 0,
    timeScale:  1.0,
};

// ── Input global ─────────────────────────────────────────────────────────────
// Read-only snapshot of input state, updated each frame.

const Input = {
    // These are set by Rust each frame via __input_update
    _keys: {},
    _mousePos: { x: 0, y: 0 },
    _mouseDelta: { x: 0, y: 0 },
    _mouseButtons: { left: false, middle: false, right: false },
    _gamepad: { connected: false, lx: 0, ly: 0, rx: 0, ry: 0, lt: 0, rt: 0, buttons: 0 },

    isKeyDown(key)    { return !!Input._keys[key]; },
    getAxis(neg, pos) { return (Input._keys[pos] ? 1 : 0) - (Input._keys[neg] ? 1 : 0); },
    get horizontal()  { return Input.getAxis("KeyA", "KeyD"); },
    get vertical()    { return Input.getAxis("KeyS", "KeyW"); },
    get mousePosition() { return Input._mousePos; },
    get mouseDelta()    { return Input._mouseDelta; },
    getMouseButton(i) {
        if (i === 0) return Input._mouseButtons.left;
        if (i === 1) return Input._mouseButtons.middle;
        if (i === 2) return Input._mouseButtons.right;
        return false;
    },

    get gamepadConnected() { return Input._gamepad.connected; },
    get gamepadLeftStick() { return { x: Input._gamepad.lx, y: Input._gamepad.ly }; },
    get gamepadRightStick() { return { x: Input._gamepad.rx, y: Input._gamepad.ry }; },
    get gamepadLeftTrigger() { return Input._gamepad.lt; },
    get gamepadRightTrigger() { return Input._gamepad.rt; },
    get gamepadButtons() { return Input._gamepad.buttons; },
};
"#;

/// Initialize the JS VM with all engine bindings.
///
/// Call this once after creating the VM, before loading user scripts.
pub fn setup_bindings(vm: &JsVm) -> anyhow::Result<()> {
    // 1. Engine bootstrap (console, Engine, Time, Input globals)
    vm.eval(ENGINE_BOOTSTRAP_JS, "<engine-bootstrap>")?;

    // 2. FluxionBehaviour base class + script registry
    inject_base_classes(vm)?;

    // 3. Render graph JS API
    vm.eval(RENDER_GRAPH_JS, "<render-graph-api>")?;

    // 4. Bind Rust log functions into JS
    bind_log_functions(vm)?;

    Ok(())
}

/// Bind Rust's `log` crate into QuickJS console functions.
#[cfg(not(target_arch = "wasm32"))]
fn bind_log_functions(vm: &JsVm) -> anyhow::Result<()> {
    vm.ctx.with(|ctx| {
        let globals = ctx.globals();

        let log_info = rquickjs::Function::new(ctx.clone(), |msg: String| {
            log::info!("[JS] {}", msg);
        })?;
        globals.set("__log_info", log_info)?;

        let log_warn = rquickjs::Function::new(ctx.clone(), |msg: String| {
            log::warn!("[JS] {}", msg);
        })?;
        globals.set("__log_warn", log_warn)?;

        let log_error = rquickjs::Function::new(ctx.clone(), |msg: String| {
            log::error!("[JS] {}", msg);
        })?;
        globals.set("__log_error", log_error)?;

        Ok::<_, rquickjs::Error>(())
    })
    .map_err(|e| anyhow::anyhow!("Failed to bind log functions: {e}"))?;

    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn bind_log_functions(_vm: &JsVm) -> anyhow::Result<()> {
    Ok(())
}

/// Update the JS `Time` global before each frame tick.
pub fn update_time_global(vm: &JsVm, dt: f32, elapsed: f32, fixed_dt: f32, frame: u64) -> anyhow::Result<()> {
    vm.eval(
        &format!(
            "Time.dt = {dt}; Time.elapsed = {elapsed}; Time.fixedDt = {fixed_dt}; Time.frameCount = {frame};",
        ),
        "<time-update>",
    )
}

/// Push [`fluxion_core::InputState`] into the JS `Input` global for script reads.
pub fn update_input_global(vm: &JsVm, input: &fluxion_core::InputState) -> anyhow::Result<()> {
    let mut map = serde_json::Map::new();
    for k in input.keys_down_iter() {
        map.insert(k.to_string(), serde_json::Value::Bool(true));
    }
    let keys_json = serde_json::Value::Object(map).to_string();
    let (mx, my) = input.mouse_position();
    let (mdx, mdy) = input.mouse_delta();
    let ml = input.mouse_left();
    let mm = input.mouse_middle();
    let mr = input.mouse_right();
    let gc = input.gamepad_connected;
    let (lx, ly) = input.gamepad_left_stick;
    let (rx, ry) = input.gamepad_right_stick;
    let lt = input.gamepad_left_trigger;
    let rt = input.gamepad_right_trigger;
    let gb = input.gamepad_buttons;
    vm.eval(
        &format!(
            "Input._keys = {keys_json}; \
             Input._mousePos = {{ x: {mx}, y: {my} }}; \
             Input._mouseDelta = {{ x: {mdx}, y: {mdy} }}; \
             Input._mouseButtons = {{ left: {ml}, middle: {mm}, right: {mr} }}; \
             Input._gamepad = {{ connected: {gc}, lx: {lx}, ly: {ly}, rx: {rx}, ry: {ry}, lt: {lt}, rt: {rt}, buttons: {gb} }};",
        ),
        "<input-update>",
    )
}

/// Copy `Engine.ui.lines` into Rust and reset the JS array (native QuickJS only).
#[cfg(not(target_arch = "wasm32"))]
pub fn drain_ui_debug_lines(vm: &JsVm) -> Vec<String> {
    vm.ctx
        .with(|ctx| -> Result<Vec<String>, rquickjs::Error> {
            let g = ctx.globals();
            let engine: rquickjs::Object = g.get("Engine")?;
            let ui: rquickjs::Object = engine.get("ui")?;
            let arr: rquickjs::Array = ui.get("lines")?;
            let n = arr.len();
            let mut out = Vec::with_capacity(n as usize);
            for i in 0..n {
                let s: String = arr.get(i)?;
                out.push(s);
            }
            let fresh = rquickjs::Array::new(ctx.clone())?;
            ui.set("lines", fresh)?;
            Ok(out)
        })
        .unwrap_or_default()
}

#[cfg(target_arch = "wasm32")]
pub fn drain_ui_debug_lines(_vm: &JsVm) -> Vec<String> {
    Vec::new()
}
