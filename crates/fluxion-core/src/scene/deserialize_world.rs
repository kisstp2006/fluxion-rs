// ============================================================
// fluxion-core — Instantiate SceneFileData into ECSWorld
//
// Compatible with FluxionJS V3 scene JSON (version ≥ 2). Unknown
// component types are skipped with a log line.
// ============================================================

use std::collections::HashMap;

use glam::{EulerRot, Quat, Vec3};
use log::warn;
use serde_json::Value;

use crate::components::camera::{Camera, ProjectionMode};
use crate::components::light::{Light, LightType};
use crate::components::mesh_renderer::{MeshRenderer, PrimitiveType};
use crate::ecs::entity::EntityId;
use crate::ecs::world::ECSWorld;
use crate::transform::Transform;
use crate::transform::system::TransformSystem;

use super::{SceneFileData, SerializedComponent};

/// Load scene entities into `world`. When `clear_first` is true, removes all existing roots first.
/// Returns a map from serialized file entity id → runtime [`EntityId`].
pub fn load_scene_into_world(
    world: &mut ECSWorld,
    data: &SceneFileData,
    clear_first: bool,
) -> Result<HashMap<u32, EntityId>, String> {
    if clear_first {
        world.clear();
    }

    let mut id_map: HashMap<u32, EntityId> = HashMap::new();

    for ent in &data.entities {
        let eid = world.spawn(Some(ent.name.as_str()));
        id_map.insert(ent.id, eid);

        for comp in &ent.components {
            if let Err(msg) = attach_component(world, eid, comp) {
                warn!("[Scene] {}", msg);
            }
        }
    }

    // Parents after all entities exist (file is topo-sorted; map may still need second pass)
    for ent in &data.entities {
        if let Some(pid) = ent.parent {
            let Some(&child) = id_map.get(&ent.id) else { continue };
            let parent = id_map.get(&pid).copied();
            world.set_parent(child, parent, false);
        }
    }

    ensure_active_camera(world);
    TransformSystem::update(world);

    Ok(id_map)
}

fn attach_component(world: &mut ECSWorld, entity: EntityId, comp: &SerializedComponent) -> Result<(), String> {
    match comp.component_type.as_str() {
        "Transform" => {
            let t = parse_transform(&comp.data).ok_or_else(|| format!("Bad Transform on {:?}", entity))?;
            world.add_component(entity, t);
        }
        "MeshRenderer" => {
            let m = parse_mesh_renderer(&comp.data).ok_or_else(|| format!("Bad MeshRenderer on {:?}", entity))?;
            world.add_component(entity, m);
        }
        "Camera" => {
            let c = parse_camera(&comp.data).ok_or_else(|| format!("Bad Camera on {:?}", entity))?;
            world.add_component(entity, c);
        }
        "Light" => {
            if let Some(lt) = comp.data.get("lightType").and_then(|v| v.as_str()) {
                if lt == "ambient" {
                    return Ok(()); // global ambient comes from scene settings / renderer
                }
            }
            let l = parse_light(&comp.data).ok_or_else(|| format!("Bad Light on {:?}", entity))?;
            world.add_component(entity, l);
        }
        other => {
            return Err(format!("Unsupported component type \"{}\" — skipped", other));
        }
    }
    Ok(())
}

fn parse_vec3_arr(v: &Value) -> Option<Vec3> {
    let a = v.as_array()?;
    if a.len() < 3 {
        return None;
    }
    Some(Vec3::new(
        a[0].as_f64()? as f32,
        a[1].as_f64()? as f32,
        a[2].as_f64()? as f32,
    ))
}

fn parse_transform(data: &Value) -> Option<Transform> {
    let mut t = Transform::new();
    if let Some(p) = data.get("position") {
        t.position = parse_vec3_arr(p)?;
    }
    if let Some(r) = data.get("rotation") {
        let e = parse_vec3_arr(r)?;
        t.rotation = Quat::from_euler(EulerRot::XYZ, e.x, e.y, e.z);
    }
    if let Some(s) = data.get("scale") {
        t.scale = parse_vec3_arr(s)?;
    }
    t.dirty = true;
    t.world_dirty = true;
    Some(t)
}

fn parse_mesh_renderer(data: &Value) -> Option<MeshRenderer> {
    let cast_shadow = data.get("castShadow").and_then(|v| v.as_bool()).unwrap_or(true);
    let receive_shadow = data.get("receiveShadow").and_then(|v| v.as_bool()).unwrap_or(true);
    let layer = data.get("layer").and_then(|v| v.as_u64()).unwrap_or(0) as u8;

    let model_path = data
        .get("modelPath")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let material_path = data
        .get("materialPath")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let inline = data.get("material").cloned();

    let primitive = if model_path.is_some() {
        None
    } else {
        let pt = data
            .get("primitiveType")
            .and_then(|v| v.as_str())
            .unwrap_or("cube")
            .to_ascii_lowercase();
        Some(map_primitive(&pt))
    };

    Some(MeshRenderer {
        mesh_path: model_path,
        material_path,
        primitive,
        cast_shadow,
        receive_shadow,
        layer,
        mesh_handle: None,
        material_handle: None,
        scene_inline_material: inline,
    })
}

fn map_primitive(pt: &str) -> PrimitiveType {
    match pt {
        "cube" | "box" => PrimitiveType::Cube,
        "sphere" => PrimitiveType::Sphere,
        "plane" => PrimitiveType::Plane,
        "cylinder" | "cone" => PrimitiveType::Cylinder,
        "capsule" => PrimitiveType::Capsule,
        "torus" => {
            warn!("[Scene] primitiveType \"torus\" not supported — using Cube");
            PrimitiveType::Cube
        }
        _ => PrimitiveType::Cube,
    }
}

fn parse_camera(data: &Value) -> Option<Camera> {
    let mut c = Camera::new();
    c.fov = data.get("fov").and_then(|v| v.as_f64()).unwrap_or(60.0) as f32;
    c.near = data.get("near").and_then(|v| v.as_f64()).unwrap_or(0.1) as f32;
    c.far = data.get("far").and_then(|v| v.as_f64()).unwrap_or(1000.0) as f32;
    if data.get("isOrthographic").and_then(|v| v.as_bool()).unwrap_or(false) {
        c.projection_mode = ProjectionMode::Orthographic;
    }
    c.ortho_size = data.get("orthoSize").and_then(|v| v.as_f64()).unwrap_or(10.0) as f32;
    c.is_active = data.get("isMain").and_then(|v| v.as_bool()).unwrap_or(false);
    // renderToTexture + RT paths are editor/runtime-specific; not applied here yet.
    Some(c)
}

fn parse_light(data: &Value) -> Option<Light> {
    let lt = data.get("lightType").and_then(|v| v.as_str()).unwrap_or("point");
    let light_type = match lt {
        "directional" => LightType::Directional,
        "point" => LightType::Point,
        "spot" => LightType::Spot,
        _ => LightType::Point,
    };

    let color = data
        .get("color")
        .and_then(parse_color_rgb)
        .unwrap_or([1.0, 1.0, 1.0]);

    let mut l = Light {
        light_type,
        color,
        intensity: data.get("intensity").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
        range: data.get("range").and_then(|v| v.as_f64()).unwrap_or(10.0) as f32,
        spot_angle: data.get("spotAngle").and_then(|v| v.as_f64()).unwrap_or(45.0) as f32,
        spot_penumbra: data.get("spotPenumbra").and_then(|v| v.as_f64()).unwrap_or(0.1) as f32,
        cast_shadow: data.get("castShadow").and_then(|v| v.as_bool()).unwrap_or(true),
        shadow_map_size: data
            .get("shadowMapSize")
            .and_then(|v| v.as_u64())
            .unwrap_or(2048) as u32,
        shadow_bias: data.get("shadowBias").and_then(|v| v.as_f64()).unwrap_or(-0.0001) as f32,
    };

    if light_type == LightType::Directional {
        l.range = f32::MAX;
    }

    Some(l)
}

fn parse_color_rgb(v: &Value) -> Option<[f32; 3]> {
    let a = v.as_array()?;
    if a.len() < 3 {
        return None;
    }
    Some([
        a[0].as_f64()? as f32,
        a[1].as_f64()? as f32,
        a[2].as_f64()? as f32,
    ])
}

fn ensure_active_camera(world: &mut ECSWorld) {
    let mut any_active = false;
    world.query_all::<&Camera, _>(|_, c| {
        if c.is_active {
            any_active = true;
        }
    });
    if any_active {
        return;
    }
    let mut first: Option<EntityId> = None;
    world.query_all::<&Camera, _>(|id, _| {
        if first.is_none() {
            first = Some(id);
        }
    });
    if let Some(id) = first {
        if let Some(mut cam) = world.get_component_mut::<Camera>(id) {
            cam.is_active = true;
        }
    }
}
