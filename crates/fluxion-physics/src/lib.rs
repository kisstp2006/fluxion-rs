// ============================================================
// fluxion-physics — minimal Rapier3D world (f32)
//
// Scene JSON collider mapping can be added alongside
// [`fluxion_core::scene::deserialize_world`].
// ============================================================

use glam::Vec3;
use rapier3d::na::{Unit, vector};
use rapier3d::prelude::*;

pub use rapier3d::prelude::RigidBodyHandle;

/// Owns Rapier sets + pipeline. Step once per frame with [`Self::step`].
pub struct PhysicsWorld {
    gravity:                Vector<f32>,
    integration_parameters: IntegrationParameters,
    physics_pipeline:       PhysicsPipeline,
    island_manager:         IslandManager,
    broad_phase:            DefaultBroadPhase,
    narrow_phase:           NarrowPhase,
    pub bodies:             RigidBodySet,
    pub colliders:          ColliderSet,
    impulse_joints:         ImpulseJointSet,
    multibody_joints:       MultibodyJointSet,
    ccd_solver:             CCDSolver,
}

impl PhysicsWorld {
    pub fn new(gravity: Vec3) -> Self {
        Self {
            gravity: vector![gravity.x, gravity.y, gravity.z],
            integration_parameters: IntegrationParameters::default(),
            physics_pipeline: PhysicsPipeline::new(),
            island_manager: IslandManager::new(),
            broad_phase: DefaultBroadPhase::default(),
            narrow_phase: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
        }
    }

    /// Advance simulation by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        self.integration_parameters.dt = dt;
        self.physics_pipeline.step(
            &self.gravity,
            &self.integration_parameters,
            &mut self.island_manager,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            None,
            &(),
            &(),
        );
    }

    /// Static ground as half-space (+Y up).
    pub fn add_ground_plane(&mut self) {
        let body = RigidBodyBuilder::fixed().build();
        let h = self.bodies.insert(body);
        let n = Unit::new_normalize(vector![0.0f32, 1.0, 0.0]);
        let col = ColliderBuilder::halfspace(n).build();
        self.colliders.insert_with_parent(col, h, &mut self.bodies);
    }

    /// Dynamic ball; returns rigid-body handle.
    pub fn add_ball(&mut self, radius: f32, translation: Vec3) -> RigidBodyHandle {
        let rb = RigidBodyBuilder::dynamic()
            .translation(vector![translation.x, translation.y, translation.z])
            .build();
        let h = self.bodies.insert(rb);
        let col = ColliderBuilder::ball(radius).build();
        self.colliders.insert_with_parent(col, h, &mut self.bodies);
        h
    }

    pub fn body_translation(&self, h: RigidBodyHandle) -> Option<Vec3> {
        self.bodies.get(h).map(|b| {
            let t = b.translation();
            Vec3::new(t.x, t.y, t.z)
        })
    }
}
