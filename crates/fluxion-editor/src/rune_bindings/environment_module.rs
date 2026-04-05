// ============================================================
// environment_module.rs — fluxion::environment Rune module
//
// Unity-style post-processing and environment control via Rune scripts.
//
// Read functions:  query the active Environment component.
// Write functions: queue a PendingEdit on the Environment component
//                 (applied by host.rs after the Rune call).
//
// All numeric args/returns are f64 (Rune's native float type).
// ============================================================

use std::cell::Cell;
use std::ptr::NonNull;

use rune::Module;

use fluxion_core::{ECSWorld, Environment};
use fluxion_core::components::environment::{ToneMapMode, FogMode, BackgroundMode};

// ── Thread-local world pointer ────────────────────────────────────────────────

thread_local! {
    static WORLD_PTR: Cell<Option<NonNull<ECSWorld>>> = Cell::new(None);
}

pub fn set_environment_world(world: &ECSWorld) {
    WORLD_PTR.with(|c| c.set(Some(NonNull::from(world))));
}

pub fn clear_environment_world() {
    WORLD_PTR.with(|c| c.set(None));
}

// ── Pending edits queue ───────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct EnvEdit {
    pub field: String,
    pub value: EnvEditValue,
}

#[derive(Clone, Debug)]
pub enum EnvEditValue {
    F32(f32),
    Bool(bool),
    Str(String),
    Color([f32; 3]),
    U32(u32),
}

thread_local! {
    static PENDING_EDITS: std::cell::RefCell<Vec<EnvEdit>> = std::cell::RefCell::new(Vec::new());
}

fn push_edit(field: &str, value: EnvEditValue) {
    PENDING_EDITS.with(|q| q.borrow_mut().push(EnvEdit { field: field.to_string(), value }));
}

pub fn drain_environment_edits() -> Vec<EnvEdit> {
    PENDING_EDITS.with(|q| q.borrow_mut().drain(..).collect())
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn with_env<R>(mut f: impl FnMut(&Environment) -> R) -> Option<R> {
    let ptr = WORLD_PTR.with(|c| c.get())?;
    let world = unsafe { ptr.as_ref() };
    let mut result = None;
    world.query_active::<&Environment, _>(|_, env| {
        if result.is_none() {
            result = Some(f(env));
        }
    });
    result
}

// ── Rune module builder ───────────────────────────────────────────────────────

pub fn build_environment_module() -> Result<Module, rune::ContextError> {
    let mut m = Module::with_crate_item("fluxion", ["environment"])?;

    // ── Sky ───────────────────────────────────────────────────────────────────
    m.function("get_sky_mode", || -> String {
        with_env(|e| e.sky.mode.as_str().to_string()).unwrap_or_else(|| "Gradient".to_string())
    }).build()?;

    m.function("set_sky_mode", |s: String| {
        push_edit("sky_mode", EnvEditValue::Str(s));
    }).build()?;

    m.function("get_sky_solid_color", || -> Vec<f64> {
        with_env(|e| vec![e.sky.solid_color[0] as f64, e.sky.solid_color[1] as f64, e.sky.solid_color[2] as f64])
            .unwrap_or_else(|| vec![0.05, 0.07, 0.10])
    }).build()?;

    m.function("set_sky_solid_color", |r: f64, g: f64, b: f64| {
        push_edit("sky_solid_color", EnvEditValue::Color([r as f32, g as f32, b as f32]));
    }).build()?;

    m.function("get_sky_horizon_color", || -> Vec<f64> {
        with_env(|e| vec![e.sky.horizon_color[0] as f64, e.sky.horizon_color[1] as f64, e.sky.horizon_color[2] as f64])
            .unwrap_or_else(|| vec![0.6, 0.75, 1.0])
    }).build()?;

    m.function("set_sky_horizon_color", |r: f64, g: f64, b: f64| {
        push_edit("sky_horizon_color", EnvEditValue::Color([r as f32, g as f32, b as f32]));
    }).build()?;

    m.function("get_sky_zenith_color", || -> Vec<f64> {
        with_env(|e| vec![e.sky.zenith_color[0] as f64, e.sky.zenith_color[1] as f64, e.sky.zenith_color[2] as f64])
            .unwrap_or_else(|| vec![0.1, 0.3, 0.8])
    }).build()?;

    m.function("set_sky_zenith_color", |r: f64, g: f64, b: f64| {
        push_edit("sky_zenith_color", EnvEditValue::Color([r as f32, g as f32, b as f32]));
    }).build()?;

    m.function("get_sky_sun_intensity", || -> f64 {
        with_env(|e| e.sky.sun_intensity as f64).unwrap_or(20.0)
    }).build()?;

    m.function("set_sky_sun_intensity", |v: f64| {
        push_edit("sky_sun_intensity", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("get_sky_sun_size", || -> f64 {
        with_env(|e| e.sky.sun_size as f64).unwrap_or(0.02)
    }).build()?;

    m.function("set_sky_sun_size", |v: f64| {
        push_edit("sky_sun_size", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("get_sky_sun_elevation", || -> f64 {
        with_env(|e| e.sky.sun_elevation as f64).unwrap_or(45.0)
    }).build()?;

    m.function("set_sky_sun_elevation", |v: f64| {
        push_edit("sky_sun_elevation", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("get_sky_sun_azimuth", || -> f64 {
        with_env(|e| e.sky.sun_azimuth as f64).unwrap_or(180.0)
    }).build()?;

    m.function("set_sky_sun_azimuth", |v: f64| {
        push_edit("sky_sun_azimuth", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("get_sky_turbidity", || -> f64 {
        with_env(|e| e.sky.turbidity as f64).unwrap_or(2.0)
    }).build()?;

    m.function("set_sky_turbidity", |v: f64| {
        push_edit("sky_turbidity", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("get_sky_rayleigh", || -> f64 {
        with_env(|e| e.sky.rayleigh as f64).unwrap_or(1.0)
    }).build()?;

    m.function("set_sky_rayleigh", |v: f64| {
        push_edit("sky_rayleigh", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("get_sky_mie_coefficient", || -> f64 {
        with_env(|e| e.sky.mie_coefficient as f64).unwrap_or(0.005)
    }).build()?;

    m.function("set_sky_mie_coefficient", |v: f64| {
        push_edit("sky_mie_coeff", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("get_sky_mie_directional_g", || -> f64 {
        with_env(|e| e.sky.mie_directional_g as f64).unwrap_or(0.8)
    }).build()?;

    m.function("set_sky_mie_directional_g", |v: f64| {
        push_edit("sky_mie_dir_g", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("get_sky_panorama_path", || -> String {
        with_env(|e| e.sky.skybox_path.clone()).unwrap_or_default()
    }).build()?;

    m.function("set_sky_panorama_path", |path: String| {
        push_edit("sky_panorama_path", EnvEditValue::Str(path));
    }).build()?;

    // ── Ambient ──────────────────────────────────────────────────────────────
    m.function("get_ambient_color", || -> Vec<f64> {
        with_env(|e| vec![e.ambient.color[0] as f64, e.ambient.color[1] as f64, e.ambient.color[2] as f64])
            .unwrap_or_else(|| vec![0.27, 0.27, 0.35])
    }).build()?;

    m.function("get_ambient_intensity", || -> f64 {
        with_env(|e| e.ambient.intensity as f64).unwrap_or(0.5)
    }).build()?;

    m.function("set_ambient_color", |r: f64, g: f64, b: f64| {
        push_edit("ambient_color", EnvEditValue::Color([r as f32, g as f32, b as f32]));
    }).build()?;

    m.function("set_ambient_intensity", |v: f64| {
        push_edit("ambient_intensity", EnvEditValue::F32(v as f32));
    }).build()?;

    // ── Fog ──────────────────────────────────────────────────────────────────
    m.function("get_fog_enabled", || -> bool {
        with_env(|e| e.fog.enabled).unwrap_or(false)
    }).build()?;

    m.function("get_fog_color", || -> Vec<f64> {
        with_env(|e| vec![e.fog.color[0] as f64, e.fog.color[1] as f64, e.fog.color[2] as f64])
            .unwrap_or_else(|| vec![0.1, 0.1, 0.15])
    }).build()?;

    m.function("get_fog_mode", || -> String {
        with_env(|e| e.fog.mode.as_str().to_string()).unwrap_or_else(|| "Exponential".to_string())
    }).build()?;

    m.function("get_fog_density", || -> f64 {
        with_env(|e| e.fog.density as f64).unwrap_or(0.008)
    }).build()?;

    m.function("get_fog_near", || -> f64 {
        with_env(|e| e.fog.near as f64).unwrap_or(10.0)
    }).build()?;

    m.function("get_fog_far", || -> f64 {
        with_env(|e| e.fog.far as f64).unwrap_or(100.0)
    }).build()?;

    m.function("set_fog_enabled", |v: bool| {
        push_edit("fog_enabled", EnvEditValue::Bool(v));
    }).build()?;

    m.function("set_fog_color", |r: f64, g: f64, b: f64| {
        push_edit("fog_color", EnvEditValue::Color([r as f32, g as f32, b as f32]));
    }).build()?;

    m.function("set_fog_mode", |s: String| {
        push_edit("fog_mode", EnvEditValue::Str(s));
    }).build()?;

    m.function("set_fog_density", |v: f64| {
        push_edit("fog_density", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("set_fog_near", |v: f64| {
        push_edit("fog_near", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("set_fog_far", |v: f64| {
        push_edit("fog_far", EnvEditValue::F32(v as f32));
    }).build()?;

    // ── Tonemap ───────────────────────────────────────────────────────────────
    m.function("get_tone_mode", || -> String {
        with_env(|e| e.tonemap.mode.as_str().to_string()).unwrap_or_else(|| "Aces".to_string())
    }).build()?;

    m.function("get_exposure", || -> f64 {
        with_env(|e| e.tonemap.exposure as f64).unwrap_or(1.2)
    }).build()?;

    m.function("set_tone_mode", |s: String| {
        push_edit("tone_mode", EnvEditValue::Str(s));
    }).build()?;

    m.function("set_exposure", |v: f64| {
        push_edit("exposure", EnvEditValue::F32(v as f32));
    }).build()?;

    // ── Bloom ─────────────────────────────────────────────────────────────────
    m.function("get_bloom_enabled", || -> bool {
        with_env(|e| e.bloom.enabled).unwrap_or(true)
    }).build()?;

    m.function("get_bloom_threshold", || -> f64 {
        with_env(|e| e.bloom.threshold as f64).unwrap_or(0.8)
    }).build()?;

    m.function("get_bloom_strength", || -> f64 {
        with_env(|e| e.bloom.strength as f64).unwrap_or(0.5)
    }).build()?;

    m.function("get_bloom_soft_knee", || -> f64 {
        with_env(|e| e.bloom.soft_knee as f64).unwrap_or(0.5)
    }).build()?;

    m.function("get_bloom_blur_passes", || -> i64 {
        with_env(|e| e.bloom.blur_passes as i64).unwrap_or(4)
    }).build()?;

    m.function("set_bloom_enabled", |v: bool| {
        push_edit("bloom_enabled", EnvEditValue::Bool(v));
    }).build()?;

    m.function("set_bloom_threshold", |v: f64| {
        push_edit("bloom_threshold", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("set_bloom_strength", |v: f64| {
        push_edit("bloom_strength", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("set_bloom_soft_knee", |v: f64| {
        push_edit("bloom_soft_knee", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("set_bloom_blur_passes", |v: i64| {
        push_edit("bloom_blur_passes", EnvEditValue::U32(v.max(1).min(8) as u32));
    }).build()?;

    // ── SSAO ──────────────────────────────────────────────────────────────────
    m.function("get_ssao_enabled", || -> bool {
        with_env(|e| e.ssao.enabled).unwrap_or(false)
    }).build()?;

    m.function("get_ssao_radius", || -> f64 {
        with_env(|e| e.ssao.radius as f64).unwrap_or(0.5)
    }).build()?;

    m.function("get_ssao_bias", || -> f64 {
        with_env(|e| e.ssao.bias as f64).unwrap_or(0.025)
    }).build()?;

    m.function("get_ssao_intensity", || -> f64 {
        with_env(|e| e.ssao.intensity as f64).unwrap_or(1.0)
    }).build()?;

    m.function("set_ssao_enabled", |v: bool| {
        push_edit("ssao_enabled", EnvEditValue::Bool(v));
    }).build()?;

    m.function("set_ssao_radius", |v: f64| {
        push_edit("ssao_radius", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("set_ssao_bias", |v: f64| {
        push_edit("ssao_bias", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("set_ssao_intensity", |v: f64| {
        push_edit("ssao_intensity", EnvEditValue::F32(v as f32));
    }).build()?;

    // ── DoF (stored, no pass yet) ─────────────────────────────────────────────
    m.function("get_dof_enabled", || -> bool {
        with_env(|e| e.dof.enabled).unwrap_or(false)
    }).build()?;

    m.function("get_dof_focus_dist", || -> f64 {
        with_env(|e| e.dof.focus_dist as f64).unwrap_or(10.0)
    }).build()?;

    m.function("get_dof_aperture", || -> f64 {
        with_env(|e| e.dof.aperture as f64).unwrap_or(0.025)
    }).build()?;

    m.function("get_dof_max_blur", || -> f64 {
        with_env(|e| e.dof.max_blur as f64).unwrap_or(10.0)
    }).build()?;

    m.function("set_dof_enabled", |v: bool| {
        push_edit("dof_enabled", EnvEditValue::Bool(v));
    }).build()?;

    m.function("set_dof_focus_dist", |v: f64| {
        push_edit("dof_focus_dist", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("set_dof_aperture", |v: f64| {
        push_edit("dof_aperture", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("set_dof_max_blur", |v: f64| {
        push_edit("dof_max_blur", EnvEditValue::F32(v as f32));
    }).build()?;

    // ── Vignette ──────────────────────────────────────────────────────────────
    m.function("get_vignette_enabled", || -> bool {
        with_env(|e| e.vignette.enabled).unwrap_or(false)
    }).build()?;

    m.function("get_vignette_intensity", || -> f64 {
        with_env(|e| e.vignette.intensity as f64).unwrap_or(0.3)
    }).build()?;

    m.function("get_vignette_roundness", || -> f64 {
        with_env(|e| e.vignette.roundness as f64).unwrap_or(0.8)
    }).build()?;

    m.function("set_vignette_enabled", |v: bool| {
        push_edit("vignette_enabled", EnvEditValue::Bool(v));
    }).build()?;

    m.function("set_vignette_intensity", |v: f64| {
        push_edit("vignette_intensity", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("set_vignette_roundness", |v: f64| {
        push_edit("vignette_roundness", EnvEditValue::F32(v as f32));
    }).build()?;

    // ── Film ──────────────────────────────────────────────────────────────────
    m.function("get_chromatic_aberration", || -> f64 {
        with_env(|e| e.film.chromatic_aberration as f64).unwrap_or(0.0)
    }).build()?;

    m.function("get_film_grain", || -> f64 {
        with_env(|e| e.film.film_grain as f64).unwrap_or(0.0)
    }).build()?;

    m.function("set_chromatic_aberration", |v: f64| {
        push_edit("chromatic_aberration", EnvEditValue::F32(v as f32));
    }).build()?;

    m.function("set_film_grain", |v: f64| {
        push_edit("film_grain", EnvEditValue::F32(v as f32));
    }).build()?;

    Ok(m)
}
