// ============================================================
// fluxion-core — Prefab system
//
// Prefabs are reusable entity templates stored as JSON files.
// Instances maintain a link back to their source prefab and
// track per-field overrides so that updating the prefab
// propagates to all instances (unless overridden).
// ============================================================

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::assets::database::new_guid;
use crate::ecs::entity::EntityId;
use crate::ecs::world::ECSWorld;
use crate::registry::ComponentRegistry;

use super::{instantiate_entities, SerializedEntity, SerializedComponent, topo_sort_entities};

/// Prefab root JSON: `version`, `name`, `entities` (same shape as scene entities).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrefabFileData {
    pub version: u32,
    pub name: String,
    pub entities: Vec<SerializedEntity>,
}

pub fn parse_prefab_json(json: &str) -> Result<PrefabFileData, String> {
    serde_json::from_str(json).map_err(|e| format!("Prefab JSON parse error: {e}"))
}

/// Spawn prefab entities into `world` without clearing. Returns file id → runtime entity map.
/// Each spawned entity is tagged with `prefab_source` so the editor can track the link.
pub fn spawn_prefab_into_world(
    world: &mut ECSWorld,
    prefab: &PrefabFileData,
    prefab_path: &str,
    registry: &ComponentRegistry,
) -> Result<HashMap<u32, EntityId>, String> {
    let sorted = topo_sort_entities(prefab.entities.clone())?;
    let id_map = instantiate_entities(world, &sorted, registry)?;

    for eid in id_map.values() {
        world.set_prefab_source(*eid, prefab_path.to_string());
    }

    Ok(id_map)
}

/// Compute which fields on a prefab instance differ from the source prefab.
/// Returns a map of "ComponentType.field_name" → current JSON value.
pub fn compute_overrides(
    world: &ECSWorld,
    entity: EntityId,
    prefab: &PrefabFileData,
    registry: &ComponentRegistry,
) -> HashMap<String, Value> {
    let mut overrides = HashMap::new();

    let Some(prefab_entity) = prefab.entities.first() else {
        return overrides;
    };

    let prefab_comp_map: HashMap<&str, &Value> = prefab_entity.components.iter()
        .map(|c| (c.component_type.as_str(), &c.data))
        .collect();

    for &comp_type in world.component_names(entity) {
        let Some(reflected) = registry.get_reflect(comp_type, world, entity) else { continue };
        let current_data = reflected.to_serialized_data();

        if let Some(&prefab_data) = prefab_comp_map.get(comp_type) {
            if let (Some(cur_obj), Some(pref_obj)) = (current_data.as_object(), prefab_data.as_object()) {
                for (field, cur_val) in cur_obj {
                    if let Some(pref_val) = pref_obj.get(field) {
                        if cur_val != pref_val {
                            let key = format!("{}.{}", comp_type, field);
                            overrides.insert(key, cur_val.clone());
                        }
                    }
                }
            }
        }
    }

    overrides
}

/// Revert a prefab instance to match its source prefab, clearing all overrides.
/// Reloads component data from the prefab file and re-applies it.
pub fn revert_to_prefab(
    world: &mut ECSWorld,
    entity: EntityId,
    prefab: &PrefabFileData,
    registry: &ComponentRegistry,
) -> Result<(), String> {
    let Some(prefab_entity) = prefab.entities.first() else {
        return Err("Prefab has no entities".to_string());
    };

    for comp in &prefab_entity.components {
        registry.attach(&comp.component_type, &comp.data, world, entity)?;
    }

    world.clear_prefab_overrides(entity);
    Ok(())
}

/// Apply changes from a prefab instance back to the source prefab file.
/// Serializes the entity's current state as the new prefab definition.
pub fn apply_to_prefab(
    world: &ECSWorld,
    entity: EntityId,
    registry: &ComponentRegistry,
) -> Result<PrefabFileData, String> {
    let entity_name = world.get_name(entity).to_string();
    let tags: Vec<String> = world.tags_of(entity).map(str::to_string).collect();

    let mut components = Vec::new();
    for &comp_type in world.component_names(entity) {
        if let Some(reflected) = registry.get_reflect(comp_type, world, entity) {
            components.push(SerializedComponent {
                component_type: comp_type.to_string(),
                data: reflected.to_serialized_data(),
            });
        }
    }

    Ok(PrefabFileData {
        version: 2,
        name: entity_name.clone(),
        entities: vec![SerializedEntity {
            id: 1,
            name: entity_name,
            uuid: world.get_uuid(entity).map(str::to_string).or_else(|| Some(new_guid())),
            parent: None,
            prefab_source: None,
            prefab_overrides: HashMap::new(),
            tags,
            components,
        }],
    })
}

/// Load a prefab file from disk.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_prefab_file(path: &str) -> Result<PrefabFileData, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read prefab '{path}': {e}"))?;
    parse_prefab_json(&raw)
}

/// Save a prefab file to disk (atomic write).
#[cfg(not(target_arch = "wasm32"))]
pub fn save_prefab_file(path: &str, prefab: &PrefabFileData) -> Result<(), String> {
    let json = serde_json::to_string_pretty(prefab)
        .map_err(|e| format!("Prefab serialize error: {e}"))?;
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, &json)
        .map_err(|e| format!("Failed to write '{tmp}': {e}"))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("Failed to rename prefab file: {e}"))?;
    Ok(())
}
