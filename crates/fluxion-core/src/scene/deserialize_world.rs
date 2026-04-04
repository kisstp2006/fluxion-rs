// ============================================================
// fluxion-core — Instantiate SceneFileData into ECSWorld
//
// Compatible with FluxionJS V3 scene JSON (version ≥ 2).
// Unknown component types are skipped with a log line.
//
// Component deserialization is fully driven by a ComponentRegistry —
// the hard-coded match statement is gone. Call
// `registry.register_builtins()` to get the standard set, then
// optionally add custom types before passing the registry here.
// ============================================================

use std::collections::HashMap;

use log::warn;

use crate::ecs::entity::EntityId;
use crate::ecs::world::ECSWorld;
use crate::registry::ComponentRegistry;
use crate::transform::system::TransformSystem;

use super::{SceneFileData, SerializedComponent, SerializedEntity};

/// Load scene entities into `world`. When `clear_first` is true, removes all existing roots first.
/// Returns a map from serialized file entity id → runtime [`EntityId`].
pub fn load_scene_into_world(
    world: &mut ECSWorld,
    data: &SceneFileData,
    clear_first: bool,
    registry: &ComponentRegistry,
) -> Result<HashMap<u32, EntityId>, String> {
    if clear_first {
        world.clear();
    }

    let id_map = instantiate_entities(world, &data.entities, registry)?;

    ensure_active_camera(world);
    TransformSystem::update(world);

    Ok(id_map)
}

/// Spawn a topo-sorted list of serialized entities (scene slice or prefab). Does not clear the world.
/// File `id` values are only used for parenting inside this batch; they need not match existing ECS ids.
pub fn instantiate_entities(
    world: &mut ECSWorld,
    entities: &[SerializedEntity],
    registry: &ComponentRegistry,
) -> Result<HashMap<u32, EntityId>, String> {
    let mut id_map: HashMap<u32, EntityId> = HashMap::new();

    for ent in entities {
        let eid = world.spawn(Some(ent.name.as_str()));
        id_map.insert(ent.id, eid);

        for comp in &ent.components {
            if let Err(msg) = attach_component(world, eid, comp, registry) {
                warn!("[Scene] {}", msg);
            }
        }
    }

    for ent in entities {
        if let Some(pid) = ent.parent {
            let Some(&child) = id_map.get(&ent.id) else { continue };
            let parent = id_map.get(&pid).copied();
            world.set_parent(child, parent, false);
        }
    }

    TransformSystem::update(world);
    Ok(id_map)
}

fn attach_component(
    world: &mut ECSWorld,
    entity: EntityId,
    comp: &SerializedComponent,
    registry: &ComponentRegistry,
) -> Result<(), String> {
    match registry.attach(&comp.component_type, &comp.data, world, entity)? {
        true  => Ok(()),
        false => Err(format!("Unsupported component type \"{}\" — skipped", comp.component_type)),
    }
}

fn ensure_active_camera(world: &mut ECSWorld) {
    use crate::components::camera::Camera;

    let mut any_active = false;
    world.query_all::<&Camera, _>(|_, c| {
        if c.is_active { any_active = true; }
    });
    if any_active { return; }

    let mut first: Option<EntityId> = None;
    world.query_all::<&Camera, _>(|id, _| {
        if first.is_none() { first = Some(id); }
    });
    if let Some(id) = first {
        if let Some(mut cam) = world.get_component_mut::<Camera>(id) {
            cam.is_active = true;
        }
    }
}
