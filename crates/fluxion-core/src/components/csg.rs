// ============================================================
// fluxion-core — CsgShape component
//
// Constructive Solid Geometry descriptor.
// The component marks an entity as a CSG primitive.
// A CsgSystem collects these and builds the combined mesh.
//
// Unity analogy: ProBuilder BSP / RealtimeCSG plugin.
//
// Supported operations: Union | Subtract | Intersect
// Primitive shapes: Cube | Sphere | Cylinder | Capsule
//
// The system merges the primitives on the parent entity into a
// single MeshRenderer mesh (stored in `merged_mesh_handle`).
// Re-bake is triggered whenever `dirty` is set to true.
// ============================================================

use std::sync::OnceLock;
use serde::{Deserialize, Serialize};
use crate::ecs::Component;
use crate::reflect::{Reflect, ReflectValue, FieldDescriptor, ReflectFieldType, RangeHint};

// ── Enums ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CsgOperation {
    Union,
    Subtract,
    Intersect,
}

impl CsgOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Union     => "Union",
            Self::Subtract  => "Subtract",
            Self::Intersect => "Intersect",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "Subtract"  => Self::Subtract,
            "Intersect" => Self::Intersect,
            _           => Self::Union,
        }
    }
    pub fn variants() -> &'static [&'static str] {
        &["Union", "Subtract", "Intersect"]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CsgPrimitive {
    Cube,
    Sphere,
    Cylinder,
    Capsule,
}

impl CsgPrimitive {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cube     => "Cube",
            Self::Sphere   => "Sphere",
            Self::Cylinder => "Cylinder",
            Self::Capsule  => "Capsule",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "Sphere"   => Self::Sphere,
            "Cylinder" => Self::Cylinder,
            "Capsule"  => Self::Capsule,
            _          => Self::Cube,
        }
    }
    pub fn variants() -> &'static [&'static str] {
        &["Cube", "Sphere", "Cylinder", "Capsule"]
    }
}

// ── Component ─────────────────────────────────────────────────────────────────

/// Marks an entity as a CSG primitive.
///
/// The [`CsgSystem`] queries all entities with this component each frame
/// (only when `dirty == true`) and bakes the combined mesh into the parent
/// entity's [`MeshRenderer`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsgShape {
    /// Boolean operation with respect to the parent / previous primitive.
    pub operation: CsgOperation,
    /// Primitive shape type.
    pub shape:     CsgPrimitive,
    /// Uniform scale applied to the primitive before the CSG operation.
    pub size:      [f32; 3],
    /// Marks the combined mesh as needing a re-bake.
    /// Set to `true` whenever the user changes any CSG property.
    pub dirty:     bool,
    /// Cached mesh handle after the last bake (not serialized).
    #[serde(skip)]
    pub merged_mesh_handle: Option<u32>,
}

impl Default for CsgShape {
    fn default() -> Self {
        Self {
            operation: CsgOperation::Union,
            shape:     CsgPrimitive::Cube,
            size:      [1.0, 1.0, 1.0],
            dirty:     true,
            merged_mesh_handle: None,
        }
    }
}

impl Component for CsgShape {}

// ── Reflection ────────────────────────────────────────────────────────────────

static CSG_FIELDS: OnceLock<Vec<FieldDescriptor>> = OnceLock::new();

fn csg_fields() -> &'static [FieldDescriptor] {
    CSG_FIELDS.get_or_init(|| vec![
        FieldDescriptor::new("operation", "Operation", ReflectFieldType::Enum)
            .with_variants(CsgOperation::variants()),
        FieldDescriptor::new("shape", "Shape", ReflectFieldType::Enum)
            .with_variants(CsgPrimitive::variants()),
        FieldDescriptor::new("size", "Size", ReflectFieldType::Vec3)
            .with_range(RangeHint::min_max(0.01, 100.0)),
    ])
}

impl Reflect for CsgShape {
    fn reflect_type_name(&self) -> &'static str { "CsgShape" }
    fn fields(&self) -> &'static [FieldDescriptor] { csg_fields() }

    fn get_field(&self, name: &str) -> Option<ReflectValue> {
        match name {
            "operation" => Some(ReflectValue::Enum(self.operation.as_str().to_string())),
            "shape"     => Some(ReflectValue::Enum(self.shape.as_str().to_string())),
            "size"      => Some(ReflectValue::Vec3(self.size)),
            _           => None,
        }
    }

    fn set_field(&mut self, name: &str, value: ReflectValue) -> Result<(), String> {
        match (name, value) {
            ("operation", ReflectValue::Enum(s))  => { self.operation = CsgOperation::from_str(&s); self.dirty = true; }
            ("shape",     ReflectValue::Enum(s))  => { self.shape = CsgPrimitive::from_str(&s); self.dirty = true; }
            ("size",      ReflectValue::Vec3(v))  => { self.size = v; self.dirty = true; }
            (f, v) => return Err(format!("CsgShape: unknown field '{}' or type mismatch ({:?})", f, v)),
        }
        Ok(())
    }
}

// ── System ────────────────────────────────────────────────────────────────────

/// Stub system that collects dirty CSG entities and schedules a re-bake.
///
/// Currently the actual mesh boolean operations are NOT implemented (that
/// requires a full BSP / Sutherland-Hodgman pipeline).  The system marks
/// the entity as clean so the editor does not loop, and logs a one-time
/// notice.  Replace the body of `bake_entity` with a real CSG library
/// (e.g. `csgrs` crate) when available.
pub struct CsgSystem;

impl CsgSystem {
    /// Call once per frame (editor or runtime).
    pub fn update(world: &mut crate::ecs::ECSWorld) {
        let dirty_ids: Vec<crate::ecs::EntityId> = world
            .all_entities()
            .filter(|&id| {
                world.get_component::<CsgShape>(id)
                    .map(|c| c.dirty)
                    .unwrap_or(false)
            })
            .collect();

        for id in dirty_ids {
            Self::bake_entity(world, id);
        }
    }

    fn bake_entity(world: &mut crate::ecs::ECSWorld, id: crate::ecs::EntityId) {
        if let Some(mut csg) = world.get_component_mut::<CsgShape>(id) {
            csg.dirty = false;
            // TODO: run BSP merge here using csgrs or similar.
            // For now this is a no-op stub — values are stored and exposed
            // in the inspector but no mesh is generated yet.
        }
    }
}
