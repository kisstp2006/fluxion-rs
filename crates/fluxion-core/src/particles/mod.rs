// ============================================================
// fluxion-core — Particle simulation (CPU)
//
// Steps all [`ParticleEmitter`] components with [`Transform`] for spawn origin
// and emission direction (transform forward = -Z column of world matrix).
// ============================================================

use glam::Vec3;

use crate::ecs::world::ECSWorld;
use crate::components::ParticleEmitter;
use crate::transform::Transform;

/// Advance emitters: spawn from rate, integrate, cull.
pub fn step_particle_emitters(world: &mut ECSWorld, dt: f32) {
    world.query_active_mut::<(&Transform, &mut ParticleEmitter), _>(|_, (transform, emitter)| {
        let origin = transform.world_matrix.transform_point3(Vec3::ZERO);
        // Forward in Fluxion: -Z in local space → world column 2 negated (right-handed view forward)
        let local_fwd = -Vec3::Z;
        let emit_dir = transform.world_matrix.transform_vector3(local_fwd).normalize_or_zero();

        emitter.accumulator += emitter.spawn_per_second * dt;
        while emitter.accumulator >= 1.0 {
            emitter.accumulator -= 1.0;
            emitter.emit(origin, emit_dir);
        }
        emitter.integrate(dt);
    });
}
