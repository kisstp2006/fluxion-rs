// ============================================================
// camera_controller.rs — Built-in camera controller components
//
// Three controller types matching Unity's common patterns:
//   Free  — WASD + mouse look (first-person fly cam)
//   Orbit — rotate around a target point / entity
//   Follow — chase a target entity with smooth damping
//
// The actual update logic lives in CameraControllerSystem (below).
// Call `CameraControllerSystem::update(&mut world, &input, dt)` each
// frame in the game loop (NOT the editor loop).
// ============================================================

use serde::{Deserialize, Serialize};
use glam::{Vec3, Quat};
use fluxion_reflect_derive::Reflect;

use crate::ecs::component::Component;
use crate::transform::Transform;
use crate::input::InputState;
use crate::ecs::world::ECSWorld;
use crate::ecs::entity::EntityId;

// ── Controller type ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ControllerType {
    /// WASD + mouse drag look — free-fly first-person camera.
    Free,
    /// Orbit around a fixed target point.
    Orbit,
    /// Chase a target entity with smooth follow.
    Follow,
}

// ── CameraController component ────────────────────────────────────────────────

/// Attach to a Camera entity to enable built-in movement.
/// Only active during Play mode (the host must drive `CameraControllerSystem::update`).
#[derive(Debug, Clone, Serialize, Deserialize, Reflect)]
pub struct CameraController {
    pub controller_type: ControllerType,

    // ── Free / Follow shared settings ─────────────────────────────────────────

    /// Movement speed in m/s (WASD).
    #[reflect(range(min = 0.1, max = 200.0))]
    pub move_speed: f32,

    /// Mouse look sensitivity (degrees per pixel).
    #[reflect(range(min = 0.01, max = 5.0))]
    pub look_sensitivity: f32,

    // ── Orbit settings ────────────────────────────────────────────────────────

    /// World-space pivot point for Orbit mode.
    pub orbit_target: Vec3,

    /// Distance from the pivot (meters).
    #[reflect(range(min = 0.1, max = 1000.0))]
    pub orbit_radius: f32,

    /// Current azimuth (horizontal angle, degrees).
    pub azimuth: f32,

    /// Current elevation (vertical angle, degrees, clamped -89..89).
    #[reflect(range(min = -89.0, max = 89.0))]
    pub elevation: f32,

    // ── Follow settings ───────────────────────────────────────────────────────

    /// Entity ID to follow (as i64; -1 = none).
    pub follow_target: i64,

    /// Offset from the target in the target's local space.
    pub follow_offset: Vec3,

    /// Positional damping factor (higher = snappier). Reasonable range: 1..20.
    #[reflect(range(min = 0.1, max = 50.0))]
    pub follow_damping: f32,
}

impl Default for CameraController {
    fn default() -> Self {
        Self {
            controller_type:  ControllerType::Free,
            move_speed:       5.0,
            look_sensitivity: 0.15,
            orbit_target:     Vec3::ZERO,
            orbit_radius:     5.0,
            azimuth:          0.0,
            elevation:        20.0,
            follow_target:    -1,
            follow_offset:    Vec3::new(0.0, 2.0, -5.0),
            follow_damping:   8.0,
        }
    }
}

// Needed so the Reflect derive doesn't require ControllerType to be Reflect.
// We handle it manually as "enum" type, displayed read-only in inspector.
impl Component for CameraController {}

// ── System ────────────────────────────────────────────────────────────────────

pub struct CameraControllerSystem;

impl CameraControllerSystem {
    /// Drive all CameraController components.
    /// `dt` = frame delta time in seconds.
    pub fn update(world: &mut ECSWorld, input: &InputState, dt: f32) {
        // Collect entities with CameraController so we can mutate world freely.
        let mut entities: Vec<EntityId> = Vec::new();
        world.query_all::<(&CameraController, &Transform), _>(|e, _| {
            entities.push(e);
        });

        for entity in entities {
            let ctrl_type;
            let (speed, sens, orbit_target, orbit_radius, azimuth, elevation,
                 follow_target, follow_offset, follow_damping);

            {
                let ctrl = world.get_component::<CameraController>(entity).unwrap();
                ctrl_type     = ctrl.controller_type;
                speed         = ctrl.move_speed;
                sens          = ctrl.look_sensitivity;
                orbit_target  = ctrl.orbit_target;
                orbit_radius  = ctrl.orbit_radius;
                azimuth       = ctrl.azimuth;
                elevation     = ctrl.elevation;
                follow_target = ctrl.follow_target;
                follow_offset = ctrl.follow_offset;
                follow_damping = ctrl.follow_damping;
            }

            match ctrl_type {
                ControllerType::Free => {
                    Self::update_free(world, entity, input, dt, speed, sens);
                }
                ControllerType::Orbit => {
                    Self::update_orbit(world, entity, input, dt, sens,
                        orbit_target, orbit_radius, azimuth, elevation);
                }
                ControllerType::Follow => {
                    Self::update_follow(world, entity, input, dt,
                        follow_target, follow_offset, follow_damping);
                }
            }
        }
    }

    // ── Free camera ───────────────────────────────────────────────────────────

    fn update_free(
        world: &mut ECSWorld,
        entity: EntityId,
        input: &InputState,
        dt: f32,
        speed: f32,
        sens: f32,
    ) {
        let right_mouse = input.mouse_right();
        let (dx, dy) = if right_mouse { input.mouse_delta() } else { (0.0, 0.0) };

        // WASD movement
        let fwd   = input.axis_vertical()   * speed * dt;
        let right = input.axis_horizontal() * speed * dt;
        let up    = (input.is_key_down("KeyE") as i32 as f32
                   - input.is_key_down("KeyQ") as i32 as f32) * speed * dt;

        // Speed modifier (Shift)
        let boost = if input.is_key_down("ShiftLeft") || input.is_key_down("ShiftRight") {
            3.0
        } else {
            1.0
        };

        if let Some(mut t) = world.get_component_mut::<Transform>(entity) {
            // Rotate on right-drag
            if right_mouse && (dx.abs() > 0.001 || dy.abs() > 0.001) {
                let (roll, pitch, _yaw) = t.rotation.to_euler(glam::EulerRot::YXZ);
                let new_yaw   = roll   - (dx * sens).to_radians();
                let new_pitch = (pitch - (dy * sens).to_radians())
                    .clamp(-89_f32.to_radians(), 89_f32.to_radians());
                t.rotation = Quat::from_euler(glam::EulerRot::YXZ, new_yaw, new_pitch, 0.0);
            }

            // Translate along local axes
            let forward  = t.rotation * Vec3::NEG_Z;
            let right_v  = t.rotation * Vec3::X;
            let up_v     = Vec3::Y;

            t.position += forward  * fwd   * boost;
            t.position += right_v  * right * boost;
            t.position += up_v     * up    * boost;
            t.dirty = true;
        }
    }

    // ── Orbit camera ──────────────────────────────────────────────────────────

    fn update_orbit(
        world: &mut ECSWorld,
        entity: EntityId,
        input: &InputState,
        _dt: f32,
        sens: f32,
        orbit_target: Vec3,
        orbit_radius: f32,
        mut azimuth: f32,
        mut elevation: f32,
    ) {
        let left_drag  = input.mouse_left();
        let (dx, dy)   = if left_drag { input.mouse_delta() } else { (0.0, 0.0) };
        let (_, sy)    = input.scroll_delta();

        // Drag rotates
        azimuth   -= dx * sens;
        elevation  = (elevation + dy * sens).clamp(-89.0, 89.0);

        // Scroll zooms
        let new_radius = (orbit_radius - sy * 0.5).max(0.1);

        // Recompute position
        let az_rad = azimuth.to_radians();
        let el_rad = elevation.to_radians();
        let pos = orbit_target + Vec3::new(
            el_rad.cos() * az_rad.sin(),
            el_rad.sin(),
            el_rad.cos() * az_rad.cos(),
        ) * new_radius;
        let forward = (orbit_target - pos).normalize_or_zero();
        let rot = if forward.length_squared() > 1e-6 {
            Quat::from_rotation_arc(Vec3::NEG_Z, forward)
        } else {
            Quat::IDENTITY
        };

        // Persist updated angles + radius back into the component
        if let Some(mut ctrl) = world.get_component_mut::<CameraController>(entity) {
            ctrl.azimuth      = azimuth;
            ctrl.elevation    = elevation;
            ctrl.orbit_radius = new_radius;
        }
        if let Some(mut t) = world.get_component_mut::<Transform>(entity) {
            t.position = pos;
            t.rotation = rot;
            t.dirty    = true;
        }
    }

    // ── Follow camera ─────────────────────────────────────────────────────────

    fn update_follow(
        world: &mut ECSWorld,
        entity: EntityId,
        _input: &InputState,
        dt: f32,
        follow_target: i64,
        follow_offset: Vec3,
        follow_damping: f32,
    ) {
        if follow_target < 0 { return; }

        // Find the target entity by matching its bit representation.
        let target_bits = follow_target as u64;
        let target = world.all_entities().find(|e| e.to_bits() == target_bits);
        let Some(target) = target else { return };

        let (target_pos, target_rot) = if let Some(t) = world.get_component::<Transform>(target) {
            (t.position, t.rotation)
        } else {
            return;
        };

        let desired = target_pos + target_rot * follow_offset;
        let forward = (target_pos - desired).normalize_or_zero();
        let look_at = if forward.length_squared() > 1e-6 {
            Quat::from_rotation_arc(Vec3::NEG_Z, forward)
        } else {
            Quat::IDENTITY
        };

        let alpha = (follow_damping * dt).min(1.0);

        if let Some(mut t) = world.get_component_mut::<Transform>(entity) {
            t.position = t.position.lerp(desired, alpha);
            t.rotation = t.rotation.slerp(look_at, alpha);
            t.dirty    = true;
        }
    }
}
