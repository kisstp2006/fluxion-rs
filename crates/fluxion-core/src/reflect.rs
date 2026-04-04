// ============================================================
// fluxion-core — Reflect trait
//
// Provides runtime field inspection and mutation for components.
// This is the foundation for the editor property panel, scene
// saving (world → JSON), and copy/paste of component data.
//
// Design
// ──────
// - `ReflectValue`    — a typed dynamic value (f32, Vec3, bool, …)
// - `FieldDescriptor` — static metadata for one field (name, type, hints)
// - `Reflect`         — trait implemented by each reflectable component
//
// Editor usage example
// ────────────────────
//   // List fields and their current values:
//   if let Some(comp) = registry.get_reflect("Transform", &world, entity) {
//       for field in comp.fields() {
//           let value = comp.get_field(field.name);
//           ui.show_field(field, value);
//       }
//   }
//
//   // Apply a user edit:
//   registry.set_reflect_field("Transform", &world, entity, "position",
//       ReflectValue::Vec3([1.0, 2.0, 3.0]))?;
// ============================================================

use serde_json::Value;

// ── Dynamic value type ─────────────────────────────────────────────────────────

/// A typed dynamic value used for reading and writing component fields at runtime.
#[derive(Debug, Clone, PartialEq)]
pub enum ReflectValue {
    /// Single f32 (fov, intensity, range, …)
    F32(f32),
    /// glam Vec3 stored as [x, y, z]
    Vec3([f32; 3]),
    /// glam Quat stored as [x, y, z, w]
    Quat([f32; 4]),
    /// Linear RGB color [r, g, b]  — shown as color picker in editor
    Color3([f32; 3]),
    /// Linear RGBA color [r, g, b, a] — shown as color picker with alpha
    Color4([f32; 4]),
    Bool(bool),
    U32(u32),
    U8(u8),
    USize(usize),
    Str(String),
    OptionStr(Option<String>),
    /// Enum variant name as a string ("Cube", "Directional", …)
    Enum(String),
}

// ── Field metadata ─────────────────────────────────────────────────────────────

/// Which kind of value a field holds (mirrors `ReflectValue` variants).
/// Used by the editor to pick the right widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReflectFieldType {
    F32,
    Vec3,
    Quat,
    Color3,
    Color4,
    Bool,
    U32,
    U8,
    USize,
    Str,
    OptionStr,
    Enum,
}

/// Optional numeric range hint for editor sliders / drag fields.
#[derive(Debug, Clone, Copy)]
pub struct RangeHint {
    pub min:  Option<f32>,
    pub max:  Option<f32>,
    pub step: Option<f32>,
}

impl RangeHint {
    pub const fn none() -> Self { RangeHint { min: None, max: None, step: None } }
    pub const fn min_max(min: f32, max: f32) -> Self {
        RangeHint { min: Some(min), max: Some(max), step: None }
    }
    pub const fn step(step: f32) -> Self {
        RangeHint { min: None, max: None, step: Some(step) }
    }
}

/// Static descriptor for one field on a component.
///
/// The `fields()` method on `Reflect` returns a `&'static [FieldDescriptor]`.
#[derive(Debug, Clone)]
pub struct FieldDescriptor {
    /// Internal identifier used in `get_field` / `set_field`.
    pub name: &'static str,
    /// Human-readable label shown in the editor UI ("Cast Shadow", "Field Of View").
    pub display_name: &'static str,
    /// The type of value this field holds.
    pub field_type: ReflectFieldType,
    /// Optional hint for numeric fields (shown as slider or drag widget).
    pub range: RangeHint,
    /// Whether the field is read-only in the editor (e.g. world-space cache).
    pub read_only: bool,
}

impl FieldDescriptor {
    /// Convenience constructor for a plain editable field with no range hint.
    pub const fn new(name: &'static str, display_name: &'static str, field_type: ReflectFieldType) -> Self {
        FieldDescriptor {
            name,
            display_name,
            field_type,
            range: RangeHint::none(),
            read_only: false,
        }
    }
    /// Same as `new` but marks the field as read-only.
    pub const fn read_only(name: &'static str, display_name: &'static str, field_type: ReflectFieldType) -> Self {
        FieldDescriptor {
            name,
            display_name,
            field_type,
            range: RangeHint::none(),
            read_only: true,
        }
    }
    pub const fn with_range(mut self, range: RangeHint) -> Self {
        self.range = range;
        self
    }
}

// ── Reflect trait ──────────────────────────────────────────────────────────────

/// Runtime reflection for a component.
///
/// Implementors expose their fields as named `ReflectValue` entries so that
/// the editor can display and edit them without knowing the concrete type.
pub trait Reflect: Send + Sync + 'static {
    /// The short type name used in scene files ("Transform", "Camera", …).
    fn reflect_type_name(&self) -> &'static str;

    /// Static list of field descriptors for this component type.
    fn fields(&self) -> &'static [FieldDescriptor];

    /// Read a field value by name. Returns `None` for unknown field names.
    fn get_field(&self, name: &str) -> Option<ReflectValue>;

    /// Write a field value by name.
    ///
    /// Returns `Err` if:
    ///   - the field name is unknown
    ///   - the `ReflectValue` variant doesn't match the field type
    ///   - the value is out of valid range (implementor may choose to clamp instead)
    fn set_field(&mut self, name: &str, value: ReflectValue) -> Result<(), String>;

    /// Serialize current state to the scene JSON format.
    ///
    /// This is used by the scene saver to write the component's `data` block.
    /// The default implementation iterates `fields()` and builds a JSON object
    /// from `get_field()` return values. Override for custom layouts.
    fn to_serialized_data(&self) -> Value {
        let mut map = serde_json::Map::new();
        for field in self.fields() {
            if field.read_only { continue; }
            if let Some(v) = self.get_field(field.name) {
                map.insert(field.name.to_string(), reflect_value_to_json(&v));
            }
        }
        Value::Object(map)
    }
}

// ── JSON helpers ───────────────────────────────────────────────────────────────

/// Convert a `ReflectValue` to its JSON representation.
pub fn reflect_value_to_json(v: &ReflectValue) -> Value {
    match v {
        ReflectValue::F32(f)        => Value::from(*f),
        ReflectValue::Vec3(a)       => Value::Array(a.iter().map(|&x| Value::from(x)).collect()),
        ReflectValue::Quat(a)       => Value::Array(a.iter().map(|&x| Value::from(x)).collect()),
        ReflectValue::Color3(a)     => Value::Array(a.iter().map(|&x| Value::from(x)).collect()),
        ReflectValue::Color4(a)     => Value::Array(a.iter().map(|&x| Value::from(x)).collect()),
        ReflectValue::Bool(b)       => Value::Bool(*b),
        ReflectValue::U32(n)        => Value::from(*n),
        ReflectValue::U8(n)         => Value::from(*n),
        ReflectValue::USize(n)      => Value::from(*n),
        ReflectValue::Str(s)        => Value::String(s.clone()),
        ReflectValue::OptionStr(o)  => o.as_ref().map(|s| Value::String(s.clone())).unwrap_or(Value::Null),
        ReflectValue::Enum(s)       => Value::String(s.clone()),
    }
}
