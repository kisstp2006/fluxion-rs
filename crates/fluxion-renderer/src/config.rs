// ============================================================
// fluxion-renderer — RendererConfig
//
// All renderer tuning parameters in one serializable struct.
// Load from `renderer.config.json` at startup; falls back to
// sane defaults if the file is absent or a field is missing.
//
// JSON example (renderer.config.json):
// {
//   "maxLights": 64,
//   "bloom":   { "enabled": true, "threshold": 1.2, "strength": 0.25, "blurPasses": 4 },
//   "tonemap": { "exposure": 0.7, "vignetteIntensity": 0.25, "filmGrain": 0.01 },
//   "ssao":    { "enabled": true, "radius": 0.5, "intensity": 1.0 },
//   "passes":  { "skybox": true, "ssao": true, "bloom": true, "particles": true }
// }
//
// All fields are optional in JSON — missing keys use the Default value.
// ============================================================

use serde::{Deserialize, Serialize};

// ── Sub-configs ────────────────────────────────────────────────────────────────

/// Bloom post-processing settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BloomSettings {
    /// Master enable/disable switch.
    pub enabled:     bool,
    /// Luminance threshold: pixels below this value are not bloomed.
    pub threshold:   f32,
    /// Soft-knee width for the threshold knee curve.
    pub soft_knee:   f32,
    /// Additive blend strength of the bloom onto the HDR buffer.
    pub strength:    f32,
    /// Number of Kawase blur iterations (max 8).
    pub blur_passes: u32,
}

impl Default for BloomSettings {
    fn default() -> Self {
        Self {
            enabled:     true,
            threshold:   1.2,
            soft_knee:   0.5,
            strength:    0.25,
            blur_passes: 4,
        }
    }
}

/// Tonemapping and film-look settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TonemapSettings {
    /// Exposure multiplier applied before ACES tonemap (1.0 = no change).
    pub exposure:              f32,
    /// Vignette darkness at screen edges (0 = off, 1 = full black border).
    pub vignette_intensity:    f32,
    /// Vignette shape: 1.0 = circular, values < 1.0 = more rectangular.
    pub vignette_roundness:    f32,
    /// Lateral chromatic aberration radius (0 = off).
    pub chromatic_aberration:  f32,
    /// Animated film grain strength (0 = off).
    pub film_grain:            f32,
}

impl Default for TonemapSettings {
    fn default() -> Self {
        Self {
            exposure:             0.7,
            vignette_intensity:   0.0,
            vignette_roundness:   0.8,
            chromatic_aberration: 0.3,
            film_grain:           0.01,
        }
    }
}

/// Screen-Space Ambient Occlusion settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SsaoSettings {
    /// Master enable/disable switch.
    pub enabled:   bool,
    /// Hemisphere sample radius in world-space units.
    pub radius:    f32,
    /// Depth bias to prevent self-occlusion artefacts.
    pub bias:      f32,
    /// AO intensity multiplier.
    pub intensity: f32,
}

impl Default for SsaoSettings {
    fn default() -> Self {
        Self {
            enabled:   true,
            radius:    0.5,
            bias:      0.025,
            intensity: 1.0,
        }
    }
}

/// Per-pass enable flags.
///
/// The core passes (Geometry, Lighting, Tonemap) are always enabled.
/// Use these flags to toggle optional passes without removing them from
/// the render graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PassSettings {
    pub skybox:    bool,
    pub ssao:      bool,
    pub bloom:     bool,
    pub particles: bool,
}

impl Default for PassSettings {
    fn default() -> Self {
        Self {
            skybox:    true,
            ssao:      true,
            bloom:     true,
            particles: true,
        }
    }
}

// ── Root config ────────────────────────────────────────────────────────────────

/// Top-level renderer configuration.
///
/// Serialized to / deserialized from `renderer.config.json`.
/// All fields have sensible defaults so the file is optional.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RendererConfig {
    /// Runtime upper limit on active lights. Must be ≤ 64 (the GPU buffer size).
    /// Lower values improve light-loop performance on simple scenes.
    pub max_lights: usize,

    /// Bloom post-process settings.
    pub bloom:   BloomSettings,

    /// Tonemapping and film-look settings.
    pub tonemap: TonemapSettings,

    /// Screen-space ambient occlusion settings.
    pub ssao:    SsaoSettings,

    /// Per-pass enable flags.
    pub passes:  PassSettings,
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self {
            max_lights: 64,
            bloom:      BloomSettings::default(),
            tonemap:    TonemapSettings::default(),
            ssao:       SsaoSettings::default(),
            passes:     PassSettings::default(),
        }
    }
}

// ── File I/O ───────────────────────────────────────────────────────────────────

/// Load a `RendererConfig` from a JSON file.
///
/// Returns `Default::default()` if the file does not exist.
/// Returns an error if the file exists but cannot be parsed.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_renderer_config(path: &str) -> Result<RendererConfig, String> {
    use std::path::Path;
    if !Path::new(path).exists() {
        return Ok(RendererConfig::default());
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read '{path}': {e}"))?;
    serde_json::from_str(&text)
        .map_err(|e| format!("Failed to parse '{path}': {e}"))
}

/// Serialize a `RendererConfig` to a pretty-printed JSON file.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_renderer_config(path: &str, config: &RendererConfig) -> Result<(), String> {
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Serialize error: {e}"))?;
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, &json)
        .map_err(|e| format!("Write failed '{tmp}': {e}"))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("Rename failed: {e}"))?;
    Ok(())
}
