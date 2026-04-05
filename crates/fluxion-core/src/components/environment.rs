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
    /// ACES fitted (Stephen Hill / Baking Lab RRT+ODT, industry standard).
    Aces,
    /// AgX perceptual tonemapping (Blender-style, good for neutrals).
    AgX,
    /// Uchimura / Gran Turismo 7 style — punchy but natural highlights.
    Uchimura,
}

impl ToneMapMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None     => "None",
            Self::Linear   => "Linear",
            Self::Reinhard => "Reinhard",
            Self::Aces     => "Aces",
            Self::AgX      => "AgX",
            Self::Uchimura => "Uchimura",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "None"     => Self::None,
            "Linear"   => Self::Linear,
            "Reinhard" => Self::Reinhard,
            "AgX"      => Self::AgX,
            "Uchimura" => Self::Uchimura,
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
            Self::Uchimura => 5,
        }
    }
}

impl Default for ToneMapMode {
    fn default() -> Self { Self::Aces }
}

// ── Background / sky mode ───────────────────────────────────────────────────

/// How the scene background is rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackgroundMode {
    /// Flat solid color — no sky.
    SolidColor,
    /// Gradient sky with configurable horizon / zenith colors + sun disc.
    Gradient,
    /// Preetham analytical atmospheric sky (turbidity, Rayleigh, Mie) + sun.
    ProceduralSky,
    /// Equirectangular panorama texture (.png / .hdr) used as sky background.
    Panorama,
}

impl BackgroundMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SolidColor    => "SolidColor",
            Self::Gradient      => "Gradient",
            Self::ProceduralSky => "ProceduralSky",
            Self::Panorama      => "Panorama",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "SolidColor"    => Self::SolidColor,
            "Gradient"      => Self::Gradient,
            "ProceduralSky" => Self::ProceduralSky,
            "Panorama"      => Self::Panorama,
            _               => Self::Gradient,
        }
    }
}

impl Default for BackgroundMode {
    fn default() -> Self { Self::Gradient }
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

// ── Sky settings ────────────────────────────────────────────────────────────

/// Controls the scene background / sky.
///
/// Modes:
/// - `SolidColor`    — flat RGB clear colour
/// - `Gradient`      — two-color horizon→zenith gradient with a simple sun disc
/// - `ProceduralSky` — Preetham analytical atmosphere (turbidity, Rayleigh, Mie)
/// - `Panorama`      — equirectangular HDR/PNG texture
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkySettings {
    pub mode: BackgroundMode,

    // Solid color background
    pub solid_color: [f32; 3],

    // Gradient sky (also used as fallback for procedural)
    pub horizon_color: [f32; 3],
    pub zenith_color:  [f32; 3],
    pub sun_intensity: f32,
    pub sun_size:      f32,      // angular radius in radians

    // Procedural sky (Preetham model) — also drives sun direction in gradient mode
    pub sun_elevation:      f32,  // degrees above horizon  (0–90)
    pub sun_azimuth:        f32,  // degrees, 0=north, 180=south
    pub turbidity:          f32,  // atmosphere haziness (2–20)
    pub rayleigh:           f32,  // Rayleigh scattering (0–5)
    pub mie_coefficient:    f32,  // Mie scattering (0–0.1)
    pub mie_directional_g:  f32,  // Mie asymmetry (0–1)

    // Panorama texture — project-relative path to .png / .hdr
    pub skybox_path: String,
}

impl Default for SkySettings {
    fn default() -> Self {
        Self {
            mode:              BackgroundMode::Gradient,
            solid_color:       [0.05, 0.07, 0.10],
            horizon_color:     [0.60, 0.75, 1.00],
            zenith_color:      [0.10, 0.30, 0.80],
            sun_intensity:     20.0,
            sun_size:          0.02,
            sun_elevation:     45.0,
            sun_azimuth:       180.0,
            turbidity:         2.0,
            rayleigh:          1.0,
            mie_coefficient:   0.005,
            mie_directional_g: 0.8,
            skybox_path:       String::new(),
        }
    }
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
    pub sky:      SkySettings,
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
            sky:      SkySettings::default(),
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

/// Compute sun direction vector from elevation (deg) and azimuth (deg).
pub fn sun_direction_from_angles(elevation_deg: f32, azimuth_deg: f32) -> [f32; 3] {
    let el = elevation_deg.to_radians();
    let az = azimuth_deg.to_radians();
    let x = el.cos() * az.sin();
    let y = el.sin();
    let z = el.cos() * az.cos();
    [x, y, z]
}

// ── Reflect ───────────────────────────────────────────────────────────────────

static ENV_FIELDS: OnceLock<Vec<FieldDescriptor>> = OnceLock::new();

fn env_fields() -> &'static [FieldDescriptor] {
    ENV_FIELDS.get_or_init(|| {
        let r = |min: f32, max: f32, step: f32| RangeHint {
            min: Some(min), max: Some(max), step: Some(step),
        };
        vec![
            // Sky
            FieldDescriptor::new("sky_mode",          "Sky / Mode",             ReflectFieldType::Enum)
                .with_variants(&["Gradient", "ProceduralSky", "SolidColor", "Panorama"]),
            FieldDescriptor::new("sky_solid_color",    "Sky / Solid Color",       ReflectFieldType::Color3),
            FieldDescriptor::new("sky_horizon_color",  "Sky / Horizon Color",     ReflectFieldType::Color3),
            FieldDescriptor::new("sky_zenith_color",   "Sky / Zenith Color",      ReflectFieldType::Color3),
            FieldDescriptor::new("sky_sun_intensity",  "Sky / Sun Intensity",     ReflectFieldType::F32)
                .with_range(r(0.0, 100.0, 0.5)),
            FieldDescriptor::new("sky_sun_size",       "Sky / Sun Size",          ReflectFieldType::F32)
                .with_range(r(0.001, 0.2, 0.001)),
            FieldDescriptor::new("sky_sun_elevation",  "Sky / Sun Elevation",     ReflectFieldType::F32)
                .with_range(r(-10.0, 90.0, 0.5)),
            FieldDescriptor::new("sky_sun_azimuth",    "Sky / Sun Azimuth",       ReflectFieldType::F32)
                .with_range(r(0.0, 360.0, 1.0)),
            FieldDescriptor::new("sky_turbidity",      "Sky / Turbidity",         ReflectFieldType::F32)
                .with_range(r(1.0, 20.0, 0.1)),
            FieldDescriptor::new("sky_rayleigh",       "Sky / Rayleigh",          ReflectFieldType::F32)
                .with_range(r(0.0, 5.0, 0.05)),
            FieldDescriptor::new("sky_mie_coeff",      "Sky / Mie Coefficient",   ReflectFieldType::F32)
                .with_range(r(0.0, 0.1, 0.001)),
            FieldDescriptor::new("sky_mie_dir_g",      "Sky / Mie Directional G", ReflectFieldType::F32)
                .with_range(r(0.0, 1.0, 0.01)),
            FieldDescriptor::new("sky_panorama_path",  "Sky / Panorama Texture",  ReflectFieldType::Texture),
            // Ambient
            FieldDescriptor::new("ambient_color",     "Ambient / Color",     ReflectFieldType::Color3),
            FieldDescriptor::new("ambient_intensity", "Ambient / Intensity", ReflectFieldType::F32)
                .with_range(r(0.0, 5.0, 0.05)),
            // Fog
            FieldDescriptor::new("fog_enabled", "Fog / Enabled", ReflectFieldType::Bool),
            FieldDescriptor::new("fog_color",   "Fog / Color",   ReflectFieldType::Color3),
            FieldDescriptor::new("fog_mode",    "Fog / Mode",    ReflectFieldType::Enum)
                .with_variants(&["Linear", "Exponential", "ExponentialSquared"]),
            FieldDescriptor::new("fog_density", "Fog / Density", ReflectFieldType::F32)
                .with_range(r(0.0, 0.1, 0.001)),
            FieldDescriptor::new("fog_near", "Fog / Near", ReflectFieldType::F32)
                .with_range(r(0.0, 1000.0, 1.0)),
            FieldDescriptor::new("fog_far",  "Fog / Far",  ReflectFieldType::F32)
                .with_range(r(0.0, 5000.0, 10.0)),
            // Tonemap
            FieldDescriptor::new("tone_mode", "Tonemap / Mode",     ReflectFieldType::Enum)
                .with_variants(&["None", "Linear", "Reinhard", "Aces", "AgX", "Uchimura"]),
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
            "sky_mode"          => Some(ReflectValue::Enum(self.sky.mode.as_str().to_string())),
            "sky_solid_color"   => Some(ReflectValue::Color3(self.sky.solid_color)),
            "sky_horizon_color" => Some(ReflectValue::Color3(self.sky.horizon_color)),
            "sky_zenith_color"  => Some(ReflectValue::Color3(self.sky.zenith_color)),
            "sky_sun_intensity" => Some(ReflectValue::F32(self.sky.sun_intensity)),
            "sky_sun_size"      => Some(ReflectValue::F32(self.sky.sun_size)),
            "sky_sun_elevation" => Some(ReflectValue::F32(self.sky.sun_elevation)),
            "sky_sun_azimuth"   => Some(ReflectValue::F32(self.sky.sun_azimuth)),
            "sky_turbidity"     => Some(ReflectValue::F32(self.sky.turbidity)),
            "sky_rayleigh"      => Some(ReflectValue::F32(self.sky.rayleigh)),
            "sky_mie_coeff"     => Some(ReflectValue::F32(self.sky.mie_coefficient)),
            "sky_mie_dir_g"     => Some(ReflectValue::F32(self.sky.mie_directional_g)),
            "sky_panorama_path" => Some(ReflectValue::AssetPath(if self.sky.skybox_path.is_empty() { None } else { Some(self.sky.skybox_path.clone()) })),
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
            ("sky_mode",          ReflectValue::Enum(s))       => self.sky.mode = BackgroundMode::from_str(&s),
            ("sky_solid_color",   ReflectValue::Color3(c))     => self.sky.solid_color = c,
            ("sky_horizon_color", ReflectValue::Color3(c))     => self.sky.horizon_color = c,
            ("sky_zenith_color",  ReflectValue::Color3(c))     => self.sky.zenith_color = c,
            ("sky_sun_intensity", ReflectValue::F32(f))        => self.sky.sun_intensity = f,
            ("sky_sun_size",      ReflectValue::F32(f))        => self.sky.sun_size = f,
            ("sky_sun_elevation", ReflectValue::F32(f))        => self.sky.sun_elevation = f,
            ("sky_sun_azimuth",   ReflectValue::F32(f))        => self.sky.sun_azimuth = f,
            ("sky_turbidity",     ReflectValue::F32(f))        => self.sky.turbidity = f,
            ("sky_rayleigh",      ReflectValue::F32(f))        => self.sky.rayleigh = f,
            ("sky_mie_coeff",     ReflectValue::F32(f))        => self.sky.mie_coefficient = f,
            ("sky_mie_dir_g",     ReflectValue::F32(f))        => self.sky.mie_directional_g = f,
            ("sky_panorama_path", ReflectValue::AssetPath(p))  => self.sky.skybox_path = p.unwrap_or_default(),
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
