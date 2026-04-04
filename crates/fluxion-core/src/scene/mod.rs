// ============================================================
// fluxion-core — Scene serialization
//
// Mirrors the existing scene/mod.rs from the TypeScript engine's
// fluxion-core Rust bridge, with the same JSON format so existing
// .scene files remain compatible.
//
// For 1:1 TS field comparison, keep FluxionJsV3/src (engine runtime) alongside this repo when available.
//
// Key operations:
//   parse_and_sort_scene()  — parse JSON + topo-sort entities (parents first)
//   serialize_scene()       — serialize SceneFileData back to JSON
//   load_scene_file()       — read + parse from disk (native only)
//   save_scene_file()       — serialize + atomically write to disk (native only)
//
// The topo-sort (Kahn's algorithm) ensures that when we deserialize,
// we always process parent entities before their children, which is
// required for correct set_parent() calls during scene loading.
// ============================================================

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};

// ── Data types (JSON-compatible with existing .scene files) ───────────────────

/// Top-level scene file structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneFileData {
    pub name:    String,
    pub version: u32,
    pub settings: SceneSettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_camera: Option<EditorCamera>,
    pub entities: Vec<SerializedEntity>,
}

/// Global scene settings (ambient light, fog, gravity, skybox).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneSettings {
    pub ambient_color:     [f32; 3],
    pub ambient_intensity: f32,
    pub fog_enabled:       bool,
    pub fog_color:         [f32; 3],
    pub fog_density:       f32,
    pub skybox:            Option<String>,
    pub physics_gravity:   [f32; 3],
    /// Unknown future settings fields pass through without error.
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl Default for SceneSettings {
    fn default() -> Self {
        SceneSettings {
            ambient_color:     [0.2, 0.2, 0.3],
            ambient_intensity: 0.3,
            fog_enabled:       false,
            fog_color:         [0.5, 0.6, 0.7],
            fog_density:       0.01,
            skybox:            None,
            physics_gravity:   [0.0, -9.81, 0.0],
            extra:             HashMap::new(),
        }
    }
}

/// Saved editor camera state (not part of the runtime scene).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorCamera {
    pub position: [f32; 3],
    pub target:   [f32; 3],
    pub fov:      f32,
}

/// A single serialized entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedEntity {
    pub id:     u32,
    pub name:   String,
    pub parent: Option<u32>,
    pub tags:   Vec<String>,
    pub components: Vec<SerializedComponent>,
}

/// A single serialized component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedComponent {
    /// Component type identifier string, e.g. "Transform", "MeshRenderer".
    #[serde(rename = "type")]
    pub component_type: String,
    /// Arbitrary JSON data — deserialized by component-specific code.
    pub data: Value,
}

// ── Core operations ────────────────────────────────────────────────────────────

/// Parse scene JSON and topologically sort entities (parents before children).
/// Returns an error string if the JSON is malformed or the hierarchy has cycles.
pub fn parse_and_sort_scene(json: &str) -> Result<SceneFileData, String> {
    let mut scene: SceneFileData = serde_json::from_str(json)
        .map_err(|e| format!("Scene JSON parse error: {e}"))?;
    scene.entities = topo_sort_entities(scene.entities)?;
    Ok(scene)
}

/// Load a `.scene` from raw bytes (native + WASM). Same JSON as [`parse_and_sort_scene`].
pub fn load_scene_from_bytes(data: &[u8]) -> Result<SceneFileData, String> {
    let text = std::str::from_utf8(data).map_err(|e| format!("Scene file is not valid UTF-8: {e}"))?;
    parse_and_sort_scene(text)
}

/// Serialize a SceneFileData to a pretty-printed JSON string.
pub fn serialize_scene(scene: &SceneFileData) -> Result<String, String> {
    serde_json::to_string_pretty(scene)
        .map_err(|e| format!("Scene serialize error: {e}"))
}

/// Topologically sort entities so every parent appears before its children.
/// Detects cycles (returns Err). Handles missing-parent references by
/// treating orphaned entities as roots.
///
/// Uses Kahn's algorithm (BFS over the dependency graph).
pub fn topo_sort_entities(entities: Vec<SerializedEntity>) -> Result<Vec<SerializedEntity>, String> {
    if entities.is_empty() { return Ok(entities); }

    let n = entities.len();
    let mut id_to_idx: HashMap<u32, usize> = HashMap::with_capacity(n);
    for (i, e) in entities.iter().enumerate() {
        id_to_idx.insert(e.id, i);
    }

    let mut children:  Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_degree: Vec<u32>         = vec![0; n];

    for (i, e) in entities.iter().enumerate() {
        if let Some(pid) = e.parent {
            if let Some(&pi) = id_to_idx.get(&pid) {
                children[pi].push(i);
                in_degree[i] += 1;
            }
            // Unknown parent → treat as root (orphan)
        }
    }

    let mut queue: VecDeque<usize> = VecDeque::with_capacity(n);
    for i in 0..n {
        if in_degree[i] == 0 { queue.push_back(i); }
    }

    let mut order: Vec<usize> = Vec::with_capacity(n);
    while let Some(i) = queue.pop_front() {
        order.push(i);
        for &child in &children[i] {
            in_degree[child] -= 1;
            if in_degree[child] == 0 { queue.push_back(child); }
        }
    }

    if order.len() != n {
        return Err(format!(
            "Scene hierarchy cycle detected ({} of {} entities unreachable after sort)",
            n - order.len(), n
        ));
    }

    let mut result: Vec<Option<SerializedEntity>> = entities.into_iter().map(Some).collect();
    Ok(order.into_iter().map(|i| result[i].take().unwrap()).collect())
}

// ── Native file I/O (not available on WASM — use fetch API instead) ───────────

#[cfg(not(target_arch = "wasm32"))]
pub fn load_scene_file(path: &str) -> Result<SceneFileData, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read scene '{path}': {e}"))?;
    parse_and_sort_scene(&raw)
}

/// Atomically write a scene file. Writes to a temp file then renames,
/// so a crash mid-write doesn't corrupt the original file.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_scene_file(path: &str, scene: &SceneFileData) -> Result<(), String> {
    let json = serialize_scene(scene)?;
    let tmp  = format!("{path}.tmp");
    std::fs::write(&tmp, &json)
        .map_err(|e| format!("Failed to write '{tmp}': {e}"))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("Failed to rename scene file: {e}"))?;
    Ok(())
}

// ── World → SceneFileData serialization ───────────────────────────────────────

/// Serialize a live [`ECSWorld`] into a [`SceneFileData`] ready for [`save_scene_file`].
///
/// All entities in the world are included. Components that have a reflect accessor
/// registered in `registry` are serialized via [`crate::reflect::Reflect::to_serialized_data`].
/// Components without a reflect accessor are skipped with a `warn!` log.
///
/// # Entity file-ID assignment
/// Each entity receives a sequential `u32` file-ID (starting at 1) in the order
/// returned by [`crate::ecs::world::ECSWorld::all_entities`]. These IDs are local
/// to this call and used only for the parent-child relationship in the JSON.
#[cfg(not(target_arch = "wasm32"))]
pub fn world_to_scene_data(
    world:         &crate::ecs::world::ECSWorld,
    registry:      &crate::registry::ComponentRegistry,
    name:          String,
    settings:      SceneSettings,
    editor_camera: Option<EditorCamera>,
) -> SceneFileData {
    // Step 1: assign each entity a sequential file-ID.
    let mut entity_to_file_id: HashMap<crate::ecs::entity::EntityId, u32> = HashMap::new();
    let mut next_id: u32 = 1;
    for eid in world.all_entities() {
        entity_to_file_id.insert(eid, next_id);
        next_id += 1;
    }

    // Step 2: build a SerializedEntity for every entity.
    let mut entities: Vec<SerializedEntity> = Vec::new();

    for eid in world.all_entities() {
        let file_id    = entity_to_file_id[&eid];
        let entity_name = world.get_name(eid).to_string();

        let parent_file_id = world
            .get_parent(eid)
            .and_then(|pid| entity_to_file_id.get(&pid).copied());

        let tags: Vec<String> = world.tags_of(eid).map(str::to_string).collect();

        let mut components: Vec<SerializedComponent> = Vec::new();
        for &comp_type in world.component_names(eid) {
            if let Some(reflected) = registry.get_reflect(comp_type, world, eid) {
                components.push(SerializedComponent {
                    component_type: comp_type.to_string(),
                    data:           reflected.to_serialized_data(),
                });
            } else {
                log::warn!(
                    "[world_to_scene_data] No reflect accessor for '{}' on entity '{}' — skipped",
                    comp_type, entity_name
                );
            }
        }

        entities.push(SerializedEntity {
            id:     file_id,
            name:   entity_name,
            parent: parent_file_id,
            tags,
            components,
        });
    }

    SceneFileData { name, version: 2, settings, editor_camera, entities }
}

mod deserialize_world;
mod prefab;

pub use deserialize_world::{instantiate_entities, load_scene_into_world};
pub use prefab::{parse_prefab_json, spawn_prefab_into_world, PrefabFileData};

// Re-export ComponentRegistry here so scene users don't need to know about registry module.
pub use crate::registry::ComponentRegistry;

