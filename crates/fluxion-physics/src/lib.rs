// ============================================================
// fluxion-physics — Rapier3D world + ECS integration
//
// Two public types:
//
//   PhysicsWorld       — low-level Rapier wrapper (unchanged from before)
//   PhysicsEcsWorld    — ECS-integrated wrapper; maps EntityId → RigidBodyHandle,
//                        syncs Transform components each frame.
// ============================================================

use std::collections::HashMap;

use glam::Vec3;
use rapier3d::na::{Unit, vector, UnitQuaternion, Quaternion};
use rapier3d::prelude::*;

use fluxion_core::{ECSWorld, EntityId, RigidBody, PhysicsShape, BodyType};
use fluxion_core::transform::Transform;

pub use rapier3d::prelude::RigidBodyHandle;

// ── Low-level Rapier wrapper (unchanged API) ───────────────────────────────────

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

// ── ECS-integrated physics world ──────────────────────────────────────────────

/// Per-entity physics handles stored in [`PhysicsEcsWorld`].
struct EntityEntry {
    body_handle:     RigidBodyHandle,
    #[allow(dead_code)]
    collider_handle: ColliderHandle,
}

/// ECS-integrated physics world.
///
/// Wraps [`PhysicsWorld`] and adds a mapping from [`EntityId`] → Rapier handles.
///
/// # Typical per-frame usage
/// ```text
/// physics.sync_from_ecs(&world);   // register new RigidBody entities, remove despawned
/// physics.step(dt);                // simulate
/// physics.sync_to_ecs(&world);     // write Rapier positions back to Transform (Dynamic only)
/// TransformSystem::update(&mut world); // propagate dirty flags through hierarchy
/// ```
pub struct PhysicsEcsWorld {
    inner:      PhysicsWorld,
    entity_map: HashMap<EntityId, EntityEntry>,
}

impl PhysicsEcsWorld {
    pub fn new(gravity: Vec3) -> Self {
        Self {
            inner:      PhysicsWorld::new(gravity),
            entity_map: HashMap::new(),
        }
    }

    // ── Public API ─────────────────────────────────────────────────────────────

    /// Scan the ECS for entities with [`RigidBody`] that are not yet registered
    /// and create Rapier bodies for them. Also removes entries for despawned entities.
    ///
    /// Call this **before** [`Self::step`] every frame.
    pub fn sync_from_ecs(&mut self, world: &ECSWorld) {
        // 1. Remove stale entries (entities that were despawned).
        let stale: Vec<EntityId> = self.entity_map
            .keys()
            .filter(|&&eid| !world.is_alive(eid))
            .copied()
            .collect();
        for eid in stale {
            if let Some(entry) = self.entity_map.remove(&eid) {
                self.inner.bodies.remove(
                    entry.body_handle,
                    &mut self.inner.island_manager,
                    &mut self.inner.colliders,
                    &mut self.inner.impulse_joints,
                    &mut self.inner.multibody_joints,
                    true, // also removes attached colliders
                );
            }
        }

        // 2. Register new RigidBody + Transform entities.
        let mut to_add: Vec<(EntityId, RigidBody, Vec3, glam::Quat)> = Vec::new();
        world.query_all::<(&RigidBody, &Transform), _>(|eid, (rb, t)| {
            if !self.entity_map.contains_key(&eid) {
                to_add.push((eid, rb.clone(), t.world_position, t.world_rotation));
            }
        });

        for (eid, rb, pos, rot) in to_add {
            let entry = self.create_body_for(&rb, pos, rot);
            self.entity_map.insert(eid, entry);
        }
    }

    /// Advance the simulation by `dt` seconds (fixed timestep recommended).
    pub fn step(&mut self, dt: f32) {
        self.inner.step(dt);
    }

    /// Copy simulated body positions/rotations back to ECS [`Transform`] components.
    ///
    /// Only Dynamic bodies are written back. Static and Kinematic bodies remain
    /// authoritative in the ECS.
    ///
    /// Call this **after** [`Self::step`], then run `TransformSystem::update`.
    pub fn sync_to_ecs(&self, world: &ECSWorld) {
        for (eid, entry) in &self.entity_map {
            let Some(body) = self.inner.bodies.get(entry.body_handle) else { continue };
            if !body.is_dynamic() { continue; }

            let Some(mut t) = world.get_component_mut::<Transform>(*eid) else { continue };

            let tr = body.translation();
            let ro = body.rotation();

            t.position    = Vec3::new(tr.x, tr.y, tr.z);
            t.rotation    = glam::Quat::from_xyzw(ro.i, ro.j, ro.k, ro.w);
            t.dirty       = true;
            t.world_dirty = true;
        }
    }

    // ── Accessors ──────────────────────────────────────────────────────────────

    /// Returns the Rapier [`RigidBodyHandle`] for `entity`, if registered.
    pub fn body_handle(&self, entity: EntityId) -> Option<RigidBodyHandle> {
        self.entity_map.get(&entity).map(|e| e.body_handle)
    }

    /// Read-only access to the inner [`PhysicsWorld`] (e.g. for debug draw).
    pub fn inner(&self) -> &PhysicsWorld { &self.inner }

    // ── Internal helpers ───────────────────────────────────────────────────────

    fn create_body_for(
        &mut self,
        rb:  &RigidBody,
        pos: Vec3,
        rot: glam::Quat,
    ) -> EntityEntry {
        // Build Rapier rigid body.
        let body_builder = match rb.body_type {
            BodyType::Dynamic   => RigidBodyBuilder::dynamic(),
            BodyType::Kinematic => RigidBodyBuilder::kinematic_position_based(),
            BodyType::Static    => RigidBodyBuilder::fixed(),
        };

        // Convert glam Quat → rapier nalgebra rotation.
        let na_rot = UnitQuaternion::new_normalize(
            Quaternion::new(rot.w, rot.x, rot.y, rot.z)
        );
        let axis_angle = na_rot.scaled_axis();

        let rapier_body = body_builder
            .translation(vector![pos.x, pos.y, pos.z])
            .rotation(axis_angle)
            .linear_damping(rb.linear_damping)
            .angular_damping(rb.angular_damping)
            .gravity_scale(rb.gravity_scale)
            .can_sleep(rb.can_sleep)
            .build();

        let body_handle = self.inner.bodies.insert(rapier_body);

        // Build collider.
        let col_builder: ColliderBuilder = match rb.shape {
            PhysicsShape::Box { half_extents } =>
                ColliderBuilder::cuboid(half_extents[0], half_extents[1], half_extents[2]),
            PhysicsShape::Sphere { radius } =>
                ColliderBuilder::ball(radius),
            PhysicsShape::Capsule { half_height, radius } =>
                ColliderBuilder::capsule_y(half_height, radius),
            PhysicsShape::HalfSpace => {
                let n = Unit::new_normalize(vector![0.0f32, 1.0, 0.0]);
                ColliderBuilder::halfspace(n)
            }
        };

        let collider = col_builder
            .restitution(rb.restitution)
            .friction(rb.friction)
            .build();

        let collider_handle = self.inner.colliders
            .insert_with_parent(collider, body_handle, &mut self.inner.bodies);

        EntityEntry { body_handle, collider_handle }
    }
}
