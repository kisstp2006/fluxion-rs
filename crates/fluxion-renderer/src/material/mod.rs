// ============================================================
// fluxion-renderer — Material system
// ============================================================

pub mod material_asset;
pub mod pbr;

pub use material_asset::{MaterialAsset, ShaderParamValue, AlphaMode};
pub use pbr::{PbrMaterial, PbrParams, MaterialRegistry};
