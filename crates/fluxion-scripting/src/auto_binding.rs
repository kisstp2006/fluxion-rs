// ============================================================
// fluxion-scripting — auto_binding
//
// Bridges ScriptBindingRegistry → QuickJS VM.
//
// Responsibilities:
//   1. Register a single `__native_invoke(path, ...args)` Rust function
//      into the JS global scope.  All generated API wrappers call this.
//   2. Generate and eval the JS module objects (Input, Physics, …) from
//      the registry metadata, so every function in every module gets a
//      matching JS wrapper that routes through __native_invoke.
//   3. Provide `apply_frame_snapshot` which pushes per-frame state
//      (Time, Input) into JS without string-eval — using typed setters.
//
// No API logic lives here.  Logic lives in api/*.rs and is registered
// into the ScriptBindingRegistry before this module is invoked.
// ============================================================

use std::sync::{Arc, Mutex};

use anyhow::Context;
use fluxion_core::{ReflectValue, InputState};

use crate::vm::JsVm;
use crate::binding_registry::ScriptBindingRegistry;

// ── JSON-based arg/result serialisation ───────────────────────────────────────
// We cross the Rust↔JS boundary using plain Strings so we avoid `Ctx<'js>` /
// `Value<'js>` lifetime problems in closures that must be `'static`.

/// Deserialise a JSON array string (e.g. `"[1, true, \"hello\"]"`) into
/// a `Vec<ReflectValue>` for dispatch through the registry.
fn json_array_to_reflect_values(json: &str) -> Vec<ReflectValue> {
    let arr: serde_json::Value = match serde_json::from_str(json) {
        Ok(v)  => v,
        Err(_) => return Vec::new(),
    };
    let Some(items) = arr.as_array() else { return Vec::new(); };
    items.iter().map(json_value_to_reflect).collect()
}

fn json_value_to_reflect(v: &serde_json::Value) -> ReflectValue {
    match v {
        serde_json::Value::Bool(b)   => ReflectValue::Bool(*b),
        serde_json::Value::Number(n) => ReflectValue::F32(n.as_f64().unwrap_or(0.0) as f32),
        serde_json::Value::String(s) => ReflectValue::Str(s.clone()),
        serde_json::Value::Null      => ReflectValue::OptionStr(None),
        serde_json::Value::Object(m) => {
            let x = m.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            let y = m.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            let z = m.get("z").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            if let Some(w) = m.get("w").and_then(|v| v.as_f64()) {
                ReflectValue::Quat([x, y, z, w as f32])
            } else {
                ReflectValue::Vec3([x, y, z])
            }
        }
        serde_json::Value::Array(_)  => ReflectValue::Str(v.to_string()),
    }
}

/// Serialise a `ReflectValue` to a JSON string for returning to JS.
fn reflect_value_to_json_string(v: &ReflectValue) -> String {
    use fluxion_core::reflect::reflect_value_to_json;
    reflect_value_to_json(v).to_string()
}

// ── Core: register __native_invoke + generate module wrappers ─────────────────

/// Wire the entire registry into the JS VM.
///
/// Strategy: register a single `__native_invoke_raw(path, argsJson) → resultJson`
/// Rust function, then expose a thin JS `__native_invoke(path, ...args)` wrapper
/// that does `JSON.stringify`/`JSON.parse`.  This sidesteps all `Ctx<'js>` /
/// `Value<'js>` lifetime issues — Rust only touches plain `String` arguments.
///
/// Call once after `setup_bindings` but before loading user scripts.
#[cfg(not(target_arch = "wasm32"))]
pub fn apply_registry_to_vm(
    vm: &JsVm,
    registry: Arc<Mutex<ScriptBindingRegistry>>,
) -> anyhow::Result<()> {
    // 1. Register __native_invoke_raw(path, argsJson) → resultJson | undefined
    {
        let reg = Arc::clone(&registry);
        vm.ctx.with(|ctx| {
            let globals = ctx.globals();
            let raw_fn = rquickjs::Function::new(
                ctx.clone(),
                move |path: String, args_json: String| -> Option<String> {
                    let reg = match reg.lock() {
                        Ok(r)  => r,
                        Err(_) => return None,
                    };

                    // Deserialise args from JSON array
                    let args = json_array_to_reflect_values(&args_json);

                    match reg.invoke(&path, &args) {
                        Ok(Some(rv)) => Some(reflect_value_to_json_string(&rv)),
                        Ok(None)     => None,
                        Err(e) => {
                            log::warn!("[native_invoke] {}: {}", path, e);
                            None
                        }
                    }
                },
            )?;
            globals.set("__native_invoke_raw", raw_fn)?;
            Ok::<_, rquickjs::Error>(())
        })
        .map_err(|e| anyhow::anyhow!("Failed to register __native_invoke_raw: {e}"))?;
    }

    // 2. Thin JS wrapper: __native_invoke(path, ...args)
    vm.eval(
        r#"
function __native_invoke(path) {
    const args = Array.prototype.slice.call(arguments, 1);
    const raw = __native_invoke_raw(path, JSON.stringify(args));
    if (raw === undefined || raw === null) return undefined;
    try { return JSON.parse(raw); } catch { return raw; }
}
"#,
        "<native-invoke-wrapper>",
    )?;

    // 3. Generate & eval the JS module wrapper objects from registry metadata
    let js_code = {
        let reg = registry.lock().map_err(|_| anyhow::anyhow!("registry lock poisoned"))?;
        generate_module_js(&reg)
    };
    vm.eval(&js_code, "<auto-generated-api>")
        .context("Failed to eval auto-generated API wrappers")?;

    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn apply_registry_to_vm(
    _vm: &JsVm,
    _registry: Arc<Mutex<ScriptBindingRegistry>>,
) -> anyhow::Result<()> {
    Ok(())
}

// ── JS code generation ─────────────────────────────────────────────────────────

/// Generate the JS source for all module objects from the registry.
///
/// Output example:
/// ```js
/// const Input = Object.assign(typeof Input !== 'undefined' ? Input : {}, {
///   GetKeyDown: function(key) { return __native_invoke("Input.GetKeyDown", key); },
///   …
/// });
/// ```
/// We use `Object.assign` so that if a module object already exists (e.g. `Input`
/// was partially defined in engine-bootstrap), we extend it rather than replace it.
pub fn generate_module_js(registry: &ScriptBindingRegistry) -> String {
    let mut out = String::from(
        "// ── Auto-generated API wrappers (ScriptBindingRegistry) ─────────────\n\n",
    );

    for module in registry.module_names() {
        let entries = registry.module_entries(module);
        if entries.is_empty() { continue; }

        out.push_str(&format!(
            "const {m} = Object.assign(typeof {m} !== 'undefined' ? {m} : {{}}, {{\n",
            m = module,
        ));

        for e in entries {
            // JS parameter names
            let param_names: Vec<&str> = e.params.iter().map(|p| p.name).collect();
            let args_joined = param_names.join(", ");
            let invoke_args = if param_names.is_empty() {
                format!("\"{}\"", full_path(module, e.name))
            } else {
                format!("\"{}\", {}", full_path(module, e.name), args_joined)
            };

            // JSDoc
            if !e.description.is_empty() {
                out.push_str(&format!("  /** {} */\n", e.description));
            }
            out.push_str(&format!(
                "  {fn_name}: function({args}) {{ return __native_invoke({invoke}); }},\n",
                fn_name = e.name,
                args    = args_joined,
                invoke  = invoke_args,
            ));
        }

        out.push_str("});\n\n");
    }

    out
}

fn full_path(module: &str, name: &str) -> String {
    format!("{}.{}", module, name)
}

// ── Per-frame snapshot injection ───────────────────────────────────────────────

/// Push Time state into `Time.*` JS properties without string-eval.
/// Much faster than formatting a JS eval string each frame.
#[cfg(not(target_arch = "wasm32"))]
pub fn push_time_snapshot(
    vm: &JsVm,
    dt: f32,
    elapsed: f32,
    fixed_dt: f32,
    frame: u64,
    time_scale: f32,
) -> anyhow::Result<()> {
    vm.ctx.with(|ctx| {
        let globals = ctx.globals();
        let time: rquickjs::Object = globals.get("Time")?;
        time.set("dt",         dt as f64)?;
        time.set("elapsed",    elapsed as f64)?;
        time.set("fixedDt",    fixed_dt as f64)?;
        time.set("frameCount", frame as f64)?;
        time.set("timeScale",  time_scale as f64)?;
        Ok::<_, rquickjs::Error>(())
    })
    .map_err(|e| anyhow::anyhow!("push_time_snapshot: {e}"))
}

#[cfg(target_arch = "wasm32")]
pub fn push_time_snapshot(_vm: &JsVm, _dt: f32, _elapsed: f32, _fixed_dt: f32, _frame: u64, _time_scale: f32) -> anyhow::Result<()> { Ok(()) }

/// Push InputState into `Input.*` JS properties without string-eval.
#[cfg(not(target_arch = "wasm32"))]
pub fn push_input_snapshot(vm: &JsVm, input: &InputState) -> anyhow::Result<()> {
    vm.ctx.with(|ctx| {
        let globals = ctx.globals();
        let inp: rquickjs::Object = globals.get("Input")?;

        // Keys: build a fresh JS object { KeyW: true, ... }
        let keys_obj = rquickjs::Object::new(ctx.clone())?;
        for k in input.keys_down_iter() {
            keys_obj.set(k, true)?;
        }
        inp.set("_keys", keys_obj)?;

        // Mouse position
        let (mx, my) = input.mouse_position();
        let mp = rquickjs::Object::new(ctx.clone())?;
        mp.set("x", mx as f64)?;
        mp.set("y", my as f64)?;
        inp.set("_mousePos", mp)?;

        // Mouse delta
        let (mdx, mdy) = input.mouse_delta();
        let md = rquickjs::Object::new(ctx.clone())?;
        md.set("x", mdx as f64)?;
        md.set("y", mdy as f64)?;
        inp.set("_mouseDelta", md)?;

        // Scroll delta
        let (sdx, sdy) = input.scroll_delta();
        let sd = rquickjs::Object::new(ctx.clone())?;
        sd.set("x", sdx as f64)?;
        sd.set("y", sdy as f64)?;
        inp.set("_scrollDelta", sd)?;

        // Mouse buttons
        let mb = rquickjs::Object::new(ctx.clone())?;
        mb.set("left",   input.mouse_left())?;
        mb.set("middle", input.mouse_middle())?;
        mb.set("right",  input.mouse_right())?;
        inp.set("_mouseButtons", mb)?;

        // Gamepad
        let gp = rquickjs::Object::new(ctx.clone())?;
        gp.set("connected", input.gamepad_connected)?;
        let (lx, ly) = input.gamepad_left_stick;
        let (rx, ry) = input.gamepad_right_stick;
        gp.set("lx", lx as f64)?;
        gp.set("ly", ly as f64)?;
        gp.set("rx", rx as f64)?;
        gp.set("ry", ry as f64)?;
        gp.set("lt", input.gamepad_left_trigger as f64)?;
        gp.set("rt", input.gamepad_right_trigger as f64)?;
        gp.set("buttons", input.gamepad_buttons as f64)?;
        inp.set("_gamepad", gp)?;

        Ok::<_, rquickjs::Error>(())
    })
    .map_err(|e| anyhow::anyhow!("push_input_snapshot: {e}"))
}

#[cfg(target_arch = "wasm32")]
pub fn push_input_snapshot(_vm: &JsVm, _input: &InputState) -> anyhow::Result<()> { Ok(()) }
