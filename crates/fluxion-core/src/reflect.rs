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

// ── Method Reflection ───────────────────────────────────────────────────────────

/// Parameter metadata for reflected methods.
#[derive(Debug, Clone)]
pub struct ParameterDescriptor {
    /// Parameter name
    pub name: &'static str,
    /// Human-readable display name
    pub display_name: &'static str,
    /// Parameter type
    pub param_type: ReflectFieldType,
    /// Whether parameter has a default value
    pub has_default: bool,
    /// Default value serialised as a JSON string (e.g. `"0"`, `"true"`, `"\"hello\""`).
    /// Stored as `&'static str` so the struct can be `const`-constructed.
    pub default_json: Option<&'static str>,
    /// Whether parameter is optional
    pub optional: bool,
}

impl ParameterDescriptor {
    pub const fn new(name: &'static str, display_name: &'static str, param_type: ReflectFieldType) -> Self {
        ParameterDescriptor {
            name,
            display_name,
            param_type,
            has_default: false,
            default_json: None,
            optional: false,
        }
    }

    pub const fn with_default_json(mut self, json: &'static str) -> Self {
        self.has_default = true;
        self.default_json = Some(json);
        self
    }

    pub const fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
}

/// Method types for reflection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodType {
    /// Static method (no self parameter)
    Static,
    /// Instance method (takes &self)
    Instance,
    /// Instance method with mutable access (takes &mut self)
    InstanceMut,
}

/// Method visibility levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodVisibility {
    Public,
    Private,
    Internal,
}

/// Method metadata for reflection.
#[derive(Debug, Clone)]
pub struct MethodDescriptor {
    /// Method name
    pub name: &'static str,
    /// Human-readable display name
    pub display_name: &'static str,
    /// Method description
    pub description: &'static str,
    /// Method type (static/instance)
    pub method_type: MethodType,
    /// Visibility level
    pub visibility: MethodVisibility,
    /// Return type (None for void)
    pub return_type: Option<ReflectFieldType>,
    /// Parameter list
    pub parameters: &'static [ParameterDescriptor],
    /// Whether method is async
    pub is_async: bool,
    /// Whether method is a coroutine (Unity-style)
    pub is_coroutine: bool,
    /// Method attributes (e.g., [ContextMenu], [Header])
    pub attributes: &'static [&'static str],
}

impl MethodDescriptor {
    pub const fn new(name: &'static str, display_name: &'static str, method_type: MethodType) -> Self {
        MethodDescriptor {
            name,
            display_name,
            description: "",
            method_type,
            visibility: MethodVisibility::Public,
            return_type: None,
            parameters: &[],
            is_async: false,
            is_coroutine: false,
            attributes: &[],
        }
    }

    pub const fn with_description(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    pub const fn with_return_type(mut self, return_type: ReflectFieldType) -> Self {
        self.return_type = Some(return_type);
        self
    }

    pub const fn with_parameters(mut self, parameters: &'static [ParameterDescriptor]) -> Self {
        self.parameters = parameters;
        self
    }

    pub const fn private(mut self) -> Self {
        self.visibility = MethodVisibility::Private;
        self
    }

    pub const fn async_fn(mut self) -> Self {
        self.is_async = true;
        self
    }

    pub const fn coroutine(mut self) -> Self {
        self.is_coroutine = true;
        self
    }

    pub const fn with_attributes(mut self, attributes: &'static [&'static str]) -> Self {
        self.attributes = attributes;
        self
    }
}

/// Trait for types that can reflect their methods.
pub trait ReflectMethods {
    /// Get all method descriptors for this type
    fn methods() -> &'static [MethodDescriptor] where Self: Sized;
    
    /// Invoke a static method by name
    fn invoke_static(method_name: &str, args: &[ReflectValue]) -> Result<Option<ReflectValue>, String> where Self: Sized {
        Err(format!("Static method '{}' not implemented for type", method_name))
    }
    
    /// Invoke an instance method by name
    fn invoke_method(&self, method_name: &str, args: &[ReflectValue]) -> Result<Option<ReflectValue>, String> {
        Err(format!("Instance method '{}' not implemented", method_name))
    }
    
    /// Invoke a mutable instance method by name
    fn invoke_method_mut(&mut self, method_name: &str, args: &[ReflectValue]) -> Result<Option<ReflectValue>, String> {
        Err(format!("Mutable instance method '{}' not implemented", method_name))
    }
}
