// ============================================================
// Script ↔ ECS Transform sync
//
// Behaviours registered with __fluxion_register(b, "EntityName") receive a plain
// `transform` object each frame: { position, rotation, scale }.
// `rotation` is **quaternion xyzw** (glam `Quat`), stable frame-to-frame — euler was
// dropped to avoid gimbal / round-trip jitter that made spins look stepped.
// Legacy 3-element rotation is still accepted on write-back as euler XYZ radians.
// ============================================================

use anyhow::Context;
use fluxion_core::{transform::Transform, ECSWorld};
use glam::{EulerRot, Quat, Vec3};
use serde_json::{json, Map, Value};

use crate::vm::JsVm;

const COLLECT_SRC: &str = r#"(() => JSON.stringify(
    __behaviours.reduce((acc, b) => {
        const n = b.__scriptTargetName;
        if (n && b.transform && typeof b.transform === "object") acc[n] = b.transform;
        return acc;
    }, {})
))()"#;

/// Push ECS transforms into `behaviour.transform` for all named script targets.
pub fn sync_transforms_from_world_to_scripts(vm: &JsVm, world: &ECSWorld) -> anyhow::Result<()> {
    let names_json = vm.eval_string_result(
        r#"JSON.stringify(__behaviours.map(b => b.__scriptTargetName).filter(n => n))"#,
        "<script-target-names>",
    )?;

    let names: Vec<String> = serde_json::from_str(&names_json).unwrap_or_default();
    if names.is_empty() {
        return Ok(());
    }

    let mut snap = Map::new();
    for name in names {
        if snap.contains_key(&name) {
            continue;
        }
        let Some(id) = world.find_by_name(&name) else {
            continue;
        };
        let Some(t) = world.get_component::<Transform>(id) else {
            continue;
        };
        let q = t.rotation;
        snap.insert(
            name,
            json!({
                "position": [t.position.x, t.position.y, t.position.z],
                "rotation": [q.x, q.y, q.z, q.w],
                "scale": [t.scale.x, t.scale.y, t.scale.z],
            }),
        );
    }

    let snap_json = serde_json::Value::Object(snap).to_string();
    vm.eval(
        &format!(
            r#"(() => {{
                const snap = {snap_json};
                for (const b of __behaviours) {{
                    const n = b.__scriptTargetName;
                    if (n && snap[n]) b.transform = snap[n];
                }}
            }})()"#
        ),
        "<inject-transform-snap>",
    )?;
    Ok(())
}

/// Apply `behaviour.transform` edits back to ECS (only entities with a script target name).
pub fn apply_transforms_from_scripts_to_world(vm: &JsVm, world: &mut ECSWorld) -> anyhow::Result<()> {
    let names_json = vm.eval_string_result(
        r#"JSON.stringify(__behaviours.map(b => b.__scriptTargetName).filter(n => n))"#,
        "<script-target-names>",
    )?;
    let names: Vec<String> = serde_json::from_str(&names_json).unwrap_or_default();
    if names.is_empty() {
        return Ok(());
    }

    let json_str = vm
        .eval_string_result(COLLECT_SRC, "<collect-script-transforms>")
        .context("collect script transforms")?;

    let value: Value = serde_json::from_str(&json_str).unwrap_or(json!({}));
    let obj = value.as_object().cloned().unwrap_or_default();

    for name in names {
        let Some(id) = world.find_by_name(&name) else {
            continue;
        };
        let Some(ent) = obj.get(&name) else {
            continue;
        };
        let Some(mut t) = world.get_component_mut::<Transform>(id) else {
            continue;
        };

        if let Some(arr) = ent.get("position").and_then(|v| v.as_array()) {
            if arr.len() >= 3 {
                t.position = Vec3::new(
                    arr[0].as_f64().unwrap_or(0.0) as f32,
                    arr[1].as_f64().unwrap_or(0.0) as f32,
                    arr[2].as_f64().unwrap_or(0.0) as f32,
                );
                t.dirty = true;
            }
        }
        if let Some(arr) = ent.get("rotation").and_then(|v| v.as_array()) {
            if arr.len() == 4 {
                let q = Quat::from_xyzw(
                    arr[0].as_f64().unwrap_or(0.0) as f32,
                    arr[1].as_f64().unwrap_or(0.0) as f32,
                    arr[2].as_f64().unwrap_or(0.0) as f32,
                    arr[3].as_f64().unwrap_or(1.0) as f32,
                );
                t.rotation = if q.length_squared() > 1e-10 {
                    q.normalize()
                } else {
                    Quat::IDENTITY
                };
                t.dirty = true;
            } else if arr.len() >= 3 {
                let rx = arr[0].as_f64().unwrap_or(0.0) as f32;
                let ry = arr[1].as_f64().unwrap_or(0.0) as f32;
                let rz = arr[2].as_f64().unwrap_or(0.0) as f32;
                t.rotation = Quat::from_euler(EulerRot::XYZ, rx, ry, rz);
                t.dirty = true;
            }
        }
        if let Some(arr) = ent.get("scale").and_then(|v| v.as_array()) {
            if arr.len() >= 3 {
                t.scale = Vec3::new(
                    arr[0].as_f64().unwrap_or(1.0) as f32,
                    arr[1].as_f64().unwrap_or(1.0) as f32,
                    arr[2].as_f64().unwrap_or(1.0) as f32,
                );
                t.dirty = true;
            }
        }
    }

    Ok(())
}
