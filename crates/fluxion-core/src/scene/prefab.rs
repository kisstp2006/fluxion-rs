// ============================================================
// fluxion-core — Prefab JSON (minimal MVP)
//
// Same entity/component schema as scene files; no settings block.
// Parse, topo-sort, then [`crate::scene::deserialize_world::instantiate_entities`].
// ============================================================

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::ecs::entity::EntityId;
use crate::ecs::world::ECSWorld;

use super::{instantiate_entities, SerializedEntity, topo_sort_entities};

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
pub fn spawn_prefab_into_world(
    world: &mut ECSWorld,
    prefab: &PrefabFileData,
) -> Result<HashMap<u32, EntityId>, String> {
    let sorted = topo_sort_entities(prefab.entities.clone())?;
    instantiate_entities(world, &sorted)
}
