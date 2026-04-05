// ============================================================
// fluxion-core — LodGroup component + LodSystem
//
// Attaches to entities that have multiple mesh-quality levels.
// Each LOD level specifies a screen-space distance threshold
// (in world units from the camera) and an optional mesh path.
//
// LodSystem runs each frame:
//   1. Finds all entities with LodGroup + Transform.
//   2. Computes distance to active camera.
//   3. Picks the appropriate LOD level (highest detail within budget).
//   4. Updates MeshRenderer::mesh_path + clears mesh_handle so the
//      renderer hydrates the new mesh on the next frame.
//
// Unity equivalent: LODGroup + LOD[].
// ============================================================

use serde::{Deserialize, Serialize};
use crate::ecs::component::Component;

// ── LodLevel ─────────────────────────────────────────────────────────────────

/// One detail level in a LOD group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LodLevel {
    /// Switch to this level when camera distance EXCEEDS this value.
    /// Level 0 (highest detail) should have the smallest threshold.
    pub screen_distance: f32,

    /// Mesh file path (e.g. "models/tree_lod1.glb").
    /// `None` = cull (don't render at this distance).
    pub mesh_path: Option<String>,
}

// ── LodGroup ──────────────────────────────────────────────────────────────────

/// LOD group component.  Attach alongside a `MeshRenderer`.
///
/// Levels should be ordered by increasing `screen_distance`.
/// The system picks the LAST level whose `screen_distance` is <= camera distance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LodGroup {
    /// LOD levels, ordered from closest (most detail) to farthest (least detail).
    pub levels: Vec<LodLevel>,

    /// Currently active level index. -1 = culled.
    #[serde(skip)]
    pub active_level: i32,
}

impl LodGroup {
    pub fn new(levels: Vec<LodLevel>) -> Self {
        Self { levels, active_level: 0 }
    }

    /// Two-level helper: high-detail mesh up to `threshold`, then low-detail.
    pub fn two_level(
        high_mesh: &str,
        low_mesh: &str,
        threshold: f32,
    ) -> Self {
        Self::new(vec![
            LodLevel { screen_distance: 0.0,      mesh_path: Some(high_mesh.to_string()) },
            LodLevel { screen_distance: threshold, mesh_path: Some(low_mesh.to_string()) },
        ])
    }

    /// High-detail → low-detail → culled.
    pub fn two_level_cull(
        high_mesh: &str,
        low_mesh: &str,
        switch_dist: f32,
        cull_dist: f32,
    ) -> Self {
        Self::new(vec![
            LodLevel { screen_distance: 0.0,        mesh_path: Some(high_mesh.to_string()) },
            LodLevel { screen_distance: switch_dist, mesh_path: Some(low_mesh.to_string()) },
            LodLevel { screen_distance: cull_dist,   mesh_path: None },
        ])
    }
}

impl Default for LodGroup {
    fn default() -> Self {
        Self { levels: Vec::new(), active_level: 0 }
    }
}

impl Component for LodGroup {}

// ── LodSystem ─────────────────────────────────────────────────────────────────

use glam::Vec3;

use crate::ecs::world::ECSWorld;
use crate::components::mesh_renderer::MeshRenderer;
use crate::transform::Transform;
use crate::components::camera::Camera;

pub struct LodSystem;

impl LodSystem {
    /// Update all LOD groups: pick the right level based on camera distance.
    pub fn update(world: &ECSWorld) {
        // Find the active camera position.
        let mut camera_pos = Vec3::ZERO;
        world.query_active::<(&Transform, &Camera), _>(|_, (t, cam)| {
            if cam.is_active {
                camera_pos = t.world_position;
            }
        });

        // Collect LOD group entities.
        let mut lod_entities: Vec<crate::EntityId> = Vec::new();
        world.query_all::<&LodGroup, _>(|id, _| lod_entities.push(id));

        for entity in lod_entities {
            // Get entity world position.
            let pos = {
                let Some(t) = world.get_component_mut::<Transform>(entity) else { continue };
                t.world_position
            };

            let dist = pos.distance(camera_pos);

            // Determine which level is active.
            let new_level = {
                let Some(lod) = world.get_component_mut::<LodGroup>(entity) else { continue };
                if lod.levels.is_empty() { continue; }

                // Walk levels: use the last one whose screen_distance <= dist.
                let mut selected = 0i32;
                for (i, level) in lod.levels.iter().enumerate() {
                    if dist >= level.screen_distance {
                        selected = i as i32;
                    }
                }
                selected
            };

            // Apply level change if needed.
            {
                let Some(mut lod) = world.get_component_mut::<LodGroup>(entity) else { continue };
                if lod.active_level == new_level { continue; }
                lod.active_level = new_level;
            }

            // Update the MeshRenderer mesh path.
            let new_path = {
                let Some(lod) = world.get_component_mut::<LodGroup>(entity) else { continue };
                lod.levels.get(new_level as usize)
                    .and_then(|l| l.mesh_path.clone())
            };

            if let Some(mut mr) = world.get_component_mut::<MeshRenderer>(entity) {
                let old_path = mr.mesh_path.clone();
                if old_path.as_deref() != new_path.as_deref() {
                    mr.mesh_path   = new_path;
                    mr.mesh_handle = None;   // force renderer to re-hydrate
                }
            }
        }
    }
}
