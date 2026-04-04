// ============================================================
// fluxion-core — Unity-style facade (thin layer over ECS)
//
// Exposes familiar names (`GameObject`) and helpers without hiding
// the underlying `ECSWorld` + `hecs` model.
//
// Script lifecycle (QuickJS `FluxionBehaviour`): `start`, `update`,
// `lateUpdate`, `fixedUpdate`, `onDestroy` — mirror Unity's order when
// used from JS; native Rust systems typically use `TransformSystem` and
// custom schedules instead of MonoBehaviour.
// ============================================================

use glam::{Quat, Vec3};

use crate::ecs::entity::EntityId;
use crate::ecs::world::ECSWorld;
use crate::transform::Transform;

/// Handle to a single entity, Unity `GameObject`-style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GameObject {
    pub id: EntityId,
}

impl GameObject {
    /// First entity with an exact name match. O(n) over named entities.
    pub fn find(world: &ECSWorld, name: &str) -> Option<Self> {
        world.find_by_name(name).map(|id| GameObject { id })
    }

    pub fn is_alive(self, world: &ECSWorld) -> bool {
        world.is_alive(self.id)
    }

    pub fn name<'a>(self, world: &'a ECSWorld) -> &'a str {
        world.get_name(self.id)
    }

    pub fn set_name(self, world: &mut ECSWorld, name: &str) {
        world.set_name(self.id, name);
    }

    pub fn transform<'a>(self, world: &'a ECSWorld) -> Option<hecs::Ref<'a, Transform>> {
        world.get_component(self.id)
    }

    pub fn transform_mut<'a>(self, world: &'a ECSWorld) -> Option<hecs::RefMut<'a, Transform>> {
        world.get_component_mut(self.id)
    }

    /// Local position (reads `Transform.position`).
    pub fn local_position(self, world: &ECSWorld) -> Option<Vec3> {
        self.transform(world).map(|t| t.position)
    }

    pub fn set_local_position(self, world: &ECSWorld, pos: Vec3) -> bool {
        if let Some(mut t) = self.transform_mut(world) {
            t.position = pos;
            t.dirty = true;
            true
        } else {
            false
        }
    }

    /// Sets local rotation from Euler angles in radians (XYZ order).
    pub fn set_local_euler(self, world: &ECSWorld, euler: Vec3) -> bool {
        if let Some(mut t) = self.transform_mut(world) {
            t.rotation = Quat::from_euler(glam::EulerRot::XYZ, euler.x, euler.y, euler.z);
            t.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn set_parent(self, world: &mut ECSWorld, parent: Option<GameObject>, world_position_stays: bool) {
        world.set_parent(self.id, parent.map(|p| p.id), world_position_stays);
    }
}
