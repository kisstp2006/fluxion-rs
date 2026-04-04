// ============================================================
// auto_binding.rs — native Rune modules for the engine API
//
// Each `build_*_module()` returns a `rune::Module` that can be
// installed into the Rune context.  Functions are native Rust
// closures — no JSON bridge needed at the Rune level.
// ============================================================

use rune::Module;

/// Debug / logging module: `fluxion::debug`
pub fn build_debug_module() -> anyhow::Result<Module> {
    use fluxion_core::{Color, debug_draw};

    let mut m = Module::with_crate_item("fluxion", ["debug"])?;

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

/// Time module: `fluxion::time`
pub fn build_time_module() -> anyhow::Result<Module> {
    use crate::vm::TIME_SNAPSHOT;

    let mut m = Module::with_crate_item("fluxion", ["time"])?;

    m.function("delta_time", || TIME_SNAPSHOT.load_dt()).build()?;
    m.function("elapsed",    || TIME_SNAPSHOT.load_elapsed()).build()?;
    m.function("frame",      || TIME_SNAPSHOT.load_frame() as i64).build()?;

    Ok(m)
}

/// Input module: `fluxion::input`
pub fn build_input_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["input"])?;

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

/// Viewport module: `fluxion::viewport`
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

/// Collect all engine modules for installation into a Rune context.
pub fn all_modules() -> anyhow::Result<Vec<Module>> {
    Ok(vec![
        build_debug_module()?,
        build_time_module()?,
        build_input_module()?,
        build_viewport_module()?,
    ])
}
