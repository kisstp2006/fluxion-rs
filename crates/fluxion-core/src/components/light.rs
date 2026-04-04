// ============================================================
// Light component
//
// Represents a light source in the scene. The renderer collects
// all active Light components each frame and packs them into a
// GPU light buffer (up to MAX_LIGHTS).
//
// Supported light types match the TypeScript engine's LightComponent.
// ============================================================

use serde::{Deserialize, Serialize};
use fluxion_reflect_derive::Reflect;

use crate::ecs::component::Component;

/// The type of light. Determines which parameters are relevant.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LightType {
    /// Infinitely distant light (like the sun). Position is ignored,
    /// only the entity's rotation (forward direction) matters.
    /// Cheap — one shadow map covers the whole scene (via CSM).
    Directional,

    /// Omnidirectional point light. Illuminates in all directions from
    /// the entity's world position. Attenuated by `range`.
    Point,

    /// Cone-shaped spotlight. Uses both position and forward direction.
    /// Cone angle controlled by `spot_angle` and `spot_penumbra`.
    Spot,
}

/// Light component. Attach to any entity with a Transform.
///
/// # Example — sun light
/// ```rust
/// let mut t = Transform::new();
/// t.rotation = Quat::from_rotation_x(-45_f32.to_radians());
/// world.add_component(sun, t);
/// world.add_component(sun, Light {
///     light_type: LightType::Directional,
///     color: [1.0, 0.98, 0.9],
///     intensity: 2.0,
///     cast_shadow: true,
///     ..Light::default()
/// });
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Reflect)]
pub struct Light {
    pub light_type: LightType,

    /// Linear RGB color. Values > 1.0 are valid for HDR rendering.
    pub color: [f32; 3],

    /// Intensity multiplier. Units depend on light type:
    ///   - Directional: lux (lm/m²), typical sunlight ≈ 100 000 lux
    ///   - Point/Spot:  luminous power (lm), typical bulb ≈ 800 lm
    /// For game purposes, just tune until it looks right. Default: 1.0.
    #[reflect(range(min = 0.0, max = 200.0))]
    pub intensity: f32,

    /// Maximum influence range in meters. Point/Spot only.
    /// Objects beyond this distance receive no light.
    #[reflect(range(min = 0.0, max = 1000.0))]
    pub range: f32,

    /// Outer half-angle of the spotlight cone in degrees. Spot only.
    /// Default: 30 degrees.
    #[reflect(range(min = 1.0, max = 89.0))]
    pub spot_angle: f32,

    /// Fraction of the cone that fades from full to zero brightness.
    /// 0.0 = hard edge, 1.0 = soft fully blended edge. Default: 0.15.
    #[reflect(range(min = 0.0, max = 1.0))]
    pub spot_penumbra: f32,

    /// Whether this light casts real-time shadows. Shadow casting is
    /// expensive — use sparingly, or only for the primary directional light.
    pub cast_shadow: bool,

    /// Shadow map resolution (width = height). Must be a power of 2.
    /// Higher values = sharper shadows at the cost of VRAM. Default: 1024.
    pub shadow_map_size: u32,

    /// Small depth bias to prevent "shadow acne" (self-shadowing artifacts).
    /// Increase if shadows show stripes on surfaces. Default: 0.005.
    #[reflect(range(min = 0.0, max = 0.1))]
    pub shadow_bias: f32,
}

impl Default for Light {
    fn default() -> Self {
        Light {
            light_type:     LightType::Directional,
            color:          [1.0, 1.0, 1.0],
            intensity:      1.0,
            range:          10.0,
            spot_angle:     30.0,
            spot_penumbra:  0.15,
            cast_shadow:    false,
            shadow_map_size: 1024,
            shadow_bias:    0.005,
        }
    }
}

impl Light {
    /// Create a directional sun-like light.
    pub fn directional(color: [f32; 3], intensity: f32) -> Self {
        Light { light_type: LightType::Directional, color, intensity, ..Self::default() }
    }

    /// Create a point light.
    pub fn point(color: [f32; 3], intensity: f32, range: f32) -> Self {
        Light { light_type: LightType::Point, color, intensity, range, ..Self::default() }
    }
}

impl Component for Light {}
