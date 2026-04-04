// ============================================================
// fluxion-core — GPU-style particle emitter (CPU simulation MVP)
//
// Particles are simulated in world space each frame; the renderer
// draws them as billboard quads (instanced pass).
// ============================================================

use glam::Vec3;

use crate::ecs::component::Component;

/// One simulated particle (world space).
#[derive(Debug, Clone, Copy)]
pub struct Particle {
    pub position: Vec3,
    pub velocity: Vec3,
    pub age:      f32,
    pub max_age:  f32,
    pub color:    [f32; 4],
    pub size:     f32,
}

/// Spawns and holds particles for one emitter entity. Attach with [`Transform`].
#[derive(Debug, Clone)]
pub struct ParticleEmitter {
    pub max_particles:    usize,
    pub spawn_per_second: f32,
    pub lifetime:         f32,
    pub start_speed:      f32,
    pub gravity:          Vec3,
    pub color:            [f32; 4],
    pub size:             f32,
    pub spread_degrees:   f32,
    pub(crate) accumulator: f32,
    pub particles:        Vec<Particle>,
    rng_state:            u32,
}

impl Default for ParticleEmitter {
    fn default() -> Self {
        Self {
            max_particles:    256,
            spawn_per_second: 24.0,
            lifetime:         2.0,
            start_speed:      2.0,
            gravity:          Vec3::new(0.0, -3.0, 0.0),
            color:            [1.0, 0.6, 0.2, 1.0],
            size:             0.08,
            spread_degrees:   35.0,
            accumulator:      0.0,
            particles:        Vec::new(),
            rng_state:          0xdeadbeef,
        }
    }
}

impl ParticleEmitter {
    fn next_unit(&mut self) -> f32 {
        // xorshift — deterministic, no `rand` dependency
        self.rng_state ^= self.rng_state << 13;
        self.rng_state ^= self.rng_state >> 17;
        self.rng_state ^= self.rng_state << 5;
        (self.rng_state as f32) * (1.0 / u32::MAX as f32)
    }

    fn random_unit_vector(&mut self) -> Vec3 {
        let u = self.next_unit() * std::f32::consts::TAU;
        let v = self.next_unit() * 2.0 - 1.0;
        let r = (1.0 - v * v).max(0.0).sqrt();
        Vec3::new(r * u.cos(), v, r * u.sin())
    }

    /// Emit one particle at `origin` with velocity biased along `emit_dir`.
    pub fn emit(&mut self, origin: Vec3, emit_dir: Vec3) {
        if self.particles.len() >= self.max_particles {
            return;
        }
        let base = emit_dir.normalize_or_zero();
        let dir = if self.spread_degrees <= 0.01 || base.length_squared() < 1e-6 {
            if base.length_squared() < 1e-6 {
                self.random_unit_vector()
            } else {
                base
            }
        } else {
            let spread = (self.spread_degrees * 0.5).to_radians().tan();
            let rnd = self.random_unit_vector();
            (base + rnd * spread).normalize_or_zero()
        };
        let dir = if dir.length_squared() < 1e-6 { Vec3::Y } else { dir };
        self.particles.push(Particle {
            position: origin,
            velocity: dir * self.start_speed,
            age:      0.0,
            max_age:  self.lifetime,
            color:    self.color,
            size:     self.size,
        });
    }

    /// Integrate gravity, advect, cull dead particles. Call each frame after spawning.
    pub fn integrate(&mut self, dt: f32) {
        for p in &mut self.particles {
            p.velocity += self.gravity * dt;
            p.position += p.velocity * dt;
            p.age += dt;
        }
        self.particles.retain(|p| p.age < p.max_age);
    }
}

impl Component for ParticleEmitter {}
