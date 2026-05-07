// ============================================================
// fluxion-core — Scene schema migration (E2 scaffolding)
//
// Walks an in-memory `SceneFileData` from its on-disk `version`
// to the current engine version, applying small fixers along the
// way. Each fixer is a `fn(&mut SceneFileData)` and is responsible
// for a single version step, so versions advance one at a time.
//
// On a stale-but-recognized version, the loader logs an info line
// and the next save writes the migrated file in the current format.
// On a future version (newer than the engine knows about), we log
// a warning and load anyway — better to display a partial scene
// than refuse to open it.
//
// The reflect-derive's `#[reflect(rename_from = "old_name")]`
// attribute handles per-field renames at the deserialize level
// without requiring an explicit migration step here, so most
// renames cost zero entries in this fixer chain.
// ============================================================

use super::SceneFileData;
use crate::assets::AssetDatabase;

/// Most recent on-disk format the engine writes. Bump together with the
/// matching fixer added to [`migrate`].
pub const CURRENT_SCENE_VERSION: u32 = 5;

/// Apply every necessary migration step to bring `scene.version` up to
/// [`CURRENT_SCENE_VERSION`]. Idempotent: a scene already at the current
/// version is left untouched.
///
/// `db` is required by some fixers (e.g. v4 → v5 path → GUID translation).
/// Pass `None` only when no project database is available — affected fixers
/// will skip their work and log a warning rather than panic.
pub fn migrate(scene: &mut SceneFileData, db: Option<&AssetDatabase>) {
    if scene.version == CURRENT_SCENE_VERSION {
        return;
    }
    if scene.version > CURRENT_SCENE_VERSION {
        log::warn!(
            "Scene version {} is newer than this engine ({}). Loading as-is — \
             some fields may be ignored or fall back to defaults.",
            scene.version, CURRENT_SCENE_VERSION,
        );
        return;
    }

    let from = scene.version;
    while scene.version < CURRENT_SCENE_VERSION {
        let next = scene.version + 1;
        match (scene.version, next) {
            (3, 4) => migrate_v3_to_v4(scene),
            (4, 5) => migrate_v4_to_v5(scene, db),
            // Older versions (0, 1, 2 → 3) are advanced trivially — no
            // schema changes occurred during those bumps. Per-field renames
            // are handled by `#[reflect(rename_from = "...")]` so they don't
            // need an entry here.
            _ => {}
        }
        scene.version = next;
    }

    log::info!("Scene migrated from v{from} to v{}", scene.version);
}

/// v3 → v4 — combined fixer:
///  1. **E3** Transform Euler → Quaternion unification.
///  2. **B5 / Week 6a** MeshRenderer key rename: `modelPath` → `mesh`,
///     `materialPath` → `material`. The old `material` key (which held an
///     inline material object) is moved to `inlineMaterial` so the new
///     `material` slot can refer to a material asset (string).
///
/// After this fixer runs, scenes are in canonical v4 layout and the
/// deserializers in `registry.rs` no longer need fallback paths.
fn migrate_v3_to_v4(scene: &mut super::SceneFileData) {
    use serde_json::Value;
    let mut quat_converted = 0usize;
    let mut mr_renamed = 0usize;
    for entity in &mut scene.entities {
        for comp in &mut entity.components {
            match comp.component_type.as_str() {
                "Transform" => {
                    let Some(obj) = comp.data.as_object_mut() else { continue };
                    let has_quat = obj.get("quaternion")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len() == 4)
                        .unwrap_or(false);
                    if has_quat {
                        // Already in canonical form — just drop the redundant Euler.
                        obj.remove("rotation");
                        continue;
                    }
                    // No quaternion yet — promote `rotation` (Euler XYZ) if present.
                    let euler = obj.get("rotation")
                        .and_then(|v| v.as_array())
                        .and_then(|a| {
                            if a.len() != 3 { return None; }
                            let x = a[0].as_f64()? as f32;
                            let y = a[1].as_f64()? as f32;
                            let z = a[2].as_f64()? as f32;
                            Some((x, y, z))
                        });
                    if let Some((rx, ry, rz)) = euler {
                        let q = glam::Quat::from_euler(glam::EulerRot::XYZ, rx, ry, rz);
                        obj.insert("quaternion".into(), Value::Array(vec![
                            Value::from(q.x), Value::from(q.y), Value::from(q.z), Value::from(q.w),
                        ]));
                        obj.remove("rotation");
                        quat_converted += 1;
                    }
                }
                "MeshRenderer" => {
                    let Some(obj) = comp.data.as_object_mut() else { continue };
                    // v3 had `material` as an INLINE material object. v4 uses
                    // `material` for the material-asset reference. Move the
                    // old inline data out of the way first so the rename
                    // below doesn't clobber it.
                    if let Some(v) = obj.remove("material") {
                        if v.is_object() {
                            obj.insert("inlineMaterial".into(), v);
                        }
                        // Strings are ignored — old scenes shouldn't have a
                        // string in `material`; if they do, drop it (no-op).
                    }
                    if let Some(v) = obj.remove("modelPath") {
                        obj.insert("mesh".into(), v);
                        mr_renamed += 1;
                    }
                    if let Some(v) = obj.remove("materialPath") {
                        obj.insert("material".into(), v);
                    }
                }
                _ => {}
            }
        }
    }
    if quat_converted > 0 {
        log::info!("[Scene v3→v4] converted {quat_converted} Transform rotations from Euler to quaternion.");
    }
    if mr_renamed > 0 {
        log::info!("[Scene v3→v4] renamed MeshRenderer keys (modelPath→mesh, materialPath→material) on {mr_renamed} components.");
    }
}

/// v4 → v5 — translate path-string asset references to GUIDs (B5 / Week 6b).
///
/// Components affected:
///   - `MeshRenderer.mesh` (was project-relative path → asset GUID)
///   - `MeshRenderer.material` (single-material reference)
///   - `MeshRenderer.materialSlots[].material` (per-submesh references)
///   - `LodGroup.levels[].mesh` (each LOD level's mesh reference)
///
/// Strings already shaped like a GUID (UUID v4 — 36 chars with 4 hyphens
/// at fixed positions) are left untouched, so re-running the fixer is
/// idempotent. Strings that resolve via [`AssetDatabase::guid_by_path`]
/// are replaced; unresolvable strings are kept as-is and logged so the
/// project author can investigate (most likely an asset was deleted or
/// the scene was authored before the asset was imported).
///
/// When `db` is `None` (e.g. unit tests, sandbox without a project) the
/// fixer logs a warning and skips translation; the values stay as paths,
/// which the runtime will fail to resolve at render time. That's an
/// explicit "don't migrate without a database" signal, not a silent fail.
fn migrate_v4_to_v5(scene: &mut super::SceneFileData, db: Option<&AssetDatabase>) {
    let Some(db) = db else {
        log::warn!("[Scene v4→v5] no AssetDatabase passed — skipping path→GUID translation.");
        return;
    };
    let mut translated = 0usize;
    let mut unresolved = 0usize;

    let mut translate = |s: &mut serde_json::Value| {
        let Some(text) = s.as_str() else { return };
        if text.is_empty() || looks_like_guid(text) {
            return;
        }
        match db.guid_by_path(text) {
            Some(guid) => {
                *s = serde_json::Value::String(guid.to_string());
                translated += 1;
            }
            None => {
                log::warn!("[Scene v4→v5] could not resolve path to GUID: '{text}' — left as-is.");
                unresolved += 1;
            }
        }
    };

    for entity in &mut scene.entities {
        for comp in &mut entity.components {
            let Some(obj) = comp.data.as_object_mut() else { continue };
            match comp.component_type.as_str() {
                "MeshRenderer" => {
                    if let Some(v) = obj.get_mut("mesh")     { translate(v); }
                    if let Some(v) = obj.get_mut("material") { translate(v); }
                    if let Some(slots) = obj.get_mut("materialSlots").and_then(|v| v.as_array_mut()) {
                        for slot in slots {
                            if let Some(slot_obj) = slot.as_object_mut() {
                                if let Some(v) = slot_obj.get_mut("material") { translate(v); }
                            }
                        }
                    }
                }
                "LodGroup" => {
                    if let Some(levels) = obj.get_mut("levels").and_then(|v| v.as_array_mut()) {
                        for level in levels {
                            if let Some(level_obj) = level.as_object_mut() {
                                if let Some(v) = level_obj.get_mut("mesh") { translate(v); }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if translated > 0 || unresolved > 0 {
        log::info!(
            "[Scene v4→v5] path→GUID: {translated} translated, {unresolved} unresolved.",
        );
    }
}

/// Heuristic: a 36-character string with hyphens at positions 8, 13, 18, 23
/// is treated as already in UUID form and skipped by the migrator.
fn looks_like_guid(s: &str) -> bool {
    s.len() == 36
        && s.as_bytes()[8]  == b'-'
        && s.as_bytes()[13] == b'-'
        && s.as_bytes()[18] == b'-'
        && s.as_bytes()[23] == b'-'
}
