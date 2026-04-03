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

    isKeyDown(key)    { return !!Input._keys[key]; },
    getAxis(neg, pos) { return (Input._keys[pos] ? 1 : 0) - (Input._keys[neg] ? 1 : 0); },
    get horizontal()  { return Input.getAxis("KeyA", "KeyD"); },
    get vertical()    { return Input.getAxis("KeyS", "KeyW"); },
    get mousePosition() { return Input._mousePos; },
    get mouseDelta()    { return Input._mouseDelta; },
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
fn bind_log_functions(vm: &JsVm) -> anyhow::Result<()> {
    vm.ctx.with(|ctx| {
        let globals = ctx.globals();

        // __log_info(msg)
        let log_info = rquickjs::Function::new(ctx.clone(), |msg: String| {
            log::info!("[JS] {}", msg);
        })?;
        globals.set("__log_info", log_info)?;

        // __log_warn(msg)
        let log_warn = rquickjs::Function::new(ctx.clone(), |msg: String| {
            log::warn!("[JS] {}", msg);
        })?;
        globals.set("__log_warn", log_warn)?;

        // __log_error(msg)
        let log_error = rquickjs::Function::new(ctx.clone(), |msg: String| {
            log::error!("[JS] {}", msg);
        })?;
        globals.set("__log_error", log_error)?;

        Ok::<_, rquickjs::Error>(())
    }).map_err(|e| anyhow::anyhow!("Failed to bind log functions: {e}"))?;

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
