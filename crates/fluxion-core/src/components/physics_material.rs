// ============================================================
// fluxion-core — Physics Material asset type
//
// Stored as `.physmat` JSON files in the asset directory.
// Assigned to RigidBody via `physics_material_path`.
// When loaded, overrides the body's friction and restitution
// with combine-mode semantics matching Rapier and Unity.
// ============================================================

use serde::{Deserialize, Serialize};

/// How two materials' friction or restitution values are combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CombineMode {
    /// Use the average of the two values.
    #[default]
    Average,
    /// Use the minimum of the two values.
    Minimum,
    /// Use the maximum of the two values.
    Maximum,
    /// Use the product of the two values.
    Multiply,
}

impl CombineMode {
    pub fn combine(self, a: f32, b: f32) -> f32 {
        match self {
            CombineMode::Average  => (a + b) * 0.5,
            CombineMode::Minimum  => a.min(b),
            CombineMode::Maximum  => a.max(b),
            CombineMode::Multiply => a * b,
        }
    }
}

/// Physics material asset — serialised as `.physmat` JSON.
///
/// # Example `.physmat` file
/// ```json
/// {
///   "friction": 0.4,
///   "restitution": 0.0,
///   "frictionCombine": "minimum",
///   "restitutionCombine": "maximum"
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhysicsMaterial {
    /// Static + dynamic friction coefficient. Range: [0, ∞).
    #[serde(default = "default_friction")]
    pub friction: f32,
    /// Bounciness coefficient. Range: [0, 1].
    #[serde(default = "default_restitution")]
    pub restitution: f32,
    /// How friction values are combined with the other body.
    #[serde(default)]
    pub friction_combine: CombineMode,
    /// How restitution values are combined with the other body.
    #[serde(default)]
    pub restitution_combine: CombineMode,
}

fn default_friction()    -> f32 { 0.5 }
fn default_restitution() -> f32 { 0.0 }

impl Default for PhysicsMaterial {
    fn default() -> Self {
        Self {
            friction:            0.5,
            restitution:         0.0,
            friction_combine:    CombineMode::Average,
            restitution_combine: CombineMode::Average,
        }
    }
}

impl PhysicsMaterial {
    /// Load a PhysicsMaterial from a `.physmat` JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Serialize to JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}
