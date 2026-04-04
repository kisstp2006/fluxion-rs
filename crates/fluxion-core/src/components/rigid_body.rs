// ============================================================
// fluxion-core — RigidBody component (no rapier dependency)
//
// Pure data component. The actual Rapier simulation lives in
// fluxion-physics::PhysicsEcsWorld, which reads these values
// to create and configure Rapier rigid bodies each frame.
//
// Kept in core so scene JSON, the reflect system, and the editor
// can all work with RigidBody without importing rapier3d.
// ============================================================

use serde::{Deserialize, Serialize};

use crate::ecs::component::Component;

/// The shape of the physics collider attached to this entity.
///
/// Serialized as a tagged object: `{ "type": "sphere", "radius": 0.5 }`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PhysicsShape {
    /// Axis-aligned box. `half_extents` is half the size in each axis (X, Y, Z).
    Box { half_extents: [f32; 3] },

    /// Sphere. `radius` is the sphere radius.
    Sphere { radius: f32 },

    /// Capsule aligned along the Y axis.
    /// `half_height` is half the height of the cylindrical part;
    /// `radius` is the hemisphere radius at each end.
    Capsule { half_height: f32, radius: f32 },

    /// Infinite static half-space. Normal points +Y (floor).
    /// Only valid on `Static` bodies. Use for ground planes.
    HalfSpace,
}

impl Default for PhysicsShape {
    fn default() -> Self {
        PhysicsShape::Box { half_extents: [0.5, 0.5, 0.5] }
    }
}

impl PhysicsShape {
    pub fn as_str(&self) -> &'static str {
        match self {
            PhysicsShape::Box { .. }      => "Box",
            PhysicsShape::Sphere { .. }   => "Sphere",
            PhysicsShape::Capsule { .. }  => "Capsule",
            PhysicsShape::HalfSpace       => "HalfSpace",
        }
    }
}

/// Whether the body is fully simulated, script-driven, or immovable.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum BodyType {
    /// Rapier applies forces, gravity, and collision response. Default.
    #[default]
    Dynamic,
    /// Position is driven by the ECS (scripts, animation). Rapier still
    /// resolves collisions against this body but doesn't apply forces.
    Kinematic,
    /// Never moves. Zero simulation cost. Use for walls, floors, terrain.
    Static,
}

impl BodyType {
    pub fn as_str(&self) -> &'static str {
        match self {
            BodyType::Dynamic   => "Dynamic",
            BodyType::Kinematic => "Kinematic",
            BodyType::Static    => "Static",
        }
    }
}

/// ECS component for physics simulation.
///
/// Add to any entity that also has a `Transform` to make it participate
/// in the Rapier physics simulation managed by `PhysicsEcsWorld`.
///
/// # Frame lifecycle
/// ```text
/// PhysicsEcsWorld::sync_from_ecs(&world)  // creates Rapier body for new RigidBodies
/// PhysicsEcsWorld::step(dt)               // advances the simulation
/// PhysicsEcsWorld::sync_to_ecs(&world)    // writes positions back to Transform
/// TransformSystem::update(&mut world)     // propagates dirty flags through hierarchy
/// ```
///
/// # Scene JSON example
/// ```json
/// { "type": "RigidBody", "data": {
///     "bodyType": "Dynamic",
///     "shape": { "type": "sphere", "radius": 0.5 },
///     "mass": 1.0, "restitution": 0.3, "friction": 0.5
/// }}
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RigidBody {
    /// Collider shape.
    pub shape: PhysicsShape,
    /// Simulation mode.
    pub body_type: BodyType,
    /// Mass in kilograms. Ignored for Static bodies.
    pub mass: f32,
    /// Linear velocity damping coefficient (air resistance). Range: [0, ∞).
    pub linear_damping: f32,
    /// Angular velocity damping coefficient. Range: [0, ∞).
    pub angular_damping: f32,
    /// Gravity multiplier. 0 = zero gravity, 1 = normal, negative = anti-gravity.
    pub gravity_scale: f32,
    /// Whether the body may sleep when at rest (reduces CPU load).
    pub can_sleep: bool,
    /// Bounciness coefficient. Range: [0, 1].
    pub restitution: f32,
    /// Friction coefficient. Range: [0, ∞); typically [0, 1].
    pub friction: f32,
}

impl Default for RigidBody {
    fn default() -> Self {
        RigidBody {
            shape:           PhysicsShape::default(),
            body_type:       BodyType::Dynamic,
            mass:            1.0,
            linear_damping:  0.0,
            angular_damping: 0.0,
            gravity_scale:   1.0,
            can_sleep:       true,
            restitution:     0.0,
            friction:        0.5,
        }
    }
}

impl Component for RigidBody {}
