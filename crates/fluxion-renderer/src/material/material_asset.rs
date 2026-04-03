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
            uv_transforms:     HashMap::new(),
            custom_shader:     None,
            custom_params:     HashMap::new(),
        }
    }
}

impl MaterialAsset {
    /// Load a .fluxmat JSON file from disk.
    pub fn load_from_file(path: &str) -> anyhow::Result<Self> {
        let raw  = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read material '{}': {}", path, e))?;
        let mat: MaterialAsset = serde_json::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("Failed to parse material '{}': {}", path, e))?;
        Ok(mat)
    }

    /// Save to a .fluxmat JSON file.
    pub fn save_to_file(&self, path: &str) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }
}
