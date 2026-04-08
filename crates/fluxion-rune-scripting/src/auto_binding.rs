// ============================================================
// auto_binding.rs — native Rune modules for the engine API
//
// Each `build_*_module()` returns a `rune::Module` that can be
// installed into the Rune context.  Functions are native Rust
// closures — no JSON bridge needed at the Rune level.
// ============================================================

use rune::Module;

/// Debug / logging module: `fluxion::native::debug`
pub fn build_debug_module() -> anyhow::Result<Module> {
    use fluxion_core::{Color, debug_draw};

    let mut m = Module::with_crate_item("fluxion", ["native", "debug"])?;

    m.function("log", |msg: String| {
        log::info!("[Rune] {msg}");
    }).build()?;

    m.function("warn", |msg: String| {
        log::warn!("[Rune] {msg}");
    }).build()?;

    m.function("error", |msg: String| {
        log::error!("[Rune] {msg}");
    }).build()?;

    // ── Draw functions (Rune 0.14 Function trait is implemented only up to arity 5) ──
    // 4-arg variants compile; 6-arg variants (draw_line, draw_ray, draw_box) exceed
    // the arity limit and must use the JS scripting API instead.

    let draw_sphere_fn: fn(f64,f64,f64,f64) = |cx,cy,cz,r| {
        debug_draw::draw_sphere(glam::Vec3::new(cx as f32,cy as f32,cz as f32), r as f32, Color::White);
    };
    m.function("draw_sphere", draw_sphere_fn).build()?;

    let draw_cross_fn: fn(f64,f64,f64,f64) = |px,py,pz,s| {
        debug_draw::draw_cross(glam::Vec3::new(px as f32,py as f32,pz as f32), s as f32, Color::White);
    };
    m.function("draw_cross", draw_cross_fn).build()?;

    Ok(m)
}

/// Time module: `fluxion::native::time`
pub fn build_time_module() -> anyhow::Result<Module> {
    use crate::vm::TIME_SNAPSHOT;

    let mut m = Module::with_crate_item("fluxion", ["native", "time"])?;

    m.function("delta_time",  || TIME_SNAPSHOT.load_dt()).build()?;
    m.function("time",         || TIME_SNAPSHOT.load_elapsed()).build()?;
    m.function("elapsed",      || TIME_SNAPSHOT.load_elapsed()).build()?;
    m.function("frame_count",  || TIME_SNAPSHOT.load_frame() as i64).build()?;
    m.function("frame",        || TIME_SNAPSHOT.load_frame() as i64).build()?;

    Ok(m)
}

/// Input module: `fluxion::native::input` (basic, snapshot-based)
pub fn build_input_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["native", "input"])?;

    m.function("get_key", |key: String| -> bool {
        crate::vm::input_snapshot().is_key_held(&key)
    }).build()?;

    m.function("get_key_down", |key: String| -> bool {
        crate::vm::input_snapshot().is_key_down(&key)
    }).build()?;

    m.function("get_key_up", |key: String| -> bool {
        crate::vm::input_snapshot().is_key_up(&key)
    }).build()?;

    Ok(m)
}

/// Viewport module: `fluxion::viewport` (kept for editor scripts)
///
/// Exposes the current editor viewport pixel size so scripts can do
/// resolution-aware layout without hard-coding dimensions.
pub fn build_viewport_module() -> anyhow::Result<Module> {
    use crate::vm::VIEWPORT_SNAPSHOT;

    let mut m = Module::with_crate_item("fluxion", ["viewport"])?;

    m.function("width",  || VIEWPORT_SNAPSHOT.load_width()  as i64).build()?;
    m.function("height", || VIEWPORT_SNAPSHOT.load_height() as i64).build()?;

    Ok(m)
}

/// Math module: `fluxion::native::math`
///
/// Exposes common f64 math functions as free functions because Rune 0.14
/// does not install method syntax (e.g. `x.sin()`) on primitive floats.
pub fn build_math_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["native", "math"])?;

    m.function("sin",   |x: f64| -> f64 { x.sin()   }).build()?;
    m.function("cos",   |x: f64| -> f64 { x.cos()   }).build()?;
    m.function("tan",   |x: f64| -> f64 { x.tan()   }).build()?;
    m.function("asin",  |x: f64| -> f64 { x.asin()  }).build()?;
    m.function("acos",  |x: f64| -> f64 { x.acos()  }).build()?;
    m.function("atan",  |x: f64| -> f64 { x.atan()  }).build()?;
    m.function("atan2", |y: f64, x: f64| -> f64 { y.atan2(x) }).build()?;
    m.function("sqrt",  |x: f64| -> f64 { x.sqrt()  }).build()?;
    m.function("abs",   |x: f64| -> f64 { x.abs()   }).build()?;
    m.function("floor", |x: f64| -> f64 { x.floor() }).build()?;
    m.function("ceil",  |x: f64| -> f64 { x.ceil()  }).build()?;
    m.function("round", |x: f64| -> f64 { x.round() }).build()?;
    m.function("min",   |a: f64, b: f64| -> f64 { if a < b { a } else { b } }).build()?;
    m.function("max",   |a: f64, b: f64| -> f64 { if a > b { a } else { b } }).build()?;
    m.function("clamp", |x: f64, lo: f64, hi: f64| -> f64 { x.clamp(lo, hi) }).build()?;
    m.function("pow",   |x: f64, e: f64| -> f64 { x.powf(e) }).build()?;
    m.function("exp",   |x: f64| -> f64 { x.exp()   }).build()?;
    m.function("ln",    |x: f64| -> f64 { x.ln()    }).build()?;
    m.function("log2",  |x: f64| -> f64 { x.log2()  }).build()?;
    m.function("sign",  |x: f64| -> f64 { x.signum() }).build()?;
    m.function("pi",    || -> f64 { std::f64::consts::PI }).build()?;
    m.function("tau",   || -> f64 { std::f64::consts::TAU }).build()?;

    Ok(m)
}

/// Compatibility alias: `fluxion::math` → same functions as `fluxion::native::math`.
/// Editor scripts (editor_camera.rn, etc.) use the old path; gameplay scripts use
/// the new `fluxion::native::math` path via the prelude's `Mathf` shim.
pub fn build_math_compat_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["math"])?;

    m.function("sin",   |x: f64| -> f64 { x.sin()   }).build()?;
    m.function("cos",   |x: f64| -> f64 { x.cos()   }).build()?;
    m.function("tan",   |x: f64| -> f64 { x.tan()   }).build()?;
    m.function("asin",  |x: f64| -> f64 { x.asin()  }).build()?;
    m.function("acos",  |x: f64| -> f64 { x.acos()  }).build()?;
    m.function("atan",  |x: f64| -> f64 { x.atan()  }).build()?;
    m.function("atan2", |y: f64, x: f64| -> f64 { y.atan2(x) }).build()?;
    m.function("sqrt",  |x: f64| -> f64 { x.sqrt()  }).build()?;
    m.function("abs",   |x: f64| -> f64 { x.abs()   }).build()?;
    m.function("floor", |x: f64| -> f64 { x.floor() }).build()?;
    m.function("ceil",  |x: f64| -> f64 { x.ceil()  }).build()?;
    m.function("round", |x: f64| -> f64 { x.round() }).build()?;
    m.function("min",   |a: f64, b: f64| -> f64 { if a < b { a } else { b } }).build()?;
    m.function("max",   |a: f64, b: f64| -> f64 { if a > b { a } else { b } }).build()?;
    m.function("clamp", |x: f64, lo: f64, hi: f64| -> f64 { x.clamp(lo, hi) }).build()?;
    m.function("pow",   |x: f64, e: f64| -> f64 { x.powf(e) }).build()?;
    m.function("exp",   |x: f64| -> f64 { x.exp()   }).build()?;
    m.function("ln",    |x: f64| -> f64 { x.ln()    }).build()?;
    m.function("log2",  |x: f64| -> f64 { x.log2()  }).build()?;
    m.function("sign",  |x: f64| -> f64 { x.signum() }).build()?;
    m.function("pi",    || -> f64 { std::f64::consts::PI }).build()?;
    m.function("tau",   || -> f64 { std::f64::consts::TAU }).build()?;

    Ok(m)
}

/// Collect all engine modules for installation into a Rune context.
pub fn all_modules() -> anyhow::Result<Vec<Module>> {
    Ok(vec![
        build_debug_module()?,
        build_time_module()?,
        build_input_module()?,
        build_viewport_module()?,
        build_math_module()?,
        build_math_compat_module()?,
    ])
}
