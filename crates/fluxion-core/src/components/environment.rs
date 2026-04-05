// ============================================================
// fluxion-core — Environment component
//
// Scene-wide post-processing and environment settings.
// Only one Environment component should exist per scene;
// the renderer uses the first one found to override the
// global RendererConfig for that frame.
//
// Unity analogy:
//   - AmbientSettings    → RenderSettings.ambientLight / ambientIntensity
//   - FogSettings        → RenderSettings.fog*
//   - ToneMapSettings    → PostProcessVolume (Tonemapping override)
//   - BloomSettings      → PostProcessVolume (Bloom override)
//   - SsaoSettings       → PostProcessVolume (Ambient Occlusion override)
//   - DofSettings        → PostProcessVolume (Depth of Field override) [pass not yet implemented]
//   - VignetteSettings   → PostProcessVolume (Vignette override)
//   - FilmSettings       → PostProcessVolume (Film Grain / Lens Distortion)
// ============================================================

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use crate::ecs::component::Component;
use crate::reflect::{FieldDescriptor, ReflectFieldType, ReflectValue, RangeHint, Reflect};

// ── Tone-mapping mode ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToneMapMode {
    /// No tonemapping — output raw HDR values (clamped to [0,1]).
    None,
    /// Simple linear scale by exposure.
    Linear,
    /// Reinhard global tonemapping.
    Reinhard,
    /// ACES Narkowicz filmic approximation (default).
    Aces,
    /// AgX perceptual tonemapping (Blender-style, good for neutrals).
    AgX,
}

impl ToneMapMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None     => "None",
            Self::Linear   => "Linear",
            Self::Reinhard => "Reinhard",
            Self::Aces     => "Aces",
            Self::AgX      => "AgX",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "None"     => Self::None,
            "Linear"   => Self::Linear,
            "Reinhard" => Self::Reinhard,
            "AgX"      => Self::AgX,
            _          => Self::Aces,
        }
    }

    pub fn as_u32(self) -> u32 {
        match self {
            Self::None     => 0,
            Self::Linear   => 1,
            Self::Reinhard => 2,
            Self::Aces     => 3,
            Self::AgX      => 4,
        }
    }
}

impl Default for ToneMapMode {
    fn default() -> Self { Self::Aces }
}

// ── Fog mode ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FogMode {
    /// Exponential fog: density = exp(-d * fog_density).
    Exponential,
    /// Linear fog: fades from fog_near to fog_far.
    Linear,
}

impl FogMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exponential => "Exponential",
            Self::Linear      => "Linear",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "Linear" => Self::Linear,
            _        => Self::Exponential,
        }
    }
}

impl Default for FogMode {
    fn default() -> Self { Self::Exponential }
}

// ── Sub-setting structs ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmbientSettings {
    pub color:     [f32; 3],
    pub intensity: f32,
}

impl Default for AmbientSettings {
    fn default() -> Self {
        Self { color: [0.27, 0.27, 0.35], intensity: 0.5 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FogSettings {
    pub enabled: bool,
    pub color:   [f32; 3],
    pub mode:    FogMode,
    /// Exponential density (ignored in Linear mode).
    pub density: f32,
    /// Linear fog start distance (ignored in Exponential mode).
    pub near:    f32,
    /// Linear fog end distance (ignored in Exponential mode).
    pub far:     f32,
}

impl Default for FogSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            color:   [0.1, 0.1, 0.15],
            mode:    FogMode::Exponential,
            density: 0.008,
            near:    10.0,
            far:     100.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToneMapSettings {
    pub mode:     ToneMapMode,
    pub exposure: f32,
}

impl Default for ToneMapSettings {
    fn default() -> Self {
        Self { mode: ToneMapMode::Aces, exposure: 1.2 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BloomSettings {
    pub enabled:     bool,
    pub threshold:   f32,
    pub soft_knee:   f32,
    pub strength:    f32,
    pub blur_passes: u32,
}

impl Default for BloomSettings {
    fn default() -> Self {
        Self {
            enabled:     true,
            threshold:   0.8,
            soft_knee:   0.5,
            strength:    0.5,
            blur_passes: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsaoSettings {
    pub enabled:   bool,
    pub radius:    f32,
    pub bias:      f32,
    pub intensity: f32,
}

impl Default for SsaoSettings {
    fn default() -> Self {
        Self { enabled: false, radius: 0.5, bias: 0.025, intensity: 1.0 }
    }
}

/// Depth of Field settings (stored but pass not yet implemented).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DofSettings {
    pub enabled:       bool,
    pub focus_dist:    f32,
    pub aperture:      f32,
    pub max_blur:      f32,
}

impl Default for DofSettings {
    fn default() -> Self {
        Self { enabled: false, focus_dist: 10.0, aperture: 0.025, max_blur: 10.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VignetteSettings {
    pub enabled:   bool,
    pub intensity: f32,
    pub roundness: f32,
}

impl Default for VignetteSettings {
    fn default() -> Self {
        Self { enabled: true, intensity: 0.3, roundness: 0.8 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilmSettings {
    pub chromatic_aberration: f32,
    pub film_grain:           f32,
}

impl Default for FilmSettings {
    fn default() -> Self {
        Self { chromatic_aberration: 0.0, film_grain: 0.0 }
    }
}

// ── Environment component ────────────────────────────────────────────────────

/// Scene-wide post-processing and environment settings.
///
/// Add one per scene. The renderer picks the first active Environment
/// component and uses it to override the global RendererConfig.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Environment {
    pub ambient:  AmbientSettings,
    pub fog:      FogSettings,
    pub tonemap:  ToneMapSettings,
    pub bloom:    BloomSettings,
    pub ssao:     SsaoSettings,
    pub dof:      DofSettings,
    pub vignette: VignetteSettings,
    pub film:     FilmSettings,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            ambient:  AmbientSettings::default(),
            fog:      FogSettings::default(),
            tonemap:  ToneMapSettings::default(),
            bloom:    BloomSettings::default(),
            ssao:     SsaoSettings::default(),
            dof:      DofSettings::default(),
            vignette: VignetteSettings::default(),
            film:     FilmSettings::default(),
        }
    }
}

impl Component for Environment {}

// ── Reflect ───────────────────────────────────────────────────────────────────

static ENV_FIELDS: OnceLock<Vec<FieldDescriptor>> = OnceLock::new();

fn env_fields() -> &'static [FieldDescriptor] {
    ENV_FIELDS.get_or_init(|| {
        let r = |min: f32, max: f32, step: f32| RangeHint {
            min: Some(min), max: Some(max), step: Some(step),
        };
        vec![
            // Ambient
            FieldDescriptor::new("ambient_color",     "Ambient / Color",     ReflectFieldType::Color3),
            FieldDescriptor::new("ambient_intensity", "Ambient / Intensity", ReflectFieldType::F32)
                .with_range(r(0.0, 5.0, 0.05)),
            // Fog
            FieldDescriptor::new("fog_enabled", "Fog / Enabled", ReflectFieldType::Bool),
            FieldDescriptor::new("fog_color",   "Fog / Color",   ReflectFieldType::Color3),
            FieldDescriptor::new("fog_mode",    "Fog / Mode",    ReflectFieldType::Enum),
            FieldDescriptor::new("fog_density", "Fog / Density", ReflectFieldType::F32)
                .with_range(r(0.0, 0.1, 0.001)),
            FieldDescriptor::new("fog_near", "Fog / Near", ReflectFieldType::F32)
                .with_range(r(0.0, 1000.0, 1.0)),
            FieldDescriptor::new("fog_far",  "Fog / Far",  ReflectFieldType::F32)
                .with_range(r(0.0, 5000.0, 10.0)),
            // Tonemap
            FieldDescriptor::new("tone_mode", "Tonemap / Mode",     ReflectFieldType::Enum),
            FieldDescriptor::new("exposure",  "Tonemap / Exposure", ReflectFieldType::F32)
                .with_range(r(0.0, 5.0, 0.05)),
            // Bloom
            FieldDescriptor::new("bloom_enabled",     "Bloom / Enabled",     ReflectFieldType::Bool),
            FieldDescriptor::new("bloom_threshold",   "Bloom / Threshold",   ReflectFieldType::F32)
                .with_range(r(0.0, 3.0, 0.05)),
            FieldDescriptor::new("bloom_soft_knee",   "Bloom / Soft Knee",   ReflectFieldType::F32)
                .with_range(r(0.0, 1.0, 0.05)),
            FieldDescriptor::new("bloom_strength",    "Bloom / Strength",    ReflectFieldType::F32)
                .with_range(r(0.0, 3.0, 0.05)),
            FieldDescriptor::new("bloom_blur_passes", "Bloom / Blur Passes", ReflectFieldType::U32)
                .with_range(r(1.0, 8.0, 1.0)),
            // SSAO
            FieldDescriptor::new("ssao_enabled",   "SSAO / Enabled",   ReflectFieldType::Bool),
            FieldDescriptor::new("ssao_radius",    "SSAO / Radius",    ReflectFieldType::F32)
                .with_range(r(0.05, 5.0, 0.05)),
            FieldDescriptor::new("ssao_bias",      "SSAO / Bias",      ReflectFieldType::F32)
                .with_range(r(0.0, 0.5, 0.001)),
            FieldDescriptor::new("ssao_intensity", "SSAO / Intensity", ReflectFieldType::F32)
                .with_range(r(0.0, 5.0, 0.1)),
            // DoF (stored; pass not yet implemented)
            FieldDescriptor::new("dof_enabled",    "DoF / Enabled",        ReflectFieldType::Bool),
            FieldDescriptor::new("dof_focus_dist", "DoF / Focus Distance", ReflectFieldType::F32)
                .with_range(r(0.1, 500.0, 0.5)),
            FieldDescriptor::new("dof_aperture",   "DoF / Aperture",       ReflectFieldType::F32)
                .with_range(r(0.0, 0.5, 0.001)),
            FieldDescriptor::new("dof_max_blur",   "DoF / Max Blur",       ReflectFieldType::F32)
                .with_range(r(0.0, 30.0, 0.5)),
            // Vignette
            FieldDescriptor::new("vignette_enabled",   "Vignette / Enabled",   ReflectFieldType::Bool),
            FieldDescriptor::new("vignette_intensity", "Vignette / Intensity", ReflectFieldType::F32)
                .with_range(r(0.0, 2.0, 0.05)),
            FieldDescriptor::new("vignette_roundness", "Vignette / Roundness", ReflectFieldType::F32)
                .with_range(r(0.0, 1.0, 0.05)),
            // Film
            FieldDescriptor::new("chromatic_aberration", "Film / Chromatic Aberration", ReflectFieldType::F32)
                .with_range(r(0.0, 10.0, 0.1)),
            FieldDescriptor::new("film_grain", "Film / Grain", ReflectFieldType::F32)
                .with_range(r(0.0, 1.0, 0.01)),
        ]
    })
}

impl Reflect for Environment {
    fn reflect_type_name(&self) -> &'static str { "Environment" }

    fn fields(&self) -> &'static [FieldDescriptor] { env_fields() }

    fn get_field(&self, name: &str) -> Option<ReflectValue> {
        match name {
            "ambient_color"     => Some(ReflectValue::Color3(self.ambient.color)),
            "ambient_intensity" => Some(ReflectValue::F32(self.ambient.intensity)),
            "fog_enabled"  => Some(ReflectValue::Bool(self.fog.enabled)),
            "fog_color"    => Some(ReflectValue::Color3(self.fog.color)),
            "fog_mode"     => Some(ReflectValue::Enum(self.fog.mode.as_str().to_string())),
            "fog_density"  => Some(ReflectValue::F32(self.fog.density)),
            "fog_near"     => Some(ReflectValue::F32(self.fog.near)),
            "fog_far"      => Some(ReflectValue::F32(self.fog.far)),
            "tone_mode"    => Some(ReflectValue::Enum(self.tonemap.mode.as_str().to_string())),
            "exposure"     => Some(ReflectValue::F32(self.tonemap.exposure)),
            "bloom_enabled"     => Some(ReflectValue::Bool(self.bloom.enabled)),
            "bloom_threshold"   => Some(ReflectValue::F32(self.bloom.threshold)),
            "bloom_soft_knee"   => Some(ReflectValue::F32(self.bloom.soft_knee)),
            "bloom_strength"    => Some(ReflectValue::F32(self.bloom.strength)),
            "bloom_blur_passes" => Some(ReflectValue::U32(self.bloom.blur_passes)),
            "ssao_enabled"   => Some(ReflectValue::Bool(self.ssao.enabled)),
            "ssao_radius"    => Some(ReflectValue::F32(self.ssao.radius)),
            "ssao_bias"      => Some(ReflectValue::F32(self.ssao.bias)),
            "ssao_intensity" => Some(ReflectValue::F32(self.ssao.intensity)),
            "dof_enabled"    => Some(ReflectValue::Bool(self.dof.enabled)),
            "dof_focus_dist" => Some(ReflectValue::F32(self.dof.focus_dist)),
            "dof_aperture"   => Some(ReflectValue::F32(self.dof.aperture)),
            "dof_max_blur"   => Some(ReflectValue::F32(self.dof.max_blur)),
            "vignette_enabled"   => Some(ReflectValue::Bool(self.vignette.enabled)),
            "vignette_intensity" => Some(ReflectValue::F32(self.vignette.intensity)),
            "vignette_roundness" => Some(ReflectValue::F32(self.vignette.roundness)),
            "chromatic_aberration" => Some(ReflectValue::F32(self.film.chromatic_aberration)),
            "film_grain"           => Some(ReflectValue::F32(self.film.film_grain)),
            _ => None,
        }
    }

    fn set_field(&mut self, name: &str, value: ReflectValue) -> Result<(), String> {
        match (name, value) {
            ("ambient_color",     ReflectValue::Color3(c)) => self.ambient.color = c,
            ("ambient_intensity", ReflectValue::F32(f))    => self.ambient.intensity = f,
            ("fog_enabled",  ReflectValue::Bool(b))        => self.fog.enabled = b,
            ("fog_color",    ReflectValue::Color3(c))      => self.fog.color = c,
            ("fog_mode",     ReflectValue::Enum(s))        => self.fog.mode = FogMode::from_str(&s),
            ("fog_density",  ReflectValue::F32(f))         => self.fog.density = f,
            ("fog_near",     ReflectValue::F32(f))         => self.fog.near = f,
            ("fog_far",      ReflectValue::F32(f))         => self.fog.far = f,
            ("tone_mode",    ReflectValue::Enum(s))        => self.tonemap.mode = ToneMapMode::from_str(&s),
            ("exposure",     ReflectValue::F32(f))         => self.tonemap.exposure = f,
            ("bloom_enabled",     ReflectValue::Bool(b))   => self.bloom.enabled = b,
            ("bloom_threshold",   ReflectValue::F32(f))    => self.bloom.threshold = f,
            ("bloom_soft_knee",   ReflectValue::F32(f))    => self.bloom.soft_knee = f,
            ("bloom_strength",    ReflectValue::F32(f))    => self.bloom.strength = f,
            ("bloom_blur_passes", ReflectValue::U32(u))    => self.bloom.blur_passes = u,
            ("ssao_enabled",   ReflectValue::Bool(b))      => self.ssao.enabled = b,
            ("ssao_radius",    ReflectValue::F32(f))       => self.ssao.radius = f,
            ("ssao_bias",      ReflectValue::F32(f))       => self.ssao.bias = f,
            ("ssao_intensity", ReflectValue::F32(f))       => self.ssao.intensity = f,
            ("dof_enabled",    ReflectValue::Bool(b))      => self.dof.enabled = b,
            ("dof_focus_dist", ReflectValue::F32(f))       => self.dof.focus_dist = f,
            ("dof_aperture",   ReflectValue::F32(f))       => self.dof.aperture = f,
            ("dof_max_blur",   ReflectValue::F32(f))       => self.dof.max_blur = f,
            ("vignette_enabled",   ReflectValue::Bool(b))  => self.vignette.enabled = b,
            ("vignette_intensity", ReflectValue::F32(f))   => self.vignette.intensity = f,
            ("vignette_roundness", ReflectValue::F32(f))   => self.vignette.roundness = f,
            ("chromatic_aberration", ReflectValue::F32(f)) => self.film.chromatic_aberration = f,
            ("film_grain",           ReflectValue::F32(f)) => self.film.film_grain = f,
            (n, _) => return Err(format!("Unknown or type-mismatched field '{}' on Environment", n)),
        }
        Ok(())
    }
}
