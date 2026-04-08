// ============================================================
// CameraManager
//
// A plain (non-ECS) registry that tracks all Camera entities
// in the scene. Lives as a field on EditorHost and is rebuilt
// after every scene load, new-scene, or component add/remove.
//
// Responsibilities:
//   - Maintain a depth-sorted list of all Camera entity IDs.
//   - Track which entity has is_main = true.
//   - Enforce the single-main-camera invariant.
//   - Provide tag-based lookup for scripts.
// ============================================================

use crate::components::camera::Camera;
use crate::ecs::entity::EntityId;
use crate::ecs::world::ECSWorld;

/// Tracks all Camera entities in the scene.
///
/// This is **not** an ECS component. It is a plain struct stored
/// on `EditorHost` and rebuilt whenever cameras are added, removed,
/// or have their properties changed.
#[derive(Debug, Default)]
pub struct CameraManager {
    /// All Camera entity IDs sorted by `depth` ascending.
    cameras: Vec<EntityId>,
    /// The entity whose `is_main == true`, if any.
    main_camera: Option<EntityId>,
}

impl CameraManager {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Queries ───────────────────────────────────────────────────────────────

    /// Returns the entity ID of the main camera, or `None`.
    pub fn get_main(&self) -> Option<EntityId> {
        self.main_camera
    }

    /// Returns all Camera entity IDs sorted by depth ascending.
    pub fn get_all(&self) -> &[EntityId] {
        &self.cameras
    }

    /// Returns the number of tracked cameras.
    pub fn count(&self) -> usize {
        self.cameras.len()
    }

    /// Find the first camera entity whose `tag` matches `tag`.
    pub fn find_by_tag(&self, world: &ECSWorld, tag: &str) -> Option<EntityId> {
        self.cameras.iter().copied().find(|&id| {
            world.get_component::<Camera>(id)
                .map(|c| c.tag == tag)
                .unwrap_or(false)
        })
    }

    // ── Mutations ─────────────────────────────────────────────────────────────

    /// Full rescan — rebuilds the camera list and `main_camera` from the world.
    ///
    /// Call after:
    ///   - Scene load / new scene
    ///   - Entity despawn (if it could have been a camera)
    ///   - Any camera field change (depth change reorders the list)
    pub fn rebuild(&mut self, world: &ECSWorld) {
        // Collect all entities with a Camera component.
        let mut entries: Vec<(i32, EntityId)> = Vec::new();
        world.query_all::<&Camera, _>(|id, cam| {
            entries.push((cam.depth, id));
        });

        // Sort by depth ascending (lower depth = renders first / background).
        entries.sort_by_key(|(d, _)| *d);

        self.cameras = entries.iter().map(|(_, id)| *id).collect();

        // Determine main camera: prefer explicit is_main flag; fall back to
        // the lowest-depth active camera if none is flagged.
        self.main_camera = self.cameras.iter().copied().find(|&id| {
            world.get_component::<Camera>(id)
                .map(|c| c.is_main)
                .unwrap_or(false)
        });

        if self.main_camera.is_none() {
            // Fall back to first active camera.
            self.main_camera = self.cameras.iter().copied().find(|&id| {
                world.get_component::<Camera>(id)
                    .map(|c| c.is_active)
                    .unwrap_or(false)
            });
        }
    }

    /// Set `entity` as the main camera.
    ///
    /// Sets `is_main = true` on the target, clears it on all others,
    /// then rebuilds the internal list.
    pub fn set_main(&mut self, world: &mut ECSWorld, entity: EntityId) {
        // Clear is_main on every other camera.
        let others: Vec<EntityId> = self.cameras.iter()
            .copied()
            .filter(|&id| id != entity)
            .collect();
        for id in others {
            if let Some(mut cam) = world.get_component_mut::<Camera>(id) {
                cam.is_main = false;
            }
        }

        // Set is_main on the target.
        if let Some(mut cam) = world.get_component_mut::<Camera>(entity) {
            cam.is_main = true;
        }

        // Update cached main.
        self.main_camera = Some(entity);

        // Rebuild to refresh depth order.
        self.rebuild(world);
    }

    /// Notify the manager that a camera's depth may have changed.
    ///
    /// Cheaper than a full rebuild if only order needs updating.
    pub fn resort(&mut self, world: &ECSWorld) {
        // Re-read depths and sort.
        let mut entries: Vec<(i32, EntityId)> = self.cameras.iter()
            .map(|&id| {
                let depth = world.get_component::<Camera>(id)
                    .map(|c| c.depth)
                    .unwrap_or(0);
                (depth, id)
            })
            .collect();
        entries.sort_by_key(|(d, _)| *d);
        self.cameras = entries.into_iter().map(|(_, id)| id).collect();
    }
}
