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

/// Collect all engine modules for installation into a Rune context.
pub fn all_modules() -> anyhow::Result<Vec<Module>> {
    Ok(vec![
        build_debug_module()?,
        build_time_module()?,
        build_input_module()?,
    ])
}
