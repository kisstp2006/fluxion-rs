// ============================================================
// fluxion-scripting — Engine bindings
//
// Wires the ScriptBindingRegistry into the QuickJS VM.
//
// All per-API logic lives in api/*.rs.
// This module:
//   1. Creates & populates the ScriptBindingRegistry (once at startup)
//   2. Registers __native_invoke + generates JS module objects (auto_binding)
//   3. Injects JS extension snippets from each API module
//   4. Provides backward-compatible update_time_global / update_input_global
//      wrappers (delegate to auto_binding typed setters)
//   5. Provides drain_ui_debug_lines (unchanged — reads Engine.ui.lines)
// ============================================================

use std::sync::{Arc, Mutex};

use fluxion_core::InputState;

use crate::vm::JsVm;
use crate::behaviour::inject_base_classes;
use crate::render_pass::RENDER_GRAPH_JS;
use crate::binding_registry::ScriptBindingRegistry;
use crate::auto_binding;
use crate::api;
use crate::api::input_api::INPUT_JS_EXTENSION;
use crate::api::physics_api::PHYSICS_JS_EXTENSION;
use crate::api::ui_api::UI_JS_EXTENSION;
use crate::api::window_api::WINDOW_JS_EXTENSION;
use crate::api::time_api::TIME_JS_EXTENSION;
use crate::api::gameobject_api::GAMEOBJECT_JS_EXTENSION;
use crate::api::debug_api::DEBUG_JS_EXTENSION;

// ── Bootstrap JS (console shim + bare Engine/Time/Input/Screen/etc. stubs) ────
// These stubs define the global objects *before* the auto-generated wrappers
// run (which use Object.assign to extend them).  Keeping stubs here means
// any code that runs before setup is complete still has defined globals.

const ENGINE_BOOTSTRAP_JS: &str = r#"
// ── console → Rust log ────────────────────────────────────────────────────────
const console = {
    log(...args)   { if (typeof __log_info  === "function") __log_info(...args.map(String));  },
    warn(...args)  { if (typeof __log_warn  === "function") __log_warn(...args.map(String));  },
    error(...args) { if (typeof __log_error === "function") __log_error(...args.map(String)); },
    info(...args)  { if (typeof __log_info  === "function") __log_info(...args.map(String));  },
};

// ── Engine global ─────────────────────────────────────────────────────────────
const Engine = {
    renderGraph: null,
    paused: false,
    pause()  { Engine.paused = true;  if (typeof __engine_pause  === "function") __engine_pause();  },
    resume() { Engine.paused = false; if (typeof __engine_resume === "function") __engine_resume(); },
    ui: {
        lines: [],
        pushLine(s) { Engine.ui.lines.push(String(s)); },
        clearLines() { Engine.ui.lines.length = 0; },
    },
};

// ── Bare global stubs (auto-generated wrappers will extend these) ─────────────
const Time       = { dt: 0.016, elapsed: 0.0, fixedDt: 0.016667, frameCount: 0, timeScale: 1.0 };
const Input      = { _keys: {}, _mousePos: {x:0,y:0}, _mouseDelta: {x:0,y:0}, _scrollDelta: {x:0,y:0},
                     _mouseButtons: {left:false,middle:false,right:false},
                     _gamepad: {connected:false,lx:0,ly:0,rx:0,ry:0,lt:0,rt:0,buttons:0} };
const Physics    = {};
const Screen     = {};
const Application= {};
const Cursor     = {};
const GUI        = {};
const GUILayout  = {};
const GameObject = {};
const SceneManager = {};
const Debug      = {};
"#;

// ── Public API ─────────────────────────────────────────────────────────────────

/// Initialize the JS VM with all engine bindings.
/// Call once after creating the VM, before loading user scripts.
pub fn setup_bindings(vm: &JsVm) -> anyhow::Result<()> {
    // 1. Bare stubs + console shim
    vm.eval(ENGINE_BOOTSTRAP_JS, "<engine-bootstrap>")?;

    // 2. Rust log functions (still registered individually — no registry overhead for log)
    bind_log_functions(vm)?;

    // 3. FluxionBehaviour base class + script registry
    inject_base_classes(vm)?;

    // 4. Render graph JS API
    vm.eval(RENDER_GRAPH_JS, "<render-graph-api>")?;

    // 5. Build ScriptBindingRegistry with all API modules
    let mut registry = ScriptBindingRegistry::new();
    api::register_all(&mut registry);
    let registry = Arc::new(Mutex::new(registry));

    // 6. Wire registry → __native_invoke + generate JS module wrappers
    auto_binding::apply_registry_to_vm(vm, Arc::clone(&registry))?;

    // 7. JS extension snippets (add Unity-style getters, helper classes, etc.)
    vm.eval(INPUT_JS_EXTENSION,      "<input-ext>")?;
    vm.eval(PHYSICS_JS_EXTENSION,    "<physics-ext>")?;
    vm.eval(UI_JS_EXTENSION,         "<ui-ext>")?;
    vm.eval(WINDOW_JS_EXTENSION,     "<window-ext>")?;
    vm.eval(TIME_JS_EXTENSION,       "<time-ext>")?;
    vm.eval(GAMEOBJECT_JS_EXTENSION, "<gameobject-ext>")?;
    vm.eval(DEBUG_JS_EXTENSION,      "<debug-ext>")?;

    Ok(())
}

/// Update the JS `Time` global before each frame tick.
/// Delegates to the typed `push_time_snapshot` (no string-eval).
pub fn update_time_global(
    vm: &JsVm,
    dt: f32,
    elapsed: f32,
    fixed_dt: f32,
    frame: u64,
) -> anyhow::Result<()> {
    auto_binding::push_time_snapshot(vm, dt, elapsed, fixed_dt, frame, 1.0)
}

/// Push [`InputState`] into the JS `Input` global for script reads.
/// Delegates to the typed `push_input_snapshot` (no string-eval).
pub fn update_input_global(vm: &JsVm, input: &InputState) -> anyhow::Result<()> {
    auto_binding::push_input_snapshot(vm, input)?;
    // Notify JS of frame-edge tracking
    let _ = vm.eval("if (typeof __fluxion_input_begin_frame === 'function') __fluxion_input_begin_frame();",
                    "<input-frame-edge>");
    Ok(())
}

/// Copy `Engine.ui.lines` into Rust and reset the JS array.
#[cfg(not(target_arch = "wasm32"))]
pub fn drain_ui_debug_lines(vm: &JsVm) -> Vec<String> {
    vm.ctx
        .with(|ctx| -> Result<Vec<String>, rquickjs::Error> {
            let g      = ctx.globals();
            let engine: rquickjs::Object = g.get("Engine")?;
            let ui:     rquickjs::Object = engine.get("ui")?;
            let arr:    rquickjs::Array  = ui.get("lines")?;
            let n = arr.len();
            let mut out = Vec::with_capacity(n as usize);
            for i in 0..n {
                let s: String = arr.get(i)?;
                out.push(s);
            }
            ui.set("lines", rquickjs::Array::new(ctx.clone())?)?;
            Ok(out)
        })
        .unwrap_or_default()
}

#[cfg(target_arch = "wasm32")]
pub fn drain_ui_debug_lines(_vm: &JsVm) -> Vec<String> { Vec::new() }

// ── Private: log bridge ────────────────────────────────────────────────────────
// Log functions are still registered individually (not through the registry)
// because they are special: they are called *from* the console shim that runs
// before the registry is set up.

#[cfg(not(target_arch = "wasm32"))]
fn bind_log_functions(vm: &JsVm) -> anyhow::Result<()> {
    vm.ctx.with(|ctx| {
        let globals = ctx.globals();
        globals.set("__log_info",  rquickjs::Function::new(ctx.clone(), |msg: String| { log::info!("[JS] {}", msg); })?)?;
        globals.set("__log_warn",  rquickjs::Function::new(ctx.clone(), |msg: String| { log::warn!("[JS] {}", msg); })?)?;
        globals.set("__log_error", rquickjs::Function::new(ctx.clone(), |msg: String| { log::error!("[JS] {}", msg); })?)?;
        Ok::<_, rquickjs::Error>(())
    })
    .map_err(|e| anyhow::anyhow!("bind_log_functions: {e}"))
}

#[cfg(target_arch = "wasm32")]
fn bind_log_functions(_vm: &JsVm) -> anyhow::Result<()> { Ok(()) }
