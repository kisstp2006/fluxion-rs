// ============================================================
// fluxion-core — Transform component
//
// Every renderable entity has a Transform. It stores:
//   - LOCAL position/rotation/scale (the values you set)
//   - WORLD position/rotation/scale (computed by TransformSystem)
//   - Dirty flags so TransformSystem only recomputes what changed
//
// The separation of local vs world mirrors Unity's Transform component.
// Local values are the "source of truth" that get serialized.
// World values are cached derived data that the renderer reads.
//
// Dirty flag propagation:
//   1. You change position/rotation/scale → set dirty = true
//   2. TransformSystem sees dirty=true → recomputes local_matrix
//   3. TransformSystem sets world_dirty = true on this entity AND children
//   4. TransformSystem recomputes world_matrix from parent_world * local_matrix
//   5. Clears both flags
//
// This avoids recomputing the world matrix every frame for static objects.
// ============================================================

pub mod system;

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::ecs::component::Component;

/// Transform component.
///
/// Attach this to every entity that has a position in 3D space.
///
/// # Example
/// ```rust
/// let mut t = Transform::new();
/// t.position = Vec3::new(1.0, 2.0, 3.0);
/// t.dirty = true; // tell TransformSystem to recompute matrices
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transform {
    // ── Local space (serialized — these are the values you author) ─────────────
    pub position: Vec3,
    pub rotation: Quat,
    pub scale:    Vec3,

    // ── Dirty flags (not serialized — runtime state) ───────────────────────────
    /// Set to `true` whenever position/rotation/scale changes.
    /// TransformSystem recomputes `local_matrix` and clears this.
    #[serde(skip)]
    pub dirty: bool,

    /// Set to `true` when this entity's world matrix needs recomputing
    /// (either `dirty` is set, or the parent's world matrix changed).
    /// TransformSystem recomputes `world_matrix` and clears this.
    #[serde(skip)]
    pub world_dirty: bool,

    // ── World-space cache (not serialized — computed by TransformSystem) ────────
    /// World-space position. Read-only for external code.
    /// Written exclusively by TransformSystem.
    #[serde(skip)]
    pub world_position: Vec3,

    /// World-space rotation.
    #[serde(skip)]
    pub world_rotation: Quat,

    /// World-space scale.
    #[serde(skip)]
    pub world_scale: Vec3,

    // ── Matrix cache (not serialized) ──────────────────────────────────────────
    /// Local TRS matrix. Computed from position/rotation/scale.
    #[serde(skip)]
    pub(crate) local_matrix: Mat4,

    /// World matrix = parent_world * local_matrix.
    /// This is what the renderer uploads to the GPU.
    #[serde(skip)]
    pub world_matrix: Mat4,
}

impl Transform {
    /// Create a default transform at the origin with no rotation and unit scale.
    pub fn new() -> Self {
        Transform {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale:    Vec3::ONE,
            dirty:       true, // start dirty so first frame computes matrices
            world_dirty: true,
            world_position: Vec3::ZERO,
            world_rotation: Quat::IDENTITY,
            world_scale:    Vec3::ONE,
            local_matrix: Mat4::IDENTITY,
            world_matrix: Mat4::IDENTITY,
        }
    }

    /// Create a transform at a specific position.
    pub fn from_position(position: Vec3) -> Self {
        Transform { position, dirty: true, world_dirty: true, ..Self::new() }
    }

    /// Create a transform from position + rotation.
    pub fn from_position_rotation(position: Vec3, rotation: Quat) -> Self {
        Transform { position, rotation, dirty: true, world_dirty: true, ..Self::new() }
    }

    // ── Convenience setters (auto-set dirty flag) ─────────────────────────────

    /// Set local position and mark dirty.
    pub fn set_position(&mut self, pos: Vec3) {
        self.position = pos;
        self.dirty = true;
    }

    /// Set local rotation and mark dirty.
    pub fn set_rotation(&mut self, rot: Quat) {
        self.rotation = rot;
        self.dirty = true;
    }

    /// Set local scale and mark dirty.
    pub fn set_scale(&mut self, scale: Vec3) {
        self.scale = scale;
        self.dirty = true;
    }

    /// Rotate so that the entity's -Z axis points toward `target`.
    /// `up` is the world-up vector used to compute the right axis.
    ///
    /// Unity equivalent: `transform.LookAt(target)`
    pub fn look_at(&mut self, target: Vec3, up: Vec3) {
        let forward = (target - self.position).normalize_or_zero();
        if forward.length_squared() < 1e-6 {
            return; // target is at same position, ignore
        }
        self.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, forward);
        // Correct for up vector using a secondary rotation
        let right   = forward.cross(up).normalize_or_zero();
        let true_up = right.cross(forward);
        let _ = true_up; // full look-at with roll correction would go here
        self.dirty = true;
    }

    /// Translate in world space. Requires world_position to be up-to-date.
    pub fn translate_world(&mut self, delta: Vec3) {
        // We modify local position such that world position shifts by delta.
        // For a root entity: local = world, so this is just position += delta.
        // For a child: local += inverse(parent_world_rotation) * delta.
        // TransformSystem will handle this on next update.
        self.position += delta;
        self.dirty = true;
    }

    /// Get the forward direction in local space (-Z in right-hand coordinates).
    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }

    /// Get the right direction in local space (+X).
    pub fn right(&self) -> Vec3 {
        self.rotation * Vec3::X
    }

    /// Get the up direction in local space (+Y).
    pub fn up(&self) -> Vec3 {
        self.rotation * Vec3::Y
    }

    /// Get the world-space forward direction.
    /// Requires TransformSystem to have run this frame.
    pub fn world_forward(&self) -> Vec3 {
        self.world_rotation * Vec3::NEG_Z
    }

    /// Get the world-space right direction.
    pub fn world_right(&self) -> Vec3 {
        self.world_rotation * Vec3::X
    }

    /// Get the world-space up direction.
    pub fn world_up(&self) -> Vec3 {
        self.world_rotation * Vec3::Y
    }
}

impl Default for Transform {
    fn default() -> Self { Self::new() }
}

// Transform is a Component. The lifecycle hooks are no-ops for Transform
// since it has no external resources to acquire/release.
impl Component for Transform {}
