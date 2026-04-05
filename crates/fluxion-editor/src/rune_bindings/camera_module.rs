// ============================================================
// camera_module.rs — fluxion::camera Rune module
//
// Exposes full camera control to Rune panel/game scripts.
//
// Read functions:  query the active camera component directly.
// Write functions: queue a PendingEdit on the Camera component
//                 (applied by host.rs after the Rune call).
// Math functions:  use a per-frame CameraSnapshot pushed by main.rs.
// ============================================================

use std::cell::Cell;
use std::ptr::NonNull;

use rune::Module;

use fluxion_core::{
    ECSWorld, Camera,
    components::camera::{ClearFlags, ProjectionMode},
};

// ── CameraSnapshot (pushed each frame by main.rs) ─────────────────────────────

/// Lightweight snapshot of camera matrices for use in pure math functions.
#[derive(Clone, Copy)]
pub struct CameraSnapshot {
    pub view_proj:     [[f32; 4]; 4],
    pub inv_view_proj: [[f32; 4]; 4],
    pub position:      [f32; 3],
    pub viewport_w:    u32,
    pub viewport_h:    u32,
}

impl CameraSnapshot {
    pub fn identity() -> Self {
        let id = glam::Mat4::IDENTITY.to_cols_array_2d();
        Self {
            view_proj:     id,
            inv_view_proj: id,
            position:      [0.0; 3],
            viewport_w:    1280,
            viewport_h:    720,
        }
    }
}

thread_local! {
    static CAMERA_SNAPSHOT: Cell<CameraSnapshot> = Cell::new(CameraSnapshot::identity());
    static WORLD_PTR: Cell<Option<NonNull<ECSWorld>>> = Cell::new(None);
}

/// Called by main.rs each frame after rendering to push fresh matrices.
pub fn set_camera_snapshot(snap: CameraSnapshot) {
    CAMERA_SNAPSHOT.with(|c| c.set(snap));
}

/// Called alongside world_module::set_world_context so camera reads work.
pub fn set_camera_world(world: &ECSWorld) {
    WORLD_PTR.with(|c| c.set(Some(NonNull::from(world))));
}

/// Called after the Rune panel to clear the pointer.
pub fn clear_camera_world() {
    WORLD_PTR.with(|c| c.set(None));
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn with_cam<R>(mut f: impl FnMut(&Camera) -> R) -> Option<R> {
    let ptr = WORLD_PTR.with(|c| c.get())?;
    // SAFETY: pointer is valid for the panel call duration.
    let world = unsafe { ptr.as_ref() };
    let mut result = None;
    world.query_active::<&Camera, _>(|_, cam| {
        if cam.is_active && result.is_none() {
            result = Some(f(cam));
        }
    });
    result
}

fn with_cam_entity<R>(mut f: impl FnMut(fluxion_core::EntityId, &Camera) -> R) -> Option<R> {
    let ptr = WORLD_PTR.with(|c| c.get())?;
    let world = unsafe { ptr.as_ref() };
    let mut result = None;
    world.query_active::<&Camera, _>(|id, cam| {
        if cam.is_active && result.is_none() {
            result = Some(f(id, cam));
        }
    });
    result
}

fn snap() -> CameraSnapshot {
    CAMERA_SNAPSHOT.with(|c| c.get())
}

// ── Pending edit queue for camera writes ──────────────────────────────────────
// We re-use the world_module PendingEdit queue via a direct import of queue_edit.
// Since queue_edit is private, we replicate the PENDING logic here but target
// the "Camera" component by name through the reflect system.

use fluxion_core::reflect::ReflectValue;

thread_local! {
    static CAM_PENDING: std::cell::RefCell<Vec<CamEdit>> =
        std::cell::RefCell::new(Vec::new());
}

pub struct CamEdit {
    pub entity: fluxion_core::EntityId,
    pub field:  String,
    pub value:  ReflectValue,
}

pub fn drain_camera_edits() -> Vec<CamEdit> {
    CAM_PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

fn queue_cam(field: &str, value: ReflectValue) {
    let Some(id) = with_cam_entity(|id, _| id) else { return };
    CAM_PENDING.with(|p| p.borrow_mut().push(CamEdit {
        entity: id,
        field:  field.to_string(),
        value,
    }));
}

// ── Module builder ────────────────────────────────────────────────────────────

pub fn build_camera_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["camera"])?;

    // ── Read: projection ──────────────────────────────────────────────────────

    m.function("get_fov", || -> f64 {
        with_cam(|c| c.fov as f64).unwrap_or(70.0)
    }).build()?;

    m.function("get_near", || -> f64 {
        with_cam(|c| c.near as f64).unwrap_or(0.1)
    }).build()?;

    m.function("get_far", || -> f64 {
        with_cam(|c| c.far as f64).unwrap_or(1000.0)
    }).build()?;

    m.function("is_orthographic", || -> bool {
        with_cam(|c| c.projection_mode == ProjectionMode::Orthographic).unwrap_or(false)
    }).build()?;

    m.function("get_ortho_size", || -> f64 {
        with_cam(|c| c.ortho_size as f64).unwrap_or(5.0)
    }).build()?;

    // ── Read: culling & order ─────────────────────────────────────────────────

    m.function("get_culling_mask", || -> i64 {
        with_cam(|c| c.culling_mask as i64).unwrap_or(0xFFFF_FFFFi64)
    }).build()?;

    m.function("get_depth", || -> i64 {
        with_cam(|c| c.depth as i64).unwrap_or(0)
    }).build()?;

    // ── Read: viewport ────────────────────────────────────────────────────────

    m.function("get_viewport_rect", || -> Vec<f64> {
        with_cam(|c| {
            let r = c.viewport_rect;
            vec![r[0] as f64, r[1] as f64, r[2] as f64, r[3] as f64]
        }).unwrap_or_else(|| vec![0.0, 0.0, 1.0, 1.0])
    }).build()?;

    // ── Read: clear ───────────────────────────────────────────────────────────

    m.function("get_clear_flags", || -> String {
        with_cam(|c| c.clear_flags.as_str().to_string())
            .unwrap_or_else(|| "Skybox".to_string())
    }).build()?;

    m.function("get_background_color", || -> Vec<f64> {
        with_cam(|c| {
            let b = c.background_color;
            vec![b[0] as f64, b[1] as f64, b[2] as f64, b[3] as f64]
        }).unwrap_or_else(|| vec![0.1, 0.1, 0.1, 1.0])
    }).build()?;

    // ── Read: quality ─────────────────────────────────────────────────────────

    m.function("allow_hdr", || -> bool {
        with_cam(|c| c.allow_hdr).unwrap_or(true)
    }).build()?;

    m.function("allow_msaa", || -> bool {
        with_cam(|c| c.allow_msaa).unwrap_or(false)
    }).build()?;

    // ── Read: physical camera ─────────────────────────────────────────────────

    m.function("use_physical", || -> bool {
        with_cam(|c| c.use_physical).unwrap_or(false)
    }).build()?;

    m.function("get_focal_length", || -> f64 {
        with_cam(|c| c.focal_length as f64).unwrap_or(50.0)
    }).build()?;

    m.function("get_sensor_size", || -> Vec<f64> {
        with_cam(|c| vec![c.sensor_size[0] as f64, c.sensor_size[1] as f64])
            .unwrap_or_else(|| vec![36.0, 24.0])
    }).build()?;

    m.function("get_lens_shift", || -> Vec<f64> {
        with_cam(|c| vec![c.lens_shift[0] as f64, c.lens_shift[1] as f64])
            .unwrap_or_else(|| vec![0.0, 0.0])
    }).build()?;

    // ── Write: projection ─────────────────────────────────────────────────────

    m.function("set_fov", |v: f64| {
        queue_cam("fov", ReflectValue::F32(v as f32));
    }).build()?;

    m.function("set_near", |v: f64| {
        queue_cam("near", ReflectValue::F32(v as f32));
    }).build()?;

    m.function("set_far", |v: f64| {
        queue_cam("far", ReflectValue::F32(v as f32));
    }).build()?;

    m.function("set_orthographic", |v: bool| {
        let mode = if v { "Orthographic" } else { "Perspective" };
        queue_cam("projection", ReflectValue::Enum(mode.to_string()));
    }).build()?;

    m.function("set_ortho_size", |v: f64| {
        queue_cam("ortho_size", ReflectValue::F32(v as f32));
    }).build()?;

    // ── Write: culling & order ────────────────────────────────────────────────

    m.function("set_culling_mask", |v: i64| {
        queue_cam("culling_mask", ReflectValue::U32(v as u32));
    }).build()?;

    m.function("set_depth", |v: i64| {
        queue_cam("depth", ReflectValue::F32(v as f32));
    }).build()?;

    // ── Write: viewport ───────────────────────────────────────────────────────
    // viewport_rect is a [f32;4] field not covered by reflect; apply directly
    // via a separate CamEditDirect path handled in host.rs flush.
    m.function("set_viewport_rect", |xywh: Vec<f64>| {
        if xywh.len() >= 4 {
            // Stored as 4 separate F32 sub-fields via naming convention.
            // host.rs handles "viewport_rect" specially.
            queue_cam("viewport_rect_x", ReflectValue::F32(xywh[0] as f32));
            queue_cam("viewport_rect_y", ReflectValue::F32(xywh[1] as f32));
            queue_cam("viewport_rect_w", ReflectValue::F32(xywh[2] as f32));
            queue_cam("viewport_rect_h", ReflectValue::F32(xywh[3] as f32));
        }
    }).build()?;

    // ── Write: clear ──────────────────────────────────────────────────────────

    m.function("set_clear_flags", |s: String| {
        queue_cam("clear_flags", ReflectValue::Enum(s));
    }).build()?;

    m.function("set_background_color", |rgba: Vec<f64>| {
        if rgba.len() >= 4 {
            queue_cam("background_color", ReflectValue::Color4([
                rgba[0] as f32, rgba[1] as f32, rgba[2] as f32, rgba[3] as f32,
            ]));
        }
    }).build()?;

    // ── Write: quality ────────────────────────────────────────────────────────

    m.function("set_allow_hdr", |v: bool| {
        queue_cam("allow_hdr", ReflectValue::Bool(v));
    }).build()?;

    m.function("set_allow_msaa", |v: bool| {
        queue_cam("allow_msaa", ReflectValue::Bool(v));
    }).build()?;

    // ── Write: physical ───────────────────────────────────────────────────────

    m.function("set_use_physical", |v: bool| {
        queue_cam("use_physical", ReflectValue::Bool(v));
    }).build()?;

    m.function("set_focal_length", |v: f64| {
        queue_cam("focal_length", ReflectValue::F32(v as f32));
    }).build()?;

    m.function("set_sensor_size", |wh: Vec<f64>| {
        if wh.len() >= 2 {
            queue_cam("sensor_size_w", ReflectValue::F32(wh[0] as f32));
            queue_cam("sensor_size_h", ReflectValue::F32(wh[1] as f32));
        }
    }).build()?;

    m.function("set_lens_shift", |xy: Vec<f64>| {
        if xy.len() >= 2 {
            queue_cam("lens_shift_x", ReflectValue::F32(xy[0] as f32));
            queue_cam("lens_shift_y", ReflectValue::F32(xy[1] as f32));
        }
    }).build()?;

    // ── Math: screen ↔ world ──────────────────────────────────────────────────

    m.function("screen_to_world", |screen_xy: Vec<f64>, depth: f64| -> Vec<f64> {
        let s = snap();
        let inv_vp = glam::Mat4::from_cols_array_2d(&s.inv_view_proj);
        let world = Camera::screen_to_world(
            screen_xy.first().copied().unwrap_or(0.0) as f32,
            screen_xy.get(1).copied().unwrap_or(0.0) as f32,
            depth as f32,
            inv_vp,
            s.viewport_w,
            s.viewport_h,
        );
        vec![world.x as f64, world.y as f64, world.z as f64]
    }).build()?;

    m.function("world_to_screen", |world_xyz: Vec<f64>| -> Vec<f64> {
        let s = snap();
        let vp = glam::Mat4::from_cols_array_2d(&s.view_proj);
        let pos = glam::Vec3::new(
            world_xyz.first().copied().unwrap_or(0.0) as f32,
            world_xyz.get(1).copied().unwrap_or(0.0) as f32,
            world_xyz.get(2).copied().unwrap_or(0.0) as f32,
        );
        let sc = Camera::world_to_screen(pos, vp, s.viewport_w, s.viewport_h);
        vec![sc.x as f64, sc.y as f64, sc.z as f64]
    }).build()?;

    m.function("screen_point_to_ray", |screen_xy: Vec<f64>| -> Vec<f64> {
        let s = snap();
        let inv_vp = glam::Mat4::from_cols_array_2d(&s.inv_view_proj);
        let cam_pos = glam::Vec3::from_array(s.position);
        let (origin, dir) = Camera::screen_point_to_ray(
            screen_xy.first().copied().unwrap_or(0.0) as f32,
            screen_xy.get(1).copied().unwrap_or(0.0) as f32,
            inv_vp,
            cam_pos,
            s.viewport_w,
            s.viewport_h,
        );
        vec![
            origin.x as f64, origin.y as f64, origin.z as f64,
            dir.x    as f64, dir.y    as f64, dir.z    as f64,
        ]
    }).build()?;

    Ok(m)
}
