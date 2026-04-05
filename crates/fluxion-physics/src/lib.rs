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

#[cfg(feature = "rune-scripting")]
pub mod rune_module;
#[cfg(feature = "rune-scripting")]
pub use rune_module::{build_physics_rune_module, set_physics_context, clear_physics_context};

pub use rapier3d::prelude::RigidBodyHandle;

// ── Collision event types ─────────────────────────────────────────────────────

/// A resolved collision event between two ECS entities.
#[derive(Clone, Debug)]
pub struct PhysicsCollisionEvent {
    pub entity_a: EntityId,
    pub entity_b: EntityId,
    pub started:  bool,  // true = OnCollisionEnter, false = OnCollisionExit
}

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
/// physics.step(dt);                // simulate, collision events collected internally
/// physics.sync_to_ecs(&world);     // write Rapier positions back to Transform (Dynamic only)
/// physics.drain_collision_events() // take collected events for dispatch
/// TransformSystem::update(&mut world); // propagate dirty flags through hierarchy
/// ```
/// Result of a `Physics.Raycast` query.
#[derive(Clone, Debug)]
pub struct RaycastHit {
    pub entity:   EntityId,
    pub point:    Vec3,
    pub normal:   Vec3,
    pub distance: f32,
}

pub struct PhysicsEcsWorld {
    inner:          PhysicsWorld,
    entity_map:     HashMap<EntityId, EntityEntry>,
    /// handle → EntityId reverse lookup (rebuilt in sync_from_ecs)
    handle_to_entity: HashMap<RigidBodyHandle, EntityId>,
    /// Collision events collected during the last step().
    pending_events: Vec<PhysicsCollisionEvent>,
    /// Query pipeline for raycasts and shape queries (updated after each step).
    query_pipeline: QueryPipeline,
}

impl PhysicsEcsWorld {
    pub fn new(gravity: Vec3) -> Self {
        Self {
            inner:            PhysicsWorld::new(gravity),
            entity_map:       HashMap::new(),
            handle_to_entity: HashMap::new(),
            pending_events:   Vec::new(),
            query_pipeline:   QueryPipeline::new(),
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
                self.handle_to_entity.remove(&entry.body_handle);
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
            self.handle_to_entity.insert(entry.body_handle, eid);
            self.entity_map.insert(eid, entry);
        }
    }

    /// Advance the simulation by `dt` seconds (fixed timestep recommended).
    /// Collision events are collected and available via [`Self::drain_collision_events`].
    pub fn step(&mut self, dt: f32) {
        use rapier3d::pipeline::ChannelEventCollector;
        use crossbeam::channel::unbounded;

        let (collision_send, collision_recv) = unbounded();
        let (contact_force_send, _contact_force_recv) = unbounded();
        let event_handler = ChannelEventCollector::new(collision_send, contact_force_send);

        self.inner.integration_parameters.dt = dt;
        self.inner.physics_pipeline.step(
            &self.inner.gravity,
            &self.inner.integration_parameters,
            &mut self.inner.island_manager,
            &mut self.inner.broad_phase,
            &mut self.inner.narrow_phase,
            &mut self.inner.bodies,
            &mut self.inner.colliders,
            &mut self.inner.impulse_joints,
            &mut self.inner.multibody_joints,
            &mut self.inner.ccd_solver,
            None,
            &(),
            &event_handler,
        );

        // Update query pipeline so raycasts see the latest collider positions.
        self.query_pipeline.update(&self.inner.colliders);

        // Drain collision events into pending_events, resolving handles → EntityIds.
        while let Ok(evt) = collision_recv.try_recv() {
            let col_a = evt.collider1();
            let col_b = evt.collider2();
            let handle_a = self.inner.colliders.get(col_a).and_then(|c| c.parent());
            let handle_b = self.inner.colliders.get(col_b).and_then(|c| c.parent());
            if let (Some(ha), Some(hb)) = (handle_a, handle_b) {
                if let (Some(&ea), Some(&eb)) = (
                    self.handle_to_entity.get(&ha),
                    self.handle_to_entity.get(&hb),
                ) {
                    self.pending_events.push(PhysicsCollisionEvent {
                        entity_a: ea,
                        entity_b: eb,
                        started:  evt.started(),
                    });
                }
            }
        }
    }

    /// Take all collision events collected since the last call.
    /// The internal buffer is cleared.
    pub fn drain_collision_events(&mut self) -> Vec<PhysicsCollisionEvent> {
        std::mem::take(&mut self.pending_events)
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


    // ── Glam-friendly high-level API (used by editor scripting layer) ──────────

    /// Add a continuous world-space force (Newtons) to an entity's rigid body.
    pub fn add_force(&mut self, entity: EntityId, force: Vec3) {
        if let Some(handle) = self.body_handle(entity) {
            if let Some(body) = self.inner.bodies.get_mut(handle) {
                body.add_force(vector![force.x, force.y, force.z], true);
            }
        }
    }

    /// Apply an instantaneous world-space impulse (kg·m/s) to an entity's rigid body.
    pub fn add_impulse(&mut self, entity: EntityId, impulse: Vec3) {
        if let Some(handle) = self.body_handle(entity) {
            if let Some(body) = self.inner.bodies.get_mut(handle) {
                body.apply_impulse(vector![impulse.x, impulse.y, impulse.z], true);
            }
        }
    }

    /// Set the linear velocity of an entity's rigid body.
    pub fn set_linear_velocity(&mut self, entity: EntityId, vel: Vec3) {
        if let Some(handle) = self.body_handle(entity) {
            if let Some(body) = self.inner.bodies.get_mut(handle) {
                body.set_linvel(vector![vel.x, vel.y, vel.z], true);
            }
        }
    }

    /// Get the linear velocity of an entity's rigid body. Returns `Vec3::ZERO` if not found.
    pub fn get_linear_velocity(&self, entity: EntityId) -> Vec3 {
        if let Some(handle) = self.body_handle(entity) {
            if let Some(body) = self.inner.bodies.get(handle) {
                let v = body.linvel();
                return Vec3::new(v.x, v.y, v.z);
            }
        }
        Vec3::ZERO
    }

    /// Set the gravity scale multiplier for an entity's rigid body.
    pub fn set_gravity_scale(&mut self, entity: EntityId, scale: f32) {
        if let Some(handle) = self.body_handle(entity) {
            if let Some(body) = self.inner.bodies.get_mut(handle) {
                body.set_gravity_scale(scale, true);
            }
        }
    }

    /// Cast a ray from `origin` in `direction` up to `max_dist` meters.
    /// Returns the closest hit, if any.
    pub fn raycast(&self, origin: Vec3, direction: Vec3, max_dist: f32) -> Option<RaycastHit> {
        use rapier3d::geometry::Ray;
        use rapier3d::pipeline::QueryFilter;

        let ray = Ray::new(
            rapier3d::na::Point3::new(origin.x, origin.y, origin.z),
            vector![direction.x, direction.y, direction.z],
        );

        let (col_handle, intersection) = self.query_pipeline.cast_ray_and_get_normal(
            &self.inner.bodies,
            &self.inner.colliders,
            &ray,
            max_dist,
            true,
            QueryFilter::default(),
        )?;

        let collider = self.inner.colliders.get(col_handle)?;
        let rb_handle = collider.parent()?;
        let entity = *self.handle_to_entity.get(&rb_handle)?;

        let hit_point = ray.point_at(intersection.time_of_impact);
        let normal    = intersection.normal;

        Some(RaycastHit {
            entity,
            point:    Vec3::new(hit_point.x, hit_point.y, hit_point.z),
            normal:   Vec3::new(normal.x, normal.y, normal.z),
            distance: intersection.time_of_impact,
        })
    }

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
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .build();

        let collider_handle = self.inner.colliders
            .insert_with_parent(collider, body_handle, &mut self.inner.bodies);

        EntityEntry { body_handle, collider_handle }
    }
}
