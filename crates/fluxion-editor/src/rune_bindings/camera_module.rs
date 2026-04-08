// ============================================================
// camera_module.rs — Rune camera bindings
//
// Exposes two complementary APIs to Rune scripts:
//
//   fluxion::camera  — low-level entity-addressed bindings
//                      (editor panel scripts, tooling)
//
//   fluxion::Camera  — Unity-style static API for game scripts
//                      Camera::main()           → i64
//                      Camera::field_of_view(id)
//                      Camera::near_clip_plane(id)
//                      Camera::world_to_screen_point(id, xyz)
//                      … etc.
//
// Read functions:  query the Camera component directly.
// Write functions: queue a CamEdit (applied by host.rs after the call).
// Math functions:  build per-entity view/projection on-the-fly from
//                  Transform + Camera, or fall back to the per-frame
//                  CameraSnapshot for the snapshot-based helpers.
// ============================================================

use std::cell::Cell;
use std::ptr::NonNull;

use rune::Module;

use fluxion_core::{
    ECSWorld, Camera, Transform, EntityId,
    components::camera::ProjectionMode,
};
use crate::rune_bindings::world_module::entity_from_id;

// ── CameraSnapshot (pushed each frame by main.rs) ─────────────────────────────

/// Lightweight snapshot of camera matrices for use in pure math functions.
#[derive(Clone, Copy)]
pub struct CameraSnapshot {
    pub view_proj:     [[f32; 4]; 4],
    pub inv_view_proj: [[f32; 4]; 4],
    #[allow(dead_code)]
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

/// Returns the world pointer, if set.
fn world_ref() -> Option<&'static ECSWorld> {
    let ptr = WORLD_PTR.with(|c| c.get())?;
    // SAFETY: pointer is valid for the duration of the Rune call.
    Some(unsafe { ptr.as_ref() })
}

/// Run `f` with the first active Camera (main-camera shortcut).
#[allow(dead_code)]
fn with_cam<R>(mut f: impl FnMut(&Camera) -> R) -> Option<R> {
    let world = world_ref()?;
    let mut result = None;
    // Prefer is_main; fall back to first active.
    world.query_active::<&Camera, _>(|_, cam| {
        if cam.is_main && cam.is_active && result.is_none() {
            result = Some(f(cam));
        }
    });
    if result.is_none() {
        world.query_active::<&Camera, _>(|_, cam| {
            if cam.is_active && result.is_none() {
                result = Some(f(cam));
            }
        });
    }
    result
}

/// Run `f` with the first active Camera, also returning its EntityId.
fn with_cam_entity<R>(mut f: impl FnMut(fluxion_core::EntityId, &Camera) -> R) -> Option<R> {
    let world = world_ref()?;
    let mut result = None;
    world.query_active::<&Camera, _>(|id, cam| {
        if cam.is_main && cam.is_active && result.is_none() {
            result = Some(f(id, cam));
        }
    });
    if result.is_none() {
        world.query_active::<&Camera, _>(|id, cam| {
            if cam.is_active && result.is_none() {
                result = Some(f(id, cam));
            }
        });
    }
    result
}

/// Run `f` with the Camera on a specific entity.
fn with_cam_by_id<R>(entity_id: i64, mut f: impl FnMut(&Camera) -> R) -> Option<R> {
    let world = world_ref()?;
    let eid = entity_from_id(entity_id)?;
    world.get_component::<Camera>(eid).map(|c| f(&*c))
}

/// Run `f` with both Transform and Camera for a specific entity.
fn with_cam_transform<R>(
    entity_id: i64,
    mut f: impl FnMut(&Transform, &Camera) -> R,
) -> Option<R> {
    let world = world_ref()?;
    let eid = entity_from_id(entity_id)?;
    let cam = world.get_component::<Camera>(eid)?;
    let tr  = world.get_component::<Transform>(eid)?;
    Some(f(&*tr, &*cam))
}

fn snap() -> CameraSnapshot {
    CAMERA_SNAPSHOT.with(|c| c.get())
}

/// Build per-entity inv_view_proj and position from ECS components.
/// Used by per-entity math functions so they don't depend on the snapshot.
fn cam_matrices(entity_id: i64) -> Option<(glam::Mat4, glam::Mat4, glam::Vec3, u32, u32)> {
    with_cam_transform(entity_id, |tr, cam| {
        let s = snap();
        let (w, h) = (s.viewport_w, s.viewport_h);
        let view = glam::Mat4::look_at_rh(
            tr.world_position,
            tr.world_position + tr.world_forward(),
            tr.world_up(),
        );
        let proj     = cam.projection_matrix_ex(w, h);
        let vp       = proj * view;
        let inv_vp   = vp.inverse();
        (vp, inv_vp, tr.world_position, w, h)
    })
}

// ── Pending edit queue ────────────────────────────────────────────────────────

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

/// Queue an edit for the main camera (no-arg legacy path).
#[allow(dead_code)]
fn queue_cam(field: &str, value: ReflectValue) {
    let Some(id) = with_cam_entity(|id, _| id) else { return };
    queue_cam_id(id, field, value);
}

/// Queue an edit for a specific camera entity.
fn queue_cam_id(entity: fluxion_core::EntityId, field: &str, value: ReflectValue) {
    CAM_PENDING.with(|p| p.borrow_mut().push(CamEdit {
        entity,
        field: field.to_string(),
        value,
    }));
}

/// Resolve an entity_id (i64) to an EntityId, queue an edit if valid.
fn queue_by_id(entity_id: i64, field: &str, value: ReflectValue) {
    if let Some(eid) = entity_from_id(entity_id) {
        if world_ref().map(|w| w.get_component::<Camera>(eid).is_some()).unwrap_or(false) {
            queue_cam_id(eid, field, value);
        }
    }
}

// ── Main camera ID helper (shared by both modules) ───────────────────────────

fn main_camera_id() -> i64 {
    with_cam_entity(|id, _| id.to_bits() as i64).unwrap_or(-1)
}

// ── Module builder ────────────────────────────────────────────────────────────

pub fn build_camera_module() -> anyhow::Result<Vec<Module>> {
    Ok(vec![
        build_low_level_module()?,
        build_unity_style_module()?,
    ])
}

// ── fluxion::camera — low-level entity-addressed module ──────────────────────

fn build_low_level_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["camera"])?;

    // ── Manager queries ───────────────────────────────────────────────────────

    m.function("get_main", || -> i64 { main_camera_id() }).build()?;

    m.function("get_all", || -> Vec<i64> {
        let Some(world) = world_ref() else { return vec![] };
        let mut ids: Vec<(i32, i64)> = Vec::new();
        world.query_active::<&Camera, _>(|id, cam| {
            ids.push((cam.depth, id.to_bits() as i64));
        });
        ids.sort_by_key(|(d, _)| *d);
        ids.into_iter().map(|(_, id)| id).collect()
    }).build()?;

    m.function("set_main", |entity_id: i64| {
        let Some(eid) = entity_from_id(entity_id) else { return };
        let Some(world) = world_ref() else { return };
        let others: Vec<EntityId> = {
            let mut v = Vec::new();
            world.query_active::<&Camera, _>(|id, _| {
                if id != eid { v.push(id); }
            });
            v
        };
        for other in others {
            queue_cam_id(other, "is_main", ReflectValue::Bool(false));
        }
        queue_cam_id(eid, "is_main", ReflectValue::Bool(true));
    }).build()?;

    m.function("find_by_tag", |tag: String| -> i64 {
        let Some(world) = world_ref() else { return -1 };
        let mut found = -1i64;
        world.query_active::<&Camera, _>(|id, cam| {
            if cam.tag == tag && found == -1 {
                found = id.to_bits() as i64;
            }
        });
        found
    }).build()?;

    // ── Entity-addressed: FOV / projection ────────────────────────────────────

    m.function("get_fov", |entity_id: i64| -> f64 {
        with_cam_by_id(entity_id, |c| c.fov as f64).unwrap_or(70.0)
    }).build()?;

    m.function("set_fov", |entity_id: i64, v: f64| {
        queue_by_id(entity_id, "fov", ReflectValue::F32(v as f32));
    }).build()?;

    m.function("get_near", |entity_id: i64| -> f64 {
        with_cam_by_id(entity_id, |c| c.near as f64).unwrap_or(0.1)
    }).build()?;

    m.function("set_near", |entity_id: i64, v: f64| {
        queue_by_id(entity_id, "near", ReflectValue::F32(v as f32));
    }).build()?;

    m.function("get_far", |entity_id: i64| -> f64 {
        with_cam_by_id(entity_id, |c| c.far as f64).unwrap_or(1000.0)
    }).build()?;

    m.function("set_far", |entity_id: i64, v: f64| {
        queue_by_id(entity_id, "far", ReflectValue::F32(v as f32));
    }).build()?;

    m.function("get_projection", |entity_id: i64| -> String {
        with_cam_by_id(entity_id, |c| match c.projection_mode {
            ProjectionMode::Orthographic => "Orthographic".to_string(),
            ProjectionMode::Perspective  => "Perspective".to_string(),
        }).unwrap_or_else(|| "Perspective".to_string())
    }).build()?;

    m.function("set_projection", |entity_id: i64, s: String| {
        queue_by_id(entity_id, "projection_mode", ReflectValue::Enum(s));
    }).build()?;

    m.function("is_orthographic", |entity_id: i64| -> bool {
        with_cam_by_id(entity_id, |c| c.projection_mode == ProjectionMode::Orthographic)
            .unwrap_or(false)
    }).build()?;

    m.function("set_orthographic", |entity_id: i64, v: bool| {
        let mode = if v { "Orthographic" } else { "Perspective" };
        queue_by_id(entity_id, "projection_mode", ReflectValue::Enum(mode.to_string()));
    }).build()?;

    m.function("get_ortho_size", |entity_id: i64| -> f64 {
        with_cam_by_id(entity_id, |c| c.ortho_size as f64).unwrap_or(5.0)
    }).build()?;

    m.function("set_ortho_size", |entity_id: i64, v: f64| {
        queue_by_id(entity_id, "ortho_size", ReflectValue::F32(v as f32));
    }).build()?;

    // ── Entity-addressed: depth & culling ─────────────────────────────────────

    m.function("get_depth", |entity_id: i64| -> i64 {
        with_cam_by_id(entity_id, |c| c.depth as i64).unwrap_or(0)
    }).build()?;

    m.function("set_depth", |entity_id: i64, v: i64| {
        queue_by_id(entity_id, "depth", ReflectValue::I32(v as i32));
    }).build()?;

    m.function("get_culling_mask", |entity_id: i64| -> i64 {
        with_cam_by_id(entity_id, |c| c.culling_mask as i64).unwrap_or(0xFFFF_FFFFi64)
    }).build()?;

    m.function("set_culling_mask", |entity_id: i64, v: i64| {
        queue_by_id(entity_id, "culling_mask", ReflectValue::U32(v as u32));
    }).build()?;

    // ── Entity-addressed: active / main flags ─────────────────────────────────

    m.function("is_active", |entity_id: i64| -> bool {
        with_cam_by_id(entity_id, |c| c.is_active).unwrap_or(false)
    }).build()?;

    m.function("set_active", |entity_id: i64, v: bool| {
        queue_by_id(entity_id, "is_active", ReflectValue::Bool(v));
    }).build()?;

    m.function("is_main", |entity_id: i64| -> bool {
        with_cam_by_id(entity_id, |c| c.is_main).unwrap_or(false)
    }).build()?;

    // ── Entity-addressed: clear flags & background ────────────────────────────

    m.function("get_clear_flags", |entity_id: i64| -> String {
        with_cam_by_id(entity_id, |c| c.clear_flags.as_str().to_string())
            .unwrap_or_else(|| "Skybox".to_string())
    }).build()?;

    m.function("set_clear_flags", |entity_id: i64, s: String| {
        queue_by_id(entity_id, "clear_flags", ReflectValue::Enum(s));
    }).build()?;

    m.function("get_background_color", |entity_id: i64| -> Vec<f64> {
        with_cam_by_id(entity_id, |c| {
            let b = c.background_color;
            vec![b[0] as f64, b[1] as f64, b[2] as f64, b[3] as f64]
        }).unwrap_or_else(|| vec![0.1, 0.1, 0.1, 1.0])
    }).build()?;

    m.function("set_background_color", |entity_id: i64, rgba: Vec<f64>| {
        if rgba.len() >= 4 {
            queue_by_id(entity_id, "background_color", ReflectValue::Color4([
                rgba[0] as f32, rgba[1] as f32, rgba[2] as f32, rgba[3] as f32,
            ]));
        }
    }).build()?;

    // ── Entity-addressed: viewport rect ───────────────────────────────────────

    m.function("get_viewport_rect", |entity_id: i64| -> Vec<f64> {
        with_cam_by_id(entity_id, |c| {
            let r = c.viewport_rect;
            vec![r[0] as f64, r[1] as f64, r[2] as f64, r[3] as f64]
        }).unwrap_or_else(|| vec![0.0, 0.0, 1.0, 1.0])
    }).build()?;

    m.function("set_viewport_rect", |entity_id: i64, xywh: Vec<f64>| {
        if xywh.len() >= 4 {
            queue_by_id(entity_id, "viewport_rect_x", ReflectValue::F32(xywh[0] as f32));
            queue_by_id(entity_id, "viewport_rect_y", ReflectValue::F32(xywh[1] as f32));
            queue_by_id(entity_id, "viewport_rect_w", ReflectValue::F32(xywh[2] as f32));
            queue_by_id(entity_id, "viewport_rect_h", ReflectValue::F32(xywh[3] as f32));
        }
    }).build()?;

    // ── Entity-addressed: quality flags ───────────────────────────────────────

    m.function("allow_hdr", |entity_id: i64| -> bool {
        with_cam_by_id(entity_id, |c| c.allow_hdr).unwrap_or(true)
    }).build()?;

    m.function("set_allow_hdr", |entity_id: i64, v: bool| {
        queue_by_id(entity_id, "allow_hdr", ReflectValue::Bool(v));
    }).build()?;

    m.function("allow_msaa", |entity_id: i64| -> bool {
        with_cam_by_id(entity_id, |c| c.allow_msaa).unwrap_or(false)
    }).build()?;

    m.function("set_allow_msaa", |entity_id: i64, v: bool| {
        queue_by_id(entity_id, "allow_msaa", ReflectValue::Bool(v));
    }).build()?;

    // ── Entity-addressed: physical camera ─────────────────────────────────────

    m.function("use_physical", |entity_id: i64| -> bool {
        with_cam_by_id(entity_id, |c| c.use_physical).unwrap_or(false)
    }).build()?;

    m.function("set_use_physical", |entity_id: i64, v: bool| {
        queue_by_id(entity_id, "use_physical", ReflectValue::Bool(v));
    }).build()?;

    m.function("get_focal_length", |entity_id: i64| -> f64 {
        with_cam_by_id(entity_id, |c| c.focal_length as f64).unwrap_or(50.0)
    }).build()?;

    m.function("set_focal_length", |entity_id: i64, v: f64| {
        queue_by_id(entity_id, "focal_length", ReflectValue::F32(v as f32));
    }).build()?;

    m.function("get_sensor_size", |entity_id: i64| -> Vec<f64> {
        with_cam_by_id(entity_id, |c| {
            vec![c.sensor_size[0] as f64, c.sensor_size[1] as f64]
        }).unwrap_or_else(|| vec![36.0, 24.0])
    }).build()?;

    m.function("set_sensor_size", |entity_id: i64, wh: Vec<f64>| {
        if wh.len() >= 2 {
            queue_by_id(entity_id, "sensor_size_w", ReflectValue::F32(wh[0] as f32));
            queue_by_id(entity_id, "sensor_size_h", ReflectValue::F32(wh[1] as f32));
        }
    }).build()?;

    m.function("get_lens_shift", |entity_id: i64| -> Vec<f64> {
        with_cam_by_id(entity_id, |c| {
            vec![c.lens_shift[0] as f64, c.lens_shift[1] as f64]
        }).unwrap_or_else(|| vec![0.0, 0.0])
    }).build()?;

    m.function("set_lens_shift", |entity_id: i64, xy: Vec<f64>| {
        if xy.len() >= 2 {
            queue_by_id(entity_id, "lens_shift_x", ReflectValue::F32(xy[0] as f32));
            queue_by_id(entity_id, "lens_shift_y", ReflectValue::F32(xy[1] as f32));
        }
    }).build()?;

    // ── Entity-addressed: tag ─────────────────────────────────────────────────

    m.function("get_tag", |entity_id: i64| -> String {
        with_cam_by_id(entity_id, |c| c.tag.clone()).unwrap_or_default()
    }).build()?;

    m.function("set_tag", |entity_id: i64, tag: String| {
        queue_by_id(entity_id, "tag", ReflectValue::Str(tag));
    }).build()?;

    // ── Entity-addressed: per-camera math ─────────────────────────────────────

    m.function("world_to_screen_point", |entity_id: i64, world_xyz: Vec<f64>| -> Vec<f64> {
        let Some((vp, _, _, w, h)) = cam_matrices(entity_id) else {
            return vec![0.0, 0.0, 0.0];
        };
        let pos = glam::Vec3::new(
            world_xyz.first().copied().unwrap_or(0.0) as f32,
            world_xyz.get(1).copied().unwrap_or(0.0) as f32,
            world_xyz.get(2).copied().unwrap_or(0.0) as f32,
        );
        let sc = Camera::world_to_screen(pos, vp, w, h);
        vec![sc.x as f64, sc.y as f64, sc.z as f64]
    }).build()?;

    m.function("screen_to_world_point", |entity_id: i64, screen_xy: Vec<f64>, depth: f64| -> Vec<f64> {
        let Some((_, inv_vp, _, w, h)) = cam_matrices(entity_id) else {
            return vec![0.0, 0.0, 0.0];
        };
        let wp = Camera::screen_to_world(
            screen_xy.first().copied().unwrap_or(0.0) as f32,
            screen_xy.get(1).copied().unwrap_or(0.0) as f32,
            depth as f32,
            inv_vp,
            w,
            h,
        );
        vec![wp.x as f64, wp.y as f64, wp.z as f64]
    }).build()?;

    m.function("screen_point_to_ray", |entity_id: i64, screen_xy: Vec<f64>| -> Vec<f64> {
        let Some((_, inv_vp, cam_pos, w, h)) = cam_matrices(entity_id) else {
            return vec![0.0; 6];
        };
        let (origin, dir) = Camera::screen_point_to_ray(
            screen_xy.first().copied().unwrap_or(0.0) as f32,
            screen_xy.get(1).copied().unwrap_or(0.0) as f32,
            inv_vp,
            cam_pos,
            w,
            h,
        );
        vec![
            origin.x as f64, origin.y as f64, origin.z as f64,
            dir.x    as f64, dir.y    as f64, dir.z    as f64,
        ]
    }).build()?;

    m.function("world_to_viewport_point", |entity_id: i64, world_xyz: Vec<f64>| -> Vec<f64> {
        let Some((vp, _, _, w, h)) = cam_matrices(entity_id) else {
            return vec![0.0, 0.0, 0.0];
        };
        let pos = glam::Vec3::new(
            world_xyz.first().copied().unwrap_or(0.0) as f32,
            world_xyz.get(1).copied().unwrap_or(0.0) as f32,
            world_xyz.get(2).copied().unwrap_or(0.0) as f32,
        );
        let sc = Camera::world_to_screen(pos, vp, w, h);
        // Viewport point: normalized [0,1] instead of pixels.
        vec![
            (sc.x / w as f32) as f64,
            (sc.y / h as f32) as f64,
            sc.z as f64,
        ]
    }).build()?;

    // ── Snapshot-based math (legacy / main-camera shortcuts) ──────────────────

    m.function("screen_to_world", |screen_xy: Vec<f64>, depth: f64| -> Vec<f64> {
        let s = snap();
        let inv_vp = glam::Mat4::from_cols_array_2d(&s.inv_view_proj);
        let wp = Camera::screen_to_world(
            screen_xy.first().copied().unwrap_or(0.0) as f32,
            screen_xy.get(1).copied().unwrap_or(0.0) as f32,
            depth as f32,
            inv_vp,
            s.viewport_w,
            s.viewport_h,
        );
        vec![wp.x as f64, wp.y as f64, wp.z as f64]
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

    Ok(m)
}

// ── fluxion::Camera — Unity-style static API for game scripts ─────────────────
//
// Rune usage (mirrors Unity C# API):
//
//   let cam = fluxion::Camera::main();
//   let fov = fluxion::Camera::field_of_view(cam);
//   fluxion::Camera::set_field_of_view(cam, 90.0);
//   let ray = fluxion::Camera::screen_point_to_ray(cam, mouse_pos);
//
fn build_unity_style_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["Camera"])?;

    // ── Static: camera discovery ──────────────────────────────────────────────

    // `Camera::main()` → entity_id of the main camera, or -1. (Unity: Camera.main)
    m.function("main", || -> i64 { main_camera_id() }).build()?;

    // `Camera::all_cameras()` → Vec<i64> sorted by depth. (Unity: Camera.allCameras)
    m.function("all_cameras", || -> Vec<i64> {
        let Some(world) = world_ref() else { return vec![] };
        let mut ids: Vec<(i32, i64)> = Vec::new();
        world.query_active::<&Camera, _>(|id, cam| {
            ids.push((cam.depth, id.to_bits() as i64));
        });
        ids.sort_by_key(|(d, _)| *d);
        ids.into_iter().map(|(_, id)| id).collect()
    }).build()?;

    // `Camera::find(tag)` → first camera with matching tag, or -1.
    m.function("find", |tag: String| -> i64 {
        let Some(world) = world_ref() else { return -1 };
        let mut found = -1i64;
        world.query_active::<&Camera, _>(|id, cam| {
            if cam.tag == tag && found == -1 { found = id.to_bits() as i64; }
        });
        found
    }).build()?;

    // ── Properties: projection ────────────────────────────────────────────────

    // `Camera::field_of_view(cam)` — matches Unity `camera.fieldOfView`.
    m.function("field_of_view", |entity_id: i64| -> f64 {
        with_cam_by_id(entity_id, |c| c.fov as f64).unwrap_or(70.0)
    }).build()?;

    m.function("set_field_of_view", |entity_id: i64, v: f64| {
        queue_by_id(entity_id, "fov", ReflectValue::F32(v as f32));
    }).build()?;

    // `Camera::near_clip_plane(cam)` — matches Unity `camera.nearClipPlane`.
    m.function("near_clip_plane", |entity_id: i64| -> f64 {
        with_cam_by_id(entity_id, |c| c.near as f64).unwrap_or(0.1)
    }).build()?;

    m.function("set_near_clip_plane", |entity_id: i64, v: f64| {
        queue_by_id(entity_id, "near", ReflectValue::F32(v as f32));
    }).build()?;

    // `Camera::far_clip_plane(cam)` — matches Unity `camera.farClipPlane`.
    m.function("far_clip_plane", |entity_id: i64| -> f64 {
        with_cam_by_id(entity_id, |c| c.far as f64).unwrap_or(1000.0)
    }).build()?;

    m.function("set_far_clip_plane", |entity_id: i64, v: f64| {
        queue_by_id(entity_id, "far", ReflectValue::F32(v as f32));
    }).build()?;

    // `Camera::orthographic(cam)` — matches Unity `camera.orthographic`.
    m.function("orthographic", |entity_id: i64| -> bool {
        with_cam_by_id(entity_id, |c| c.projection_mode == ProjectionMode::Orthographic)
            .unwrap_or(false)
    }).build()?;

    m.function("set_orthographic", |entity_id: i64, v: bool| {
        let mode = if v { "Orthographic" } else { "Perspective" };
        queue_by_id(entity_id, "projection_mode", ReflectValue::Enum(mode.to_string()));
    }).build()?;

    // `Camera::orthographic_size(cam)` — matches Unity `camera.orthographicSize`.
    m.function("orthographic_size", |entity_id: i64| -> f64 {
        with_cam_by_id(entity_id, |c| c.ortho_size as f64).unwrap_or(5.0)
    }).build()?;

    m.function("set_orthographic_size", |entity_id: i64, v: f64| {
        queue_by_id(entity_id, "ortho_size", ReflectValue::F32(v as f32));
    }).build()?;

    // ── Properties: depth & culling ───────────────────────────────────────────

    // `Camera::depth(cam)` — matches Unity `camera.depth`.
    m.function("depth", |entity_id: i64| -> i64 {
        with_cam_by_id(entity_id, |c| c.depth as i64).unwrap_or(0)
    }).build()?;

    m.function("set_depth", |entity_id: i64, v: i64| {
        queue_by_id(entity_id, "depth", ReflectValue::I32(v as i32));
    }).build()?;

    // `Camera::culling_mask(cam)` — matches Unity `camera.cullingMask`.
    m.function("culling_mask", |entity_id: i64| -> i64 {
        with_cam_by_id(entity_id, |c| c.culling_mask as i64).unwrap_or(0xFFFF_FFFFi64)
    }).build()?;

    m.function("set_culling_mask", |entity_id: i64, v: i64| {
        queue_by_id(entity_id, "culling_mask", ReflectValue::U32(v as u32));
    }).build()?;

    // ── Properties: active ────────────────────────────────────────────────────

    m.function("enabled", |entity_id: i64| -> bool {
        with_cam_by_id(entity_id, |c| c.is_active).unwrap_or(false)
    }).build()?;

    m.function("set_enabled", |entity_id: i64, v: bool| {
        queue_by_id(entity_id, "is_active", ReflectValue::Bool(v));
    }).build()?;

    // ── Properties: clear flags & background ─────────────────────────────────

    // `Camera::clear_flags(cam)` — "Skybox" | "SolidColor" | "DepthOnly" | "Nothing".
    m.function("clear_flags", |entity_id: i64| -> String {
        with_cam_by_id(entity_id, |c| c.clear_flags.as_str().to_string())
            .unwrap_or_else(|| "Skybox".to_string())
    }).build()?;

    m.function("set_clear_flags", |entity_id: i64, s: String| {
        queue_by_id(entity_id, "clear_flags", ReflectValue::Enum(s));
    }).build()?;

    // `Camera::background_color(cam)` → [r, g, b, a] — matches Unity `camera.backgroundColor`.
    m.function("background_color", |entity_id: i64| -> Vec<f64> {
        with_cam_by_id(entity_id, |c| {
            let b = c.background_color;
            vec![b[0] as f64, b[1] as f64, b[2] as f64, b[3] as f64]
        }).unwrap_or_else(|| vec![0.1, 0.1, 0.1, 1.0])
    }).build()?;

    m.function("set_background_color", |entity_id: i64, rgba: Vec<f64>| {
        if rgba.len() >= 4 {
            queue_by_id(entity_id, "background_color", ReflectValue::Color4([
                rgba[0] as f32, rgba[1] as f32, rgba[2] as f32, rgba[3] as f32,
            ]));
        }
    }).build()?;

    // ── Properties: viewport rect ─────────────────────────────────────────────

    // `Camera::rect(cam)` → [x, y, w, h] normalized — matches Unity `camera.rect`.
    m.function("rect", |entity_id: i64| -> Vec<f64> {
        with_cam_by_id(entity_id, |c| {
            let r = c.viewport_rect;
            vec![r[0] as f64, r[1] as f64, r[2] as f64, r[3] as f64]
        }).unwrap_or_else(|| vec![0.0, 0.0, 1.0, 1.0])
    }).build()?;

    m.function("set_rect", |entity_id: i64, xywh: Vec<f64>| {
        if xywh.len() >= 4 {
            queue_by_id(entity_id, "viewport_rect_x", ReflectValue::F32(xywh[0] as f32));
            queue_by_id(entity_id, "viewport_rect_y", ReflectValue::F32(xywh[1] as f32));
            queue_by_id(entity_id, "viewport_rect_w", ReflectValue::F32(xywh[2] as f32));
            queue_by_id(entity_id, "viewport_rect_h", ReflectValue::F32(xywh[3] as f32));
        }
    }).build()?;

    // ── Properties: quality ───────────────────────────────────────────────────

    // `Camera::allow_hdr(cam)` — matches Unity `camera.allowHDR`.
    m.function("allow_hdr", |entity_id: i64| -> bool {
        with_cam_by_id(entity_id, |c| c.allow_hdr).unwrap_or(true)
    }).build()?;

    m.function("set_allow_hdr", |entity_id: i64, v: bool| {
        queue_by_id(entity_id, "allow_hdr", ReflectValue::Bool(v));
    }).build()?;

    // `Camera::allow_msaa(cam)` — matches Unity `camera.allowMSAA`.
    m.function("allow_msaa", |entity_id: i64| -> bool {
        with_cam_by_id(entity_id, |c| c.allow_msaa).unwrap_or(false)
    }).build()?;

    m.function("set_allow_msaa", |entity_id: i64, v: bool| {
        queue_by_id(entity_id, "allow_msaa", ReflectValue::Bool(v));
    }).build()?;

    // ── Properties: physical camera ───────────────────────────────────────────

    m.function("use_physical_properties", |entity_id: i64| -> bool {
        with_cam_by_id(entity_id, |c| c.use_physical).unwrap_or(false)
    }).build()?;

    m.function("set_use_physical_properties", |entity_id: i64, v: bool| {
        queue_by_id(entity_id, "use_physical", ReflectValue::Bool(v));
    }).build()?;

    m.function("focal_length", |entity_id: i64| -> f64 {
        with_cam_by_id(entity_id, |c| c.focal_length as f64).unwrap_or(50.0)
    }).build()?;

    m.function("set_focal_length", |entity_id: i64, v: f64| {
        queue_by_id(entity_id, "focal_length", ReflectValue::F32(v as f32));
    }).build()?;

    m.function("sensor_size", |entity_id: i64| -> Vec<f64> {
        with_cam_by_id(entity_id, |c| {
            vec![c.sensor_size[0] as f64, c.sensor_size[1] as f64]
        }).unwrap_or_else(|| vec![36.0, 24.0])
    }).build()?;

    m.function("lens_shift", |entity_id: i64| -> Vec<f64> {
        with_cam_by_id(entity_id, |c| {
            vec![c.lens_shift[0] as f64, c.lens_shift[1] as f64]
        }).unwrap_or_else(|| vec![0.0, 0.0])
    }).build()?;

    // ── Transform convenience (position / forward / up / right) ──────────────

    // `Camera::position(cam)` → [x, y, z] — matches Unity `camera.transform.position`.
    m.function("position", |entity_id: i64| -> Vec<f64> {
        let Some(world) = world_ref() else { return vec![0.0; 3] };
        let Some(eid) = entity_from_id(entity_id) else { return vec![0.0; 3] };
        world.get_component::<Transform>(eid)
            .map(|t| vec![t.world_position.x as f64, t.world_position.y as f64, t.world_position.z as f64])
            .unwrap_or_else(|| vec![0.0; 3])
    }).build()?;

    // `Camera::forward(cam)` → [x, y, z] — matches Unity `camera.transform.forward`.
    m.function("forward", |entity_id: i64| -> Vec<f64> {
        let Some(world) = world_ref() else { return vec![0.0, 0.0, -1.0] };
        let Some(eid) = entity_from_id(entity_id) else { return vec![0.0, 0.0, -1.0] };
        world.get_component::<Transform>(eid)
            .map(|t| { let f = t.world_forward(); vec![f.x as f64, f.y as f64, f.z as f64] })
            .unwrap_or_else(|| vec![0.0, 0.0, -1.0])
    }).build()?;

    // `Camera::up(cam)` → [x, y, z] — matches Unity `camera.transform.up`.
    m.function("up", |entity_id: i64| -> Vec<f64> {
        let Some(world) = world_ref() else { return vec![0.0, 1.0, 0.0] };
        let Some(eid) = entity_from_id(entity_id) else { return vec![0.0, 1.0, 0.0] };
        world.get_component::<Transform>(eid)
            .map(|t| { let u = t.world_up(); vec![u.x as f64, u.y as f64, u.z as f64] })
            .unwrap_or_else(|| vec![0.0, 1.0, 0.0])
    }).build()?;

    // `Camera::right(cam)` → [x, y, z] — matches Unity `camera.transform.right`.
    m.function("right", |entity_id: i64| -> Vec<f64> {
        let Some(world) = world_ref() else { return vec![1.0, 0.0, 0.0] };
        let Some(eid) = entity_from_id(entity_id) else { return vec![1.0, 0.0, 0.0] };
        world.get_component::<Transform>(eid)
            .map(|t| { let r = t.world_right(); vec![r.x as f64, r.y as f64, r.z as f64] })
            .unwrap_or_else(|| vec![1.0, 0.0, 0.0])
    }).build()?;

    // ── World ↔ Screen / Viewport ─────────────────────────────────────────────

    // `Camera::world_to_screen_point(cam, xyz)` → [sx, sy, depth] — Unity: WorldToScreenPoint
    m.function("world_to_screen_point", |entity_id: i64, world_xyz: Vec<f64>| -> Vec<f64> {
        let Some((vp, _, _, w, h)) = cam_matrices(entity_id) else {
            return vec![0.0, 0.0, 0.0];
        };
        let pos = glam::Vec3::new(
            world_xyz.first().copied().unwrap_or(0.0) as f32,
            world_xyz.get(1).copied().unwrap_or(0.0) as f32,
            world_xyz.get(2).copied().unwrap_or(0.0) as f32,
        );
        let sc = Camera::world_to_screen(pos, vp, w, h);
        vec![sc.x as f64, sc.y as f64, sc.z as f64]
    }).build()?;

    // `Camera::world_to_viewport_point(cam, xyz)` → [vx, vy, depth] 0-1 — Unity: WorldToViewportPoint
    m.function("world_to_viewport_point", |entity_id: i64, world_xyz: Vec<f64>| -> Vec<f64> {
        let Some((vp, _, _, w, h)) = cam_matrices(entity_id) else {
            return vec![0.0, 0.0, 0.0];
        };
        let pos = glam::Vec3::new(
            world_xyz.first().copied().unwrap_or(0.0) as f32,
            world_xyz.get(1).copied().unwrap_or(0.0) as f32,
            world_xyz.get(2).copied().unwrap_or(0.0) as f32,
        );
        let sc = Camera::world_to_screen(pos, vp, w, h);
        vec![(sc.x / w as f32) as f64, (sc.y / h as f32) as f64, sc.z as f64]
    }).build()?;

    // `Camera::screen_to_world_point(cam, xy, depth)` → xyz — Unity: ScreenToWorldPoint
    m.function("screen_to_world_point", |entity_id: i64, screen_xy: Vec<f64>, depth: f64| -> Vec<f64> {
        let Some((_, inv_vp, _, w, h)) = cam_matrices(entity_id) else {
            return vec![0.0, 0.0, 0.0];
        };
        let wp = Camera::screen_to_world(
            screen_xy.first().copied().unwrap_or(0.0) as f32,
            screen_xy.get(1).copied().unwrap_or(0.0) as f32,
            depth as f32,
            inv_vp,
            w,
            h,
        );
        vec![wp.x as f64, wp.y as f64, wp.z as f64]
    }).build()?;

    // `Camera::screen_point_to_ray(cam, xy)` → [origin xyz, dir xyz] — Unity: ScreenPointToRay
    m.function("screen_point_to_ray", |entity_id: i64, screen_xy: Vec<f64>| -> Vec<f64> {
        let Some((_, inv_vp, cam_pos, w, h)) = cam_matrices(entity_id) else {
            return vec![0.0; 6];
        };
        let (origin, dir) = Camera::screen_point_to_ray(
            screen_xy.first().copied().unwrap_or(0.0) as f32,
            screen_xy.get(1).copied().unwrap_or(0.0) as f32,
            inv_vp,
            cam_pos,
            w,
            h,
        );
        vec![
            origin.x as f64, origin.y as f64, origin.z as f64,
            dir.x    as f64, dir.y    as f64, dir.z    as f64,
        ]
    }).build()?;

    Ok(m)
}
