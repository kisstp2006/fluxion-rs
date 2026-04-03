// ============================================================
// fluxion-core — TransformSystem
//
// Propagates dirty-flagged transform changes through the hierarchy
// using a BFS traversal from root entities. This ensures every parent
// is processed before its children (correct world matrix calculation).
//
// Algorithm (ported from fluxion-core/src/transform/mod.rs WASM bridge):
//   1. Collect all root entities (no parent)
//   2. BFS queue: for each entity in order:
//      a. If dirty: recompute local_matrix from position/rotation/scale
//         and mark world_dirty = true
//      b. If world_dirty: world_matrix = parent_world * local_matrix
//         Decompose into world_position/rotation/scale for convenience
//         Mark all direct children as world_dirty
//      c. Push children onto BFS queue
//   3. Done — all dirty flags cleared
//
// Call this before any system that reads world_matrix (renderer, physics sync).
// Call it TWICE per frame if using fixed timestep:
//   - Once after fixed update (physics writes positions)
//   - Once after variable update (script code writes positions)
// Or once after all updates if your scripts only write during variable update.
// ============================================================

use crate::ecs::world::ECSWorld;
use crate::transform::Transform;

/// Runs the BFS transform propagation pass.
///
/// Must be called after any code that modifies Transform position/rotation/scale.
/// The renderer calls `transform.world_matrix` which is only valid after this runs.
///
/// Cost: O(dirty entities) — static objects with unchanged transforms cost nothing.
pub struct TransformSystem;

impl TransformSystem {
    /// Execute one full transform propagation pass over the ECS world.
    pub fn update(world: &mut ECSWorld) {
        // ── Step 1: collect root entities ─────────────────────────────────────
        // We gather them into a Vec first to avoid holding a borrow on `world`
        // while we later need `&mut world` to read transform components.
        let roots: Vec<_> = world.root_entities().collect();

        // ── Step 2: BFS ────────────────────────────────────────────────────────
        // queue holds the BFS frontier. We grow it as we discover children.
        // Using a Vec as a queue (head index trick) avoids allocations
        // compared to a VecDeque — this is the same trick in the original WASM code.
        let mut queue: Vec<_> = roots;
        let mut head = 0usize;

        while head < queue.len() {
            let id = queue[head];
            head += 1;

            // Enqueue all children of this entity NOW (BFS order).
            // We do this before the transform computation so children
            // are visited after their parent regardless.
            let children: Vec<_> = world.get_children(id).collect();
            queue.extend_from_slice(&children);

            // ── Read parent's world matrix ────────────────────────────────────
            let parent_world: Option<glam::Mat4> = world
                .get_parent(id)
                .and_then(|parent| world.get_component::<Transform>(parent))
                .map(|t| t.world_matrix);

            // ── Update this entity's transform ────────────────────────────────
            // We need a mutable borrow. hecs provides this via get::<&mut T>.
            // The RefMut is dropped before we read the children again.
            let needs_child_propagation = {
                let mut t_ref = match world.get_component_mut::<Transform>(id) {
                    Some(t) => t,
                    None    => continue, // entity has no Transform, skip
                };
                let t = &mut *t_ref;

                // Recompute local matrix if local TRS changed
                if t.dirty {
                    t.local_matrix = glam::Mat4::from_scale_rotation_translation(
                        t.scale,
                        t.rotation,
                        t.position,
                    );
                    t.world_dirty = true;
                    t.dirty       = false;
                }

                // Recompute world matrix if this or any ancestor changed
                if t.world_dirty {
                    t.world_matrix = match parent_world {
                        Some(pw) => pw * t.local_matrix,
                        None     => t.local_matrix,
                    };

                    // Decompose world matrix into convenient world_position/rotation/scale.
                    // These are available as read-only properties for scripting.
                    let (ws, wr, wp) = t.world_matrix.to_scale_rotation_translation();
                    t.world_position = wp;
                    t.world_rotation = wr;
                    t.world_scale    = ws;

                    t.world_dirty = false;
                    true // children need their world_dirty set
                } else {
                    false
                }
            };

            // Propagate world_dirty to direct children if this entity's world changed.
            if needs_child_propagation {
                for child_id in &children {
                    if let Some(mut child_t) = world.get_component_mut::<Transform>(*child_id) {
                        child_t.world_dirty = true;
                    }
                }
            }
        }
    }
}
