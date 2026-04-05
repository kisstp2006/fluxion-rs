// ============================================================
// fluxion-core — Reflect implementations for built-in components
//
// Each built-in component gets a `Reflect` impl here.
// The field list is `&'static` (via `std::sync::OnceLock`) so it
// is allocated only once and borrowed cheaply on every inspector call.
//
// Adding a new component? Copy one of the blocks below:
//   1. Declare a static FIELDS array
//   2. Impl Reflect: reflect_type_name, fields, get_field, set_field
//      (to_serialized_data is auto-derived from the default impl)
// ============================================================

use std::sync::OnceLock;

use glam::{EulerRot, Quat, Vec3};

use crate::components::camera::{Camera, ClearFlags, ProjectionMode};
use crate::components::light::{Light, LightType};
use crate::components::mesh_renderer::{MeshRenderer, PrimitiveType};
use crate::components::particle_emitter::ParticleEmitter;
use crate::components::rigid_body::{BodyType, PhysicsShape, RigidBody};
use crate::reflect::{FieldDescriptor, ReflectFieldType, ReflectValue, RangeHint, Reflect};
use crate::transform::Transform;

// ── Transform ──────────────────────────────────────────────────────────────────

static TRANSFORM_FIELDS: OnceLock<Vec<FieldDescriptor>> = OnceLock::new();

fn transform_fields() -> &'static [FieldDescriptor] {
    TRANSFORM_FIELDS.get_or_init(|| vec![
        FieldDescriptor::new("position",       "Position",       ReflectFieldType::Vec3),
        FieldDescriptor::new("rotation",       "Rotation",       ReflectFieldType::Quat),
        FieldDescriptor::new("scale",          "Scale",          ReflectFieldType::Vec3),
        FieldDescriptor::read_only("world_position", "World Position", ReflectFieldType::Vec3),
        FieldDescriptor::read_only("world_rotation", "World Rotation", ReflectFieldType::Quat),
        FieldDescriptor::read_only("world_scale",    "World Scale",    ReflectFieldType::Vec3),
    ])
}

impl Reflect for Transform {
    fn reflect_type_name(&self) -> &'static str { "Transform" }
    fn fields(&self) -> &'static [FieldDescriptor] { transform_fields() }

    fn get_field(&self, name: &str) -> Option<ReflectValue> {
        match name {
            "position"       => Some(ReflectValue::Vec3(self.position.to_array())),
            "rotation"       => Some(ReflectValue::Quat(self.rotation.to_array())),
            "scale"          => Some(ReflectValue::Vec3(self.scale.to_array())),
            "world_position" => Some(ReflectValue::Vec3(self.world_position.to_array())),
            "world_rotation" => Some(ReflectValue::Quat(self.world_rotation.to_array())),
            "world_scale"    => Some(ReflectValue::Vec3(self.world_scale.to_array())),
            _ => None,
        }
    }

    fn set_field(&mut self, name: &str, value: ReflectValue) -> Result<(), String> {
        match (name, value) {
            ("position", ReflectValue::Vec3(v)) => {
                self.position = Vec3::from(v);
                self.dirty = true;
                self.world_dirty = true;
            }
            ("rotation", ReflectValue::Quat(q)) => {
                self.rotation = Quat::from_array(q).normalize();
                self.dirty = true;
                self.world_dirty = true;
            }
            ("scale", ReflectValue::Vec3(v)) => {
                self.scale = Vec3::from(v);
                self.dirty = true;
                self.world_dirty = true;
            }
            ("world_position" | "world_rotation" | "world_scale", _) => {
                return Err(format!("Field '{}' is read-only on Transform", name));
            }
            (n, _) => return Err(format!("Unknown or type-mismatched field '{}' on Transform", n)),
        }
        Ok(())
    }

    /// Transform serializes only local position/rotation/scale (Euler XYZ degrees for readability).
    fn to_serialized_data(&self) -> serde_json::Value {
        let (ax, ay, az) = self.rotation.to_euler(EulerRot::XYZ);
        serde_json::json!({
            "position": [self.position.x, self.position.y, self.position.z],
            "rotation": [ax, ay, az],
            "scale":    [self.scale.x, self.scale.y, self.scale.z],
        })
    }
}

// ── MeshRenderer ───────────────────────────────────────────────────────────────

static MESH_RENDERER_FIELDS: OnceLock<Vec<FieldDescriptor>> = OnceLock::new();

fn mesh_renderer_fields() -> &'static [FieldDescriptor] {
    MESH_RENDERER_FIELDS.get_or_init(|| vec![
        FieldDescriptor::new("mesh_path",      "Mesh Path",       ReflectFieldType::OptionStr),
        FieldDescriptor::new("material_path",  "Material Path",   ReflectFieldType::OptionStr),
        FieldDescriptor::new("primitive",      "Primitive Type",  ReflectFieldType::Enum)
            .with_variants(&["Cube", "Sphere", "Plane", "Cylinder", "Capsule"]),
        FieldDescriptor::new("cast_shadow",    "Cast Shadow",     ReflectFieldType::Bool),
        FieldDescriptor::new("receive_shadow", "Receive Shadow",  ReflectFieldType::Bool),
        FieldDescriptor::new("layer",          "Render Layer",    ReflectFieldType::U8),
    ])
}

impl Reflect for MeshRenderer {
    fn reflect_type_name(&self) -> &'static str { "MeshRenderer" }
    fn fields(&self) -> &'static [FieldDescriptor] { mesh_renderer_fields() }

    fn get_field(&self, name: &str) -> Option<ReflectValue> {
        match name {
            "mesh_path"      => Some(ReflectValue::OptionStr(self.mesh_path.clone())),
            "material_path"  => Some(ReflectValue::OptionStr(self.material_path.clone())),
            "primitive"      => Some(ReflectValue::Enum(primitive_to_str(self.primitive))),
            "cast_shadow"    => Some(ReflectValue::Bool(self.cast_shadow)),
            "receive_shadow" => Some(ReflectValue::Bool(self.receive_shadow)),
            "layer"          => Some(ReflectValue::U8(self.layer)),
            _ => None,
        }
    }

    fn set_field(&mut self, name: &str, value: ReflectValue) -> Result<(), String> {
        match (name, value) {
            ("mesh_path",      ReflectValue::OptionStr(v)) => self.mesh_path = v,
            ("material_path",  ReflectValue::OptionStr(v)) => self.material_path = v,
            ("primitive",      ReflectValue::Enum(s))      => self.primitive = Some(str_to_primitive(&s)?),
            ("cast_shadow",    ReflectValue::Bool(b))      => self.cast_shadow = b,
            ("receive_shadow", ReflectValue::Bool(b))      => self.receive_shadow = b,
            ("layer",          ReflectValue::U8(n))        => self.layer = n,
            (n, _) => return Err(format!("Unknown or type-mismatched field '{}' on MeshRenderer", n)),
        }
        Ok(())
    }

    fn to_serialized_data(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        if let Some(ref p) = self.mesh_path {
            map.insert("modelPath".into(), serde_json::Value::String(p.clone()));
        }
        if let Some(ref p) = self.material_path {
            map.insert("materialPath".into(), serde_json::Value::String(p.clone()));
        }
        if self.mesh_path.is_none() {
            map.insert("primitiveType".into(), serde_json::Value::String(
                primitive_to_str(self.primitive).to_lowercase()
            ));
        }
        map.insert("castShadow".into(),    serde_json::Value::Bool(self.cast_shadow));
        map.insert("receiveShadow".into(), serde_json::Value::Bool(self.receive_shadow));
        map.insert("layer".into(),         serde_json::Value::from(self.layer));
        serde_json::Value::Object(map)
    }
}

fn primitive_to_str(p: Option<PrimitiveType>) -> String {
    match p {
        Some(PrimitiveType::Cube)     | None => "Cube".into(),
        Some(PrimitiveType::Sphere)          => "Sphere".into(),
        Some(PrimitiveType::Plane)           => "Plane".into(),
        Some(PrimitiveType::Cylinder)        => "Cylinder".into(),
        Some(PrimitiveType::Capsule)         => "Capsule".into(),
    }
}

fn str_to_primitive(s: &str) -> Result<PrimitiveType, String> {
    match s.to_lowercase().as_str() {
        "cube"     => Ok(PrimitiveType::Cube),
        "sphere"   => Ok(PrimitiveType::Sphere),
        "plane"    => Ok(PrimitiveType::Plane),
        "cylinder" => Ok(PrimitiveType::Cylinder),
        "capsule"  => Ok(PrimitiveType::Capsule),
        other      => Err(format!("Unknown primitive type '{}'", other)),
    }
}

// ── Camera ─────────────────────────────────────────────────────────────────────

static CAMERA_FIELDS: OnceLock<Vec<FieldDescriptor>> = OnceLock::new();

fn camera_fields() -> &'static [FieldDescriptor] {
    CAMERA_FIELDS.get_or_init(|| vec![
        FieldDescriptor::new("fov", "Field of View", ReflectFieldType::F32)
            .with_range(RangeHint::min_max(1.0, 179.0))
            .with_visible_if(|c| !matches!(c.get_field("projection"),
                Some(ReflectValue::Enum(ref s)) if s == "Orthographic")),
        FieldDescriptor::new("near", "Near Plane", ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.001, 10.0)),
        FieldDescriptor::new("far", "Far Plane", ReflectFieldType::F32)
            .with_range(RangeHint::min_max(1.0, 100_000.0)),
        FieldDescriptor::new("projection", "Projection Mode", ReflectFieldType::Enum)
            .with_variants(&["Perspective", "Orthographic"]),
        FieldDescriptor::new("ortho_size", "Ortho Size", ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.1, 1000.0))
            .with_visible_if(|c| matches!(c.get_field("projection"),
                Some(ReflectValue::Enum(ref s)) if s == "Orthographic")),
        FieldDescriptor::new("is_active",    "Is Active",    ReflectFieldType::Bool),
        FieldDescriptor::new("depth",        "Depth",        ReflectFieldType::F32),
        FieldDescriptor::new("culling_mask", "Culling Mask", ReflectFieldType::U32),
        FieldDescriptor::new("clear_flags", "Clear Flags", ReflectFieldType::Enum)
            .with_variants(&["Skybox", "SolidColor", "DepthOnly", "Nothing"]),
        FieldDescriptor::new("background_color", "Background Color", ReflectFieldType::Color4)
            .with_visible_if(|c| matches!(c.get_field("clear_flags"),
                Some(ReflectValue::Enum(ref s)) if s == "SolidColor")),
        FieldDescriptor::new("allow_hdr",    "Allow HDR",       ReflectFieldType::Bool),
        FieldDescriptor::new("allow_msaa",   "Allow MSAA",      ReflectFieldType::Bool),
        FieldDescriptor::new("use_physical", "Physical Camera", ReflectFieldType::Bool),
        FieldDescriptor::new("focal_length", "Focal Length (mm)", ReflectFieldType::F32)
            .with_range(RangeHint::min_max(1.0, 300.0))
            .with_visible_if(|c| matches!(c.get_field("use_physical"),
                Some(ReflectValue::Bool(true)))),
    ])
}

impl Reflect for Camera {
    fn reflect_type_name(&self) -> &'static str { "Camera" }
    fn fields(&self) -> &'static [FieldDescriptor] { camera_fields() }

    fn get_field(&self, name: &str) -> Option<ReflectValue> {
        match name {
            "fov"              => Some(ReflectValue::F32(self.fov)),
            "near"             => Some(ReflectValue::F32(self.near)),
            "far"              => Some(ReflectValue::F32(self.far)),
            "projection"       => Some(ReflectValue::Enum(match self.projection_mode {
                ProjectionMode::Perspective  => "Perspective".into(),
                ProjectionMode::Orthographic => "Orthographic".into(),
            })),
            "ortho_size"       => Some(ReflectValue::F32(self.ortho_size)),
            "is_active"        => Some(ReflectValue::Bool(self.is_active)),
            "depth"            => Some(ReflectValue::F32(self.depth as f32)),
            "culling_mask"     => Some(ReflectValue::U32(self.culling_mask)),
            "clear_flags"      => Some(ReflectValue::Enum(self.clear_flags.as_str().into())),
            "background_color" => Some(ReflectValue::Color4(self.background_color)),
            "allow_hdr"        => Some(ReflectValue::Bool(self.allow_hdr)),
            "allow_msaa"       => Some(ReflectValue::Bool(self.allow_msaa)),
            "use_physical"     => Some(ReflectValue::Bool(self.use_physical)),
            "focal_length"     => Some(ReflectValue::F32(self.focal_length)),
            _ => None,
        }
    }

    fn set_field(&mut self, name: &str, value: ReflectValue) -> Result<(), String> {
        match (name, value) {
            ("fov",              ReflectValue::F32(f))     => self.fov = f.clamp(1.0, 179.0),
            ("near",             ReflectValue::F32(f))     => self.near = f.max(0.0001),
            ("far",              ReflectValue::F32(f))     => self.far = f.max(self.near + 0.1),
            ("ortho_size",       ReflectValue::F32(f))     => self.ortho_size = f.max(0.01),
            ("is_active",        ReflectValue::Bool(b))    => self.is_active = b,
            ("depth",            ReflectValue::F32(f))     => self.depth = f as i32,
            ("culling_mask",     ReflectValue::U32(u))     => self.culling_mask = u,
            ("allow_hdr",        ReflectValue::Bool(b))    => self.allow_hdr = b,
            ("allow_msaa",       ReflectValue::Bool(b))    => self.allow_msaa = b,
            ("use_physical",     ReflectValue::Bool(b))    => self.use_physical = b,
            ("focal_length",     ReflectValue::F32(f))     => self.focal_length = f.max(1.0),
            ("background_color", ReflectValue::Color4(c))  => self.background_color = c,
            ("projection", ReflectValue::Enum(s)) => {
                self.projection_mode = match s.as_str() {
                    "Perspective"  => ProjectionMode::Perspective,
                    "Orthographic" => ProjectionMode::Orthographic,
                    other => return Err(format!("Unknown projection mode '{}'", other)),
                };
            }
            ("clear_flags", ReflectValue::Enum(s)) => {
                self.clear_flags = ClearFlags::from_str(&s);
            }
            (n, _) => return Err(format!("Unknown or type-mismatched field '{}' on Camera", n)),
        }
        Ok(())
    }

    fn to_serialized_data(&self) -> serde_json::Value {
        serde_json::json!({
            "fov":            self.fov,
            "near":           self.near,
            "far":            self.far,
            "isOrthographic": self.projection_mode == ProjectionMode::Orthographic,
            "orthoSize":      self.ortho_size,
            "isMain":         self.is_active,
        })
    }
}

// ── Light ──────────────────────────────────────────────────────────────────────

static LIGHT_FIELDS: OnceLock<Vec<FieldDescriptor>> = OnceLock::new();

fn light_fields() -> &'static [FieldDescriptor] {
    LIGHT_FIELDS.get_or_init(|| vec![
        FieldDescriptor::new("light_type", "Type", ReflectFieldType::Enum)
            .with_variants(&["Directional", "Point", "Spot"]),
        FieldDescriptor::new("color",     "Color",     ReflectFieldType::Color3),
        FieldDescriptor::new("intensity", "Intensity", ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.0, 200_000.0)),
        FieldDescriptor::new("range", "Range", ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.1, 10_000.0))
            .with_visible_if(|c| !matches!(c.get_field("light_type"),
                Some(ReflectValue::Enum(ref s)) if s == "Directional")),
        FieldDescriptor::new("spot_angle", "Spot Angle", ReflectFieldType::F32)
            .with_range(RangeHint::min_max(1.0, 179.0))
            .with_visible_if(|c| matches!(c.get_field("light_type"),
                Some(ReflectValue::Enum(ref s)) if s == "Spot")),
        FieldDescriptor::new("spot_penumbra", "Penumbra", ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.0, 1.0))
            .with_visible_if(|c| matches!(c.get_field("light_type"),
                Some(ReflectValue::Enum(ref s)) if s == "Spot")),
        FieldDescriptor::new("cast_shadow", "Cast Shadows", ReflectFieldType::Bool),
        FieldDescriptor::new("shadow_map_size", "Shadow Map Size", ReflectFieldType::U32)
            .with_group("Shadows")
            .with_visible_if(|c| matches!(c.get_field("cast_shadow"),
                Some(ReflectValue::Bool(true)))),
        FieldDescriptor::new("shadow_bias", "Shadow Bias", ReflectFieldType::F32)
            .with_range(RangeHint::step(0.0001))
            .with_group("Shadows")
            .with_visible_if(|c| matches!(c.get_field("cast_shadow"),
                Some(ReflectValue::Bool(true)))),
    ])
}

impl Reflect for Light {
    fn reflect_type_name(&self) -> &'static str { "Light" }
    fn fields(&self) -> &'static [FieldDescriptor] { light_fields() }

    fn get_field(&self, name: &str) -> Option<ReflectValue> {
        match name {
            "light_type"      => Some(ReflectValue::Enum(match self.light_type {
                LightType::Directional => "Directional".into(),
                LightType::Point       => "Point".into(),
                LightType::Spot        => "Spot".into(),
            })),
            "color"           => Some(ReflectValue::Color3(self.color)),
            "intensity"       => Some(ReflectValue::F32(self.intensity)),
            "range"           => Some(ReflectValue::F32(self.range)),
            "spot_angle"      => Some(ReflectValue::F32(self.spot_angle)),
            "spot_penumbra"   => Some(ReflectValue::F32(self.spot_penumbra)),
            "cast_shadow"     => Some(ReflectValue::Bool(self.cast_shadow)),
            "shadow_map_size" => Some(ReflectValue::U32(self.shadow_map_size)),
            "shadow_bias"     => Some(ReflectValue::F32(self.shadow_bias)),
            _ => None,
        }
    }

    fn set_field(&mut self, name: &str, value: ReflectValue) -> Result<(), String> {
        match (name, value) {
            ("light_type", ReflectValue::Enum(s)) => {
                self.light_type = match s.as_str() {
                    "Directional" => LightType::Directional,
                    "Point"       => LightType::Point,
                    "Spot"        => LightType::Spot,
                    other => return Err(format!("Unknown light type '{}'", other)),
                };
                if self.light_type == LightType::Directional {
                    self.range = f32::MAX;
                }
            }
            ("color",           ReflectValue::Color3(c)) => self.color = c,
            ("intensity",       ReflectValue::F32(f))    => self.intensity = f.max(0.0),
            ("range",           ReflectValue::F32(f))    => self.range = f.max(0.0),
            ("spot_angle",      ReflectValue::F32(f))    => self.spot_angle = f.clamp(0.1, 179.9),
            ("spot_penumbra",   ReflectValue::F32(f))    => self.spot_penumbra = f.clamp(0.0, 1.0),
            ("cast_shadow",     ReflectValue::Bool(b))   => self.cast_shadow = b,
            ("shadow_map_size", ReflectValue::U32(n))    => self.shadow_map_size = n,
            ("shadow_bias",     ReflectValue::F32(f))    => self.shadow_bias = f,
            (n, _) => return Err(format!("Unknown or type-mismatched field '{}' on Light", n)),
        }
        Ok(())
    }

    fn to_serialized_data(&self) -> serde_json::Value {
        let lt = match self.light_type {
            LightType::Directional => "directional",
            LightType::Point       => "point",
            LightType::Spot        => "spot",
        };
        serde_json::json!({
            "lightType":    lt,
            "color":        self.color,
            "intensity":    self.intensity,
            "range":        self.range,
            "spotAngle":    self.spot_angle,
            "spotPenumbra": self.spot_penumbra,
            "castShadow":   self.cast_shadow,
            "shadowMapSize": self.shadow_map_size,
            "shadowBias":   self.shadow_bias,
        })
    }
}

// ── ParticleEmitter ────────────────────────────────────────────────────────────

static PARTICLE_EMITTER_FIELDS: OnceLock<Vec<FieldDescriptor>> = OnceLock::new();

fn particle_emitter_fields() -> &'static [FieldDescriptor] {
    PARTICLE_EMITTER_FIELDS.get_or_init(|| vec![
        FieldDescriptor::new("max_particles",    "Max Particles",    ReflectFieldType::USize)
            .with_range(RangeHint::min_max(1.0, 10_000.0)),
        FieldDescriptor::new("spawn_per_second", "Spawn / Second",   ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.0, 1_000.0)),
        FieldDescriptor::new("lifetime",         "Lifetime (s)",     ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.01, 60.0)),
        FieldDescriptor::new("start_speed",      "Start Speed",      ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.0, 100.0)),
        FieldDescriptor::new("gravity",          "Gravity",          ReflectFieldType::Vec3),
        FieldDescriptor::new("color",            "Color",            ReflectFieldType::Color4),
        FieldDescriptor::new("size",             "Particle Size",    ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.001, 10.0)),
        FieldDescriptor::new("spread_degrees",   "Spread (degrees)", ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.0, 180.0)),
    ])
}

impl Reflect for ParticleEmitter {
    fn reflect_type_name(&self) -> &'static str { "ParticleEmitter" }
    fn fields(&self) -> &'static [FieldDescriptor] { particle_emitter_fields() }

    fn get_field(&self, name: &str) -> Option<ReflectValue> {
        match name {
            "max_particles"    => Some(ReflectValue::USize(self.max_particles)),
            "spawn_per_second" => Some(ReflectValue::F32(self.spawn_per_second)),
            "lifetime"         => Some(ReflectValue::F32(self.lifetime)),
            "start_speed"      => Some(ReflectValue::F32(self.start_speed)),
            "gravity"          => Some(ReflectValue::Vec3(self.gravity.to_array())),
            "color"            => Some(ReflectValue::Color4(self.color)),
            "size"             => Some(ReflectValue::F32(self.size)),
            "spread_degrees"   => Some(ReflectValue::F32(self.spread_degrees)),
            _ => None,
        }
    }

    fn set_field(&mut self, name: &str, value: ReflectValue) -> Result<(), String> {
        match (name, value) {
            ("max_particles",    ReflectValue::USize(n)) => self.max_particles = n.max(1),
            ("spawn_per_second", ReflectValue::F32(f))   => self.spawn_per_second = f.max(0.0),
            ("lifetime",         ReflectValue::F32(f))   => self.lifetime = f.max(0.01),
            ("start_speed",      ReflectValue::F32(f))   => self.start_speed = f.max(0.0),
            ("gravity",          ReflectValue::Vec3(v))  => self.gravity = Vec3::from(v),
            ("color",            ReflectValue::Color4(c))=> self.color = c,
            ("size",             ReflectValue::F32(f))   => self.size = f.max(0.001),
            ("spread_degrees",   ReflectValue::F32(f))   => self.spread_degrees = f.clamp(0.0, 180.0),
            (n, _) => return Err(format!("Unknown or type-mismatched field '{}' on ParticleEmitter", n)),
        }
        Ok(())
    }

    fn to_serialized_data(&self) -> serde_json::Value {
        serde_json::json!({
            "maxParticles":   self.max_particles,
            "spawnPerSecond": self.spawn_per_second,
            "lifetime":       self.lifetime,
            "startSpeed":     self.start_speed,
            "gravity":        [self.gravity.x, self.gravity.y, self.gravity.z],
            "color":          self.color,
            "size":           self.size,
            "spreadDegrees":  self.spread_degrees,
        })
    }
}

// ── RigidBody ──────────────────────────────────────────────────────────────────

static RIGID_BODY_FIELDS: OnceLock<Vec<FieldDescriptor>> = OnceLock::new();

fn rigid_body_fields() -> &'static [FieldDescriptor] {
    RIGID_BODY_FIELDS.get_or_init(|| vec![
        FieldDescriptor::new("body_type", "Body Type", ReflectFieldType::Enum)
            .with_variants(&["Dynamic", "Kinematic", "Static"]),
        FieldDescriptor::new("shape", "Shape", ReflectFieldType::Enum)
            .with_variants(&["Box", "Sphere", "Capsule", "HalfSpace"]),
        FieldDescriptor::new("shape_param_x", "Size X", ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.001, 100.0))
            .with_visible_if(|c| !matches!(c.get_field("shape"),
                Some(ReflectValue::Enum(ref s)) if s == "HalfSpace")),
        FieldDescriptor::new("shape_param_y", "Size Y", ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.001, 100.0))
            .with_visible_if(|c| matches!(c.get_field("shape"),
                Some(ReflectValue::Enum(ref s)) if s == "Box" || s == "Capsule")),
        FieldDescriptor::new("shape_param_z", "Size Z", ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.001, 100.0))
            .with_visible_if(|c| matches!(c.get_field("shape"),
                Some(ReflectValue::Enum(ref s)) if s == "Box")),
        FieldDescriptor::new("mass", "Mass (kg)", ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.001, 100_000.0))
            .with_visible_if(|c| matches!(c.get_field("body_type"),
                Some(ReflectValue::Enum(ref s)) if s == "Dynamic")),
        FieldDescriptor::new("restitution",     "Restitution",    ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.0, 1.0)),
        FieldDescriptor::new("friction",        "Friction",       ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.0, 10.0)),
        FieldDescriptor::new("linear_damping",  "Linear Damping", ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.0, 100.0))
            .with_visible_if(|c| matches!(c.get_field("body_type"),
                Some(ReflectValue::Enum(ref s)) if s == "Dynamic")),
        FieldDescriptor::new("angular_damping", "Angular Damping", ReflectFieldType::F32)
            .with_range(RangeHint::min_max(0.0, 100.0))
            .with_visible_if(|c| matches!(c.get_field("body_type"),
                Some(ReflectValue::Enum(ref s)) if s == "Dynamic")),
        FieldDescriptor::new("gravity_scale", "Gravity Scale", ReflectFieldType::F32)
            .with_range(RangeHint::min_max(-10.0, 10.0))
            .with_visible_if(|c| matches!(c.get_field("body_type"),
                Some(ReflectValue::Enum(ref s)) if s != "Static")),
        FieldDescriptor::new("can_sleep", "Can Sleep", ReflectFieldType::Bool),
    ])
}

impl Reflect for RigidBody {
    fn reflect_type_name(&self) -> &'static str { "RigidBody" }
    fn fields(&self) -> &'static [FieldDescriptor] { rigid_body_fields() }

    fn get_field(&self, name: &str) -> Option<ReflectValue> {
        match name {
            "body_type"       => Some(ReflectValue::Enum(self.body_type.as_str().to_string())),
            "shape"           => Some(ReflectValue::Enum(self.shape.as_str().to_string())),
            "mass"            => Some(ReflectValue::F32(self.mass)),
            "restitution"     => Some(ReflectValue::F32(self.restitution)),
            "friction"        => Some(ReflectValue::F32(self.friction)),
            "linear_damping"  => Some(ReflectValue::F32(self.linear_damping)),
            "angular_damping" => Some(ReflectValue::F32(self.angular_damping)),
            "gravity_scale"   => Some(ReflectValue::F32(self.gravity_scale)),
            "can_sleep"       => Some(ReflectValue::Bool(self.can_sleep)),
            // Shape dimension accessors (read component shape params as x/y/z)
            "shape_param_x"   => Some(ReflectValue::F32(match self.shape {
                PhysicsShape::Box { half_extents }   => half_extents[0],
                PhysicsShape::Sphere { radius }      => radius,
                PhysicsShape::Capsule { radius, .. } => radius,
                PhysicsShape::HalfSpace              => 0.0,
            })),
            "shape_param_y"   => Some(ReflectValue::F32(match self.shape {
                PhysicsShape::Box { half_extents }        => half_extents[1],
                PhysicsShape::Capsule { half_height, .. } => half_height,
                _                                          => 0.0,
            })),
            "shape_param_z"   => Some(ReflectValue::F32(match self.shape {
                PhysicsShape::Box { half_extents } => half_extents[2],
                _                                   => 0.0,
            })),
            _ => None,
        }
    }

    fn set_field(&mut self, name: &str, value: ReflectValue) -> Result<(), String> {
        match (name, value) {
            ("body_type", ReflectValue::Enum(s)) => {
                self.body_type = match s.as_str() {
                    "Dynamic"   => BodyType::Dynamic,
                    "Kinematic" => BodyType::Kinematic,
                    "Static"    => BodyType::Static,
                    o => return Err(format!("Unknown BodyType '{}'", o)),
                };
            }
            ("shape", ReflectValue::Enum(s)) => {
                self.shape = match s.as_str() {
                    "Box"       => PhysicsShape::Box { half_extents: [0.5, 0.5, 0.5] },
                    "Sphere"    => PhysicsShape::Sphere { radius: 0.5 },
                    "Capsule"   => PhysicsShape::Capsule { half_height: 0.5, radius: 0.25 },
                    "HalfSpace" => PhysicsShape::HalfSpace,
                    o => return Err(format!("Unknown PhysicsShape '{}'", o)),
                };
            }
            ("mass",            ReflectValue::F32(f)) => self.mass = f.max(0.001),
            ("restitution",     ReflectValue::F32(f)) => self.restitution = f.clamp(0.0, 1.0),
            ("friction",        ReflectValue::F32(f)) => self.friction = f.max(0.0),
            ("linear_damping",  ReflectValue::F32(f)) => self.linear_damping = f.max(0.0),
            ("angular_damping", ReflectValue::F32(f)) => self.angular_damping = f.max(0.0),
            ("gravity_scale",   ReflectValue::F32(f)) => self.gravity_scale = f,
            ("can_sleep",       ReflectValue::Bool(b)) => self.can_sleep = b,
            ("shape_param_x",   ReflectValue::F32(f)) => match &mut self.shape {
                PhysicsShape::Box { half_extents }   => half_extents[0] = f.max(0.001),
                PhysicsShape::Sphere { radius }      => *radius = f.max(0.001),
                PhysicsShape::Capsule { radius, .. } => *radius = f.max(0.001),
                PhysicsShape::HalfSpace              => {}
            },
            ("shape_param_y",   ReflectValue::F32(f)) => match &mut self.shape {
                PhysicsShape::Box { half_extents }        => half_extents[1] = f.max(0.001),
                PhysicsShape::Capsule { half_height, .. } => *half_height = f.max(0.001),
                _                                          => {}
            },
            ("shape_param_z",   ReflectValue::F32(f)) => match &mut self.shape {
                PhysicsShape::Box { half_extents } => half_extents[2] = f.max(0.001),
                _                                   => {}
            },
            (n, _) => return Err(format!("Unknown or type-mismatched field '{}' on RigidBody", n)),
        }
        Ok(())
    }

    fn to_serialized_data(&self) -> serde_json::Value {
        serde_json::json!({
            "bodyType":      self.body_type.as_str(),
            "shape":         serde_json::to_value(&self.shape).unwrap_or(serde_json::Value::Null),
            "mass":          self.mass,
            "linearDamping": self.linear_damping,
            "angularDamping":self.angular_damping,
            "gravityScale":  self.gravity_scale,
            "canSleep":      self.can_sleep,
            "restitution":   self.restitution,
            "friction":      self.friction,
        })
    }
}

