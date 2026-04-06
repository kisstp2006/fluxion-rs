// ============================================================
// MaterialAsset — on-disk .fluxmat format
//
// This is the serializable descriptor for a material — what gets
// saved to disk. It contains only paths and scalar parameters.
// The GPU-resident PbrMaterial is created from this by the renderer.
//
// JSON schema is compatible with the TypeScript engine's FluxMatData
// so existing .fluxmat files can be loaded without conversion.
// ============================================================

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A scriptable shader parameter value.
/// Scripts can set these by name to modify materials at runtime.
///
/// Example (JS):
///   mesh.material.setFloat("u_speed", 2.0);
///   mesh.material.setTexture("u_lava", "textures/lava.png");
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum ShaderParamValue {
    Float(f32),
    Vec2([f32; 2]),
    Vec4([f32; 4]),
    Bool(bool),
    /// Asset path to a texture file.
    Texture(String),
}

/// How the material handles transparency.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AlphaMode {
    /// Fully opaque. Fastest.
    Opaque,
    /// Alpha test: discard pixels below `cutoff`. No blending needed.
    Mask(f32),
    /// Alpha blend: transparent. Requires back-to-front sorting.
    Blend,
}

/// UV transform: scale + offset for tiling/scrolling textures.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UvTransform {
    pub scale:  [f32; 2],
    pub offset: [f32; 2],
}

impl Default for UvTransform {
    fn default() -> Self { Self { scale: [1.0, 1.0], offset: [0.0, 0.0] } }
}

/// Serializable material descriptor. Stored as .fluxmat (JSON).
///
/// This is the "asset" side — the file on disk. The renderer creates a
/// GPU-resident PbrMaterial from this at load time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialAsset {
    pub name: String,

    // ── PBR scalar parameters ─────────────────────────────────────────────────
    /// Base color (linear RGBA). Multiplied with albedo texture if present.
    pub color: [f32; 4],
    pub roughness: f32,
    pub metalness: f32,
    pub emissive:  [f32; 3],
    pub emissive_intensity: f32,
    pub normal_scale: f32,       // normal map strength (1.0 = full effect)
    pub ao_intensity: f32,       // ambient occlusion strength

    // ── Blend / surface options ───────────────────────────────────────────────
    pub alpha_mode:  AlphaMode,
    pub double_sided: bool,
    pub wireframe:    bool,

    // ── Texture asset paths (relative to project root) ────────────────────────
    pub albedo_map:    Option<String>,
    pub normal_map:    Option<String>,
    pub roughness_map: Option<String>,
    pub metalness_map: Option<String>,
    pub ao_map:        Option<String>,
    pub emissive_map:  Option<String>,

    // ── Extended PBR (WickedEngine-style) ────────────────────────────────────
    /// Clearcoat layer strength (0 = disabled, 1 = full).
    #[serde(default)]
    pub clearcoat: f32,
    /// Clearcoat layer roughness.
    #[serde(default = "default_clearcoat_roughness")]
    pub clearcoat_roughness: f32,
    /// Sheen (fabric/velvet) color tint. Black = disabled.
    #[serde(default)]
    pub sheen_color: [f32; 3],
    /// Sheen lobe roughness.
    #[serde(default = "default_sheen_roughness")]
    pub sheen_roughness: f32,
    /// Anisotropy strength (-1..1). 0 = isotropic.
    #[serde(default)]
    pub anisotropy: f32,
    /// Subsurface scattering weight (0 = disabled, 1 = full SSS).
    #[serde(default)]
    pub subsurface: f32,
    /// Subsurface scattering color (diffuse color inside the surface).
    #[serde(default = "default_subsurface_color")]
    pub subsurface_color: [f32; 3],

    // ── UV transforms (keyed by slot name: "albedo", "normal", etc.) ─────────
    #[serde(default)]
    pub uv_transforms: HashMap<String, UvTransform>,

    // ── Scriptable / custom shader support ────────────────────────────────────
    /// Path to a custom WGSL shader file. If None, the built-in PBR shader is used.
    pub custom_shader: Option<String>,

    /// Named shader parameters for custom shaders or runtime script control.
    /// JS scripts can read/write these via material.params["name"].
    #[serde(default)]
    pub custom_params: HashMap<String, ShaderParamValue>,
}

fn default_clearcoat_roughness() -> f32 { 0.0 }
fn default_sheen_roughness()     -> f32 { 0.5 }
fn default_subsurface_color()    -> [f32; 3] { [1.0, 1.0, 1.0] }

impl Default for MaterialAsset {
    fn default() -> Self {
        Self {
            name:              "Default".to_string(),
            color:             [1.0, 1.0, 1.0, 1.0],
            roughness:         0.5,
            metalness:         0.0,
            emissive:          [0.0, 0.0, 0.0],
            emissive_intensity: 0.0,
            normal_scale:      1.0,
            ao_intensity:      1.0,
            alpha_mode:        AlphaMode::Opaque,
            double_sided:      false,
            wireframe:         false,
            albedo_map:        None,
            normal_map:        None,
            roughness_map:     None,
            metalness_map:     None,
            ao_map:            None,
            emissive_map:      None,
            clearcoat:          0.0,
            clearcoat_roughness: 0.0,
            sheen_color:        [0.0, 0.0, 0.0],
            sheen_roughness:    0.5,
            anisotropy:         0.0,
            subsurface:         0.0,
            subsurface_color:   [1.0, 1.0, 1.0],
            uv_transforms:     HashMap::new(),
            custom_shader:     None,
            custom_params:     HashMap::new(),
        }
    }
}

impl MaterialAsset {
    /// Build from a FluxionJS `MeshRenderer` embedded `material` JSON object.
    pub fn from_fluxionjs_mesh_material(v: &Value, name: impl Into<String>) -> Self {
        let mut m = MaterialAsset::default();
        m.name = name.into();
        if let Some(arr) = v.get("color").and_then(|c| c.as_array()) {
            if arr.len() >= 3 {
                m.color[0] = arr[0].as_f64().unwrap_or(1.0) as f32;
                m.color[1] = arr[1].as_f64().unwrap_or(1.0) as f32;
                m.color[2] = arr[2].as_f64().unwrap_or(1.0) as f32;
            }
        }
        m.roughness = v.get("roughness").and_then(|x| x.as_f64()).unwrap_or(0.6) as f32;
        m.metalness = v.get("metalness").and_then(|x| x.as_f64()).unwrap_or(0.1) as f32;
        if let Some(arr) = v.get("emissive").and_then(|c| c.as_array()) {
            if arr.len() >= 3 {
                m.emissive[0] = arr[0].as_f64().unwrap_or(0.0) as f32;
                m.emissive[1] = arr[1].as_f64().unwrap_or(0.0) as f32;
                m.emissive[2] = arr[2].as_f64().unwrap_or(0.0) as f32;
                m.emissive_intensity = v
                    .get("emissiveIntensity")
                    .and_then(|x| x.as_f64())
                    .unwrap_or(1.0) as f32;
            }
        }
        if v.get("transparent").and_then(|x| x.as_bool()).unwrap_or(false) {
            m.color[3] = v.get("opacity").and_then(|x| x.as_f64()).unwrap_or(1.0) as f32;
            m.alpha_mode = AlphaMode::Blend;
        }
        if v.get("doubleSided").and_then(|x| x.as_bool()).unwrap_or(false) {
            m.double_sided = true;
        }
        if v.get("wireframe").and_then(|x| x.as_bool()).unwrap_or(false) {
            m.wireframe = true;
        }
        if let Some(at) = v.get("alphaTest").and_then(|x| x.as_f64()) {
            m.alpha_mode = AlphaMode::Mask(at as f32);
        }
        m.normal_scale = v
            .get("normalScale")
            .and_then(|x| x.as_f64())
            .unwrap_or(1.0) as f32;
        m.ao_intensity = v
            .get("aoIntensity")
            .and_then(|x| x.as_f64())
            .unwrap_or(1.0) as f32;
        if let Some(s) = v.get("albedoMap").and_then(|x| x.as_str()) {
            m.albedo_map = Some(s.to_string());
        }
        if let Some(s) = v.get("normalMap").and_then(|x| x.as_str()) {
            m.normal_map = Some(s.to_string());
        }
        if let Some(s) = v.get("roughnessMap").and_then(|x| x.as_str()) {
            m.roughness_map = Some(s.to_string());
        }
        if let Some(s) = v.get("metalnessMap").and_then(|x| x.as_str()) {
            m.metalness_map = Some(s.to_string());
        }
        if let Some(s) = v.get("aoMap").and_then(|x| x.as_str()) {
            m.ao_map = Some(s.to_string());
        }
        if let Some(s) = v.get("emissiveMap").and_then(|x| x.as_str()) {
            m.emissive_map = Some(s.to_string());
        }
        m
    }

    /// Parse `.fluxmat` JSON from bytes (native disk, WASM memory, fetch).
    pub fn from_json_bytes(data: &[u8], label: &str) -> anyhow::Result<Self> {
        let raw = std::str::from_utf8(data)
            .map_err(|e| anyhow::anyhow!("Material '{label}' is not valid UTF-8: {e}"))?;
        serde_json::from_str(raw)
            .map_err(|e| anyhow::anyhow!("Failed to parse material '{label}': {e}"))
    }

    /// Load a .fluxmat JSON file from disk.
    pub fn load_from_file(path: &str) -> anyhow::Result<Self> {
        let raw = std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("Failed to read material '{}': {}", path, e))?;
        Self::from_json_bytes(&raw, path)
    }

    /// Save to a .fluxmat JSON file.
    pub fn save_to_file(&self, path: &str) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }
}
