// ============================================================
// fluxion-scripting — Time API
//
// Unity-compatible Time module.
// Actual values are pushed each frame via auto_binding::push_time_snapshot.
// These handlers exist for Rune support and .d.ts generation.
//
// Unity equivalents:
//   Time.deltaTime          → dt
//   Time.time               → elapsed
//   Time.fixedDeltaTime     → fixedDt
//   Time.frameCount         → frameCount
//   Time.timeScale          → timeScale
//   Time.unscaledDeltaTime  → dt before timeScale
//   Time.realtimeSinceStartup → monotonic clock
// ============================================================

use fluxion_core::ReflectValue;
use crate::binding_registry::{BindingEntry, ParamMeta, ScriptBindingRegistry, ScriptType};

pub fn register(reg: &mut ScriptBindingRegistry) {
    reg.register("Time", BindingEntry::new(
        "getDeltaTime",
        "Time in seconds it took to complete the last frame (scaled by timeScale).",
        vec![],
        Some(ScriptType::Float),
        |_| Ok(Some(ReflectValue::F32(0.016))),
    ));

    reg.register("Time", BindingEntry::new(
        "getTime",
        "The time at the beginning of this frame (seconds since level load).",
        vec![],
        Some(ScriptType::Float),
        |_| Ok(Some(ReflectValue::F32(0.0))),
    ));

    reg.register("Time", BindingEntry::new(
        "getFixedDeltaTime",
        "The fixed time step used for physics and FixedUpdate.",
        vec![],
        Some(ScriptType::Float),
        |_| Ok(Some(ReflectValue::F32(0.016667))),
    ));

    reg.register("Time", BindingEntry::new(
        "setFixedDeltaTime",
        "Sets the fixed time step (seconds).",
        vec![ParamMeta::new("value", ScriptType::Float)],
        None,
        |_| Ok(None),
    ));

    reg.register("Time", BindingEntry::new(
        "getFrameCount",
        "The total number of frames rendered since the start of the application.",
        vec![],
        Some(ScriptType::Int),
        |_| Ok(Some(ReflectValue::U32(0))),
    ));

    reg.register("Time", BindingEntry::new(
        "getTimeScale",
        "The scale at which time passes. 1 = real time, 0 = paused, 2 = double speed.",
        vec![],
        Some(ScriptType::Float),
        |_| Ok(Some(ReflectValue::F32(1.0))),
    ));

    reg.register("Time", BindingEntry::new(
        "setTimeScale",
        "Sets the time scale.",
        vec![ParamMeta::new("scale", ScriptType::Float)],
        None,
        |_| Ok(None),
    ));

    reg.register("Time", BindingEntry::new(
        "getUnscaledDeltaTime",
        "Delta time independent of timeScale.",
        vec![],
        Some(ScriptType::Float),
        |_| Ok(Some(ReflectValue::F32(0.016))),
    ));

    reg.register("Time", BindingEntry::new(
        "getUnscaledTime",
        "Elapsed time since start, independent of timeScale.",
        vec![],
        Some(ScriptType::Float),
        |_| Ok(Some(ReflectValue::F32(0.0))),
    ));

    reg.register("Time", BindingEntry::new(
        "getRealtimeSinceStartup",
        "Monotonic clock — wall-clock time since application startup.",
        vec![],
        Some(ScriptType::Float),
        |_| Ok(Some(ReflectValue::F32(0.0))),
    ));
}

// ── JS extension ───────────────────────────────────────────────────────────────
// The auto-generated Time object already has getDeltaTime / getTime / … functions.
// We add property shims so scripts can use Time.deltaTime instead of Time.getDeltaTime().
// We also keep the raw snapshot fields (Time.dt, Time.elapsed, …) that Rust writes
// each frame directly via push_time_snapshot.
pub const TIME_JS_EXTENSION: &str = r#"
// ── Time property shims (Unity-compatible names) ──────────────────────────────
// Time.dt / .elapsed / .fixedDt / .frameCount / .timeScale are plain data
// properties written each frame by push_time_snapshot.  We ONLY add Unity-style
// aliases here; redefining frameCount / timeScale as getters with the same name
// would create circular self-referencing getters → stack overflow.
Object.defineProperties(Time, {
    deltaTime:       { get() { return Time.dt;      }, configurable: true },
    time:            { get() { return Time.elapsed; }, configurable: true },
    fixedDeltaTime:  {
        get() { return Time.fixedDt; },
        set(v){ Time.fixedDt = v; Time.setFixedDeltaTime(v); },
        configurable: true,
    },
    unscaledDeltaTime:    { get() { return Time.getUnscaledDeltaTime();    }, configurable: true },
    unscaledTime:         { get() { return Time.getUnscaledTime();         }, configurable: true },
    realtimeSinceStartup: { get() { return Time.getRealtimeSinceStartup(); }, configurable: true },
});
"#;
