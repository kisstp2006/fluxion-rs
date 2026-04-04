// ============================================================
// fluxion-physics/rune_module.rs — fluxion::physics Rune module
//
// Compiled only when the `rune-scripting` feature is enabled.
//
// Exposes physics operations to Rune scripts:
//   fluxion::physics::add_force(id, [fx, fy, fz])
//   fluxion::physics::add_impulse(id, [ix, iy, iz])
//   fluxion::physics::set_velocity(id, [vx, vy, vz])
//   fluxion::physics::get_velocity(id) -> [vx, vy, vz]
//   fluxion::physics::set_gravity_scale(id, scale)
//
// Thread-local context pointer is set each frame by the engine host
// via set_physics_context() before Rune scripts run.
// ============================================================

use std::cell::Cell;
use std::ptr::NonNull;

use rune::Module;

use crate::PhysicsEcsWorld;

// ── Thread-local context ──────────────────────────────────────────────────────

thread_local! {
    static PHYS_PTR: Cell<Option<NonNull<PhysicsEcsWorld>>> = Cell::new(None);
}

/// Set the physics world pointer for the current frame.
/// Must be called before any Rune scripts run.
/// # Safety: pointer must remain valid for the duration of the Rune call.
pub fn set_physics_context(phys: &mut PhysicsEcsWorld) {
    PHYS_PTR.with(|c| c.set(Some(NonNull::from(phys))));
}

/// Clear the physics world pointer (call after Rune scripts finish).
pub fn clear_physics_context() {
    PHYS_PTR.with(|c| c.set(None));
}

// ── Internal helper ───────────────────────────────────────────────────────────

fn with_phys<R>(f: impl FnOnce(&mut PhysicsEcsWorld) -> R) -> Option<R> {
    let mut ptr = PHYS_PTR.with(|c| c.get())?;
    // SAFETY: valid for duration of current Rune call (cleared immediately after).
    Some(unsafe { f(ptr.as_mut()) })
}

fn bits_to_entity(id: i64) -> fluxion_core::EntityId {
    unsafe { std::mem::transmute::<u64, fluxion_core::EntityId>(id as u64) }
}

// ── Module builder ────────────────────────────────────────────────────────────

/// Build the `fluxion::physics` Rune module.
/// Register this with the Rune context in your engine host.
pub fn build_physics_rune_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["physics"])?;

    // Physics.AddForce(entity_id, [fx, fy, fz])
    m.function("add_force", |id: i64, force: Vec<f64>| {
        if force.len() < 3 { return; }
        let entity = bits_to_entity(id);
        let v = glam::Vec3::new(force[0] as f32, force[1] as f32, force[2] as f32);
        let _ = with_phys(|phys| phys.add_force(entity, v));
    }).build()?;

    // Physics.AddImpulse(entity_id, [ix, iy, iz])
    m.function("add_impulse", |id: i64, impulse: Vec<f64>| {
        if impulse.len() < 3 { return; }
        let entity = bits_to_entity(id);
        let v = glam::Vec3::new(impulse[0] as f32, impulse[1] as f32, impulse[2] as f32);
        let _ = with_phys(|phys| phys.add_impulse(entity, v));
    }).build()?;

    // Physics.SetVelocity(entity_id, [vx, vy, vz])
    m.function("set_velocity", |id: i64, vel: Vec<f64>| {
        if vel.len() < 3 { return; }
        let entity = bits_to_entity(id);
        let v = glam::Vec3::new(vel[0] as f32, vel[1] as f32, vel[2] as f32);
        let _ = with_phys(|phys| phys.set_linear_velocity(entity, v));
    }).build()?;

    // Physics.GetVelocity(entity_id) -> [vx, vy, vz]
    m.function("get_velocity", |id: i64| -> Vec<f64> {
        let entity = bits_to_entity(id);
        with_phys(|phys| {
            let v = phys.get_linear_velocity(entity);
            vec![v.x as f64, v.y as f64, v.z as f64]
        }).unwrap_or_else(|| vec![0.0, 0.0, 0.0])
    }).build()?;

    // Physics.SetGravityScale(entity_id, scale)
    m.function("set_gravity_scale", |id: i64, scale: f64| {
        let entity = bits_to_entity(id);
        let _ = with_phys(|phys| phys.set_gravity_scale(entity, scale as f32));
    }).build()?;

    // Physics.Raycast(origin [x,y,z], direction [x,y,z], max_dist) -> Option<[hit_x, hit_y, hit_z, norm_x, norm_y, norm_z, dist, entity_id]>
    // Returns a Vec with 8 elements on hit, empty Vec on miss.
    // Unity equivalent: Physics.Raycast(origin, direction, out hit, maxDist)
    m.function("raycast", |origin: Vec<f64>, dir: Vec<f64>, max_dist: f64| -> Vec<f64> {
        if origin.len() < 3 || dir.len() < 3 { return vec![]; }
        let o = glam::Vec3::new(origin[0] as f32, origin[1] as f32, origin[2] as f32);
        let d = glam::Vec3::new(dir[0] as f32, dir[1] as f32, dir[2] as f32);
        with_phys(|phys| {
            match phys.raycast(o, d, max_dist as f32) {
                Some(hit) => vec![
                    hit.point.x as f64,
                    hit.point.y as f64,
                    hit.point.z as f64,
                    hit.normal.x as f64,
                    hit.normal.y as f64,
                    hit.normal.z as f64,
                    hit.distance as f64,
                    hit.entity.to_bits() as i64 as f64,
                ],
                None => vec![],
            }
        }).unwrap_or_default()
    }).build()?;

    Ok(m)
}
