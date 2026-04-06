// ============================================================
// MeshRenderer component
//
// Tells the renderer to draw a mesh at this entity's transform.
// Holds asset paths (resolved at load time) and optional GPU handles
// (filled by the renderer lazily).
//
// C# / Unity equivalent: MeshRenderer + MeshFilter combined.
// ============================================================

use serde::{Deserialize, Serialize};
use serde_json::Value;
use fluxion_reflect_derive::Reflect;

use crate::ecs::component::Component;

/// Built-in geometric shape to use when no mesh file is specified.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PrimitiveType {
    Cube,
    Sphere,
    Plane,
    Cylinder,
    Capsule,
}

/// One material slot for multi-submesh meshes (e.g. glTF with multiple primitives).
///
/// Mirrors FluxionJS V3's `FluxMeshMaterialSlot` — each sub-mesh of a `.glb`
/// can be assigned an independent `.fluxmat` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialSlot {
    /// Zero-based index matching the submesh order in the mesh file.
    pub slot_index: u8,
    /// Human-readable slot name extracted from the glTF material name (e.g. "Body", "Eyes").
    pub name: String,
    /// Project-relative path to the `.fluxmat` file assigned to this slot.
    pub material_path: Option<String>,
    /// GPU material handle — filled by the renderer at load time, not serialized.
    #[serde(skip)]
    pub material_handle: Option<u32>,
}

impl MaterialSlot {
    pub fn new(slot_index: u8, name: impl Into<String>) -> Self {
        Self { slot_index, name: name.into(), material_path: None, material_handle: None }
    }
}

/// MeshRenderer component.
///
/// Attach to any entity with a `Transform` to render a 3D mesh.
///
/// # Example
/// ```rust
/// world.add_component(entity, MeshRenderer {
///     primitive: Some(PrimitiveType::Cube),
///     cast_shadow: true,
///     ..MeshRenderer::default()
/// });
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Reflect)]
pub struct MeshRenderer {
    // ── Asset references (serialized) ─────────────────────────────────────────

    /// Path to a mesh file (.glb, .gltf, .fluxmesh).
    /// If `None`, use `primitive` instead.
    #[reflect(asset_type = "mesh", header = "Mesh", tooltip = "The 3D mesh asset to render.")]
    pub mesh_path: Option<String>,

    /// Path to a material file (.fluxmat) for single-material meshes.
    /// If `None`, the default PBR material is used.
    /// When `material_slots` is non-empty this field is ignored.
    #[reflect(asset_type = "material", header = "Material", tooltip = "Override material for single-material meshes.")]
    pub material_path: Option<String>,

    /// Per-submesh material overrides for multi-primitive meshes.
    /// Each slot corresponds to one glTF primitive / submesh by index.
    /// If empty, falls back to `material_path` (single-material mode).
    #[serde(default)]
    #[reflect(skip)]
    pub material_slots: Vec<MaterialSlot>,

    /// Use a built-in primitive shape. Ignored if `mesh_path` is set.
    #[reflect(skip)]
    pub primitive: Option<PrimitiveType>,

    // ── Rendering options (serialized) ────────────────────────────────────────

    /// Whether this mesh casts shadows. Default: true.
    #[reflect(header = "Rendering")]
    pub cast_shadow: bool,

    /// Whether this mesh receives shadows from other objects. Default: true.
    pub receive_shadow: bool,

    /// Render layer bitmask. Cameras can be configured to only render
    /// specific layers (e.g., layer 0 = scene, layer 1 = UI overlays).
    /// Default: layer 0.
    pub layer: u8,

    // ── Runtime GPU handles (NOT serialized) ──────────────────────────────────
    // These are indices/keys into the renderer's registries.
    // Filled by FluxionRenderer on first render, cleared on scene unload.

    /// Handle into the renderer's MeshRegistry. None until loaded.
    #[serde(skip)]
    #[reflect(skip)]
    pub mesh_handle: Option<u32>,

    /// Handle into the renderer's MaterialRegistry. None until loaded.
    #[serde(skip)]
    #[reflect(skip)]
    pub material_handle: Option<u32>,

    /// Embedded PBR blob from FluxionJS scene files (`material` on `MeshRenderer`). Hydrated by the renderer.
    #[serde(skip)]
    #[reflect(skip)]
    pub scene_inline_material: Option<Value>,
}

impl MeshRenderer {
    pub fn from_primitive(primitive: PrimitiveType) -> Self {
        MeshRenderer { primitive: Some(primitive), ..Self::default() }
    }

    pub fn from_mesh_path(path: &str) -> Self {
        MeshRenderer { mesh_path: Some(path.to_string()), ..Self::default() }
    }
}

impl Default for MeshRenderer {
    fn default() -> Self {
        MeshRenderer {
            mesh_path:       None,
            material_path:   None,
            material_slots:  Vec::new(),
            primitive:       Some(PrimitiveType::Cube), // default: a cube
            cast_shadow:     true,
            receive_shadow:  true,
            layer:           0,
            mesh_handle:     None,
            material_handle: None,
            scene_inline_material: None,
        }
    }
}

impl Component for MeshRenderer {
    fn on_destroy(&mut self) {
        // Release GPU handle references. The renderer will GC the actual GPU
        // resources when reference count hits zero.
        self.mesh_handle     = None;
        self.material_handle = None;
        self.scene_inline_material = None;
        for slot in &mut self.material_slots {
            slot.material_handle = None;
        }
    }
}
