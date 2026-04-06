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
    /// Signed 32-bit integer (layer ordering, depth priority, …)
    I32(i32),
    /// 2D vector stored as [x, y]
    Vec2([f32; 2]),
    /// Enum variant name as a string ("Cube", "Directional", …)
    Enum(String),
    /// Project-relative path to an asset file (texture, mesh, audio, …).
    /// Shown as an asset-picker widget in the editor (drag-and-drop from asset browser).
    AssetPath(Option<String>),
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
    /// Project-relative path to a texture/image asset.
    /// The editor shows a thumbnail + drag-drop target.
    Texture,
    /// Signed 32-bit integer — shown as drag_int in the editor.
    I32,
    /// 2D vector [x, y] — shown as two drag-float fields in the editor.
    Vec2,
    /// Project-relative path to a `.fluxmat` material asset.
    /// Unity equivalent: `public Material mat;`
    Material,
    /// Project-relative path to a mesh file (.glb, .gltf, .fluxmesh).
    /// Unity equivalent: `public Mesh mesh;`
    Mesh,
    /// Project-relative path to an audio clip asset.
    /// Unity equivalent: `public AudioClip clip;`
    Audio,
    /// Project-relative path to a scene file (.scene).
    /// Unity equivalent: scene string reference.
    Scene,
    /// Reference to another entity by ID (stored as i64, -1 = none).
    /// Unity equivalent: `public GameObject go;`
    EntityRef,
}

/// How the editor should render a numeric or Vec3 field.
///
/// - `Default`      — standard drag widget (DragFloat / DragInt / XYZ inline)
/// - `Slider`        — visible range bar (Unity `[Range]` slider)
/// - `UniformScale`  — Vec3 with a uniform scale lock button
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderHint {
    #[default]
    Default,
    /// Show a visual range slider instead of a drag widget.
    /// Requires `range.min` and `range.max` to be set.
    Slider,
    /// For Vec3 fields: adds a lock button that keeps X/Y/Z proportional.
    UniformScale,
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
#[derive(Clone)]
pub struct FieldDescriptor {
    /// Internal identifier used in `get_field` / `set_field`.
    pub name: &'static str,
    /// Human-readable label shown in the editor UI ("Cast Shadow", "Field Of View").
    pub display_name: &'static str,
    /// The type of value this field holds.
    pub field_type: ReflectFieldType,
    /// Optional hint for numeric fields (shown as slider or drag widget).
    pub range: RangeHint,
    /// How the editor should render this field.
    /// Unity equivalent: `[Range]` on float → `Slider`; scale Vec3 → `UniformScale`.
    pub render_hint: RenderHint,
    /// Whether the field is read-only in the editor (e.g. world-space cache).
    pub read_only: bool,
    /// For `Enum` fields: the list of valid variant name strings.
    /// Used by the editor to populate a combo-box.
    pub enum_variants: Option<&'static [&'static str]>,
    /// Inspector group name — fields with the same group render inside a collapsible
    /// sub-section. `None` means the field is shown at the top level.
    pub group: Option<&'static str>,
    /// Optional predicate controlling visibility in the inspector.
    ///
    /// Receives the owning component as `&dyn Reflect` so it can call `get_field`
    /// to read sibling values. Returns `true` when the field should be shown.
    /// `None` means always visible.
    pub visible_if: Option<fn(&dyn Reflect) -> bool>,
    /// [Header("Section Name")] — bold non-collapsible section label shown ABOVE this field.
    /// Unity equivalent: `[Header("My Section")]`.
    pub header: Option<&'static str>,
    /// [Tooltip("...")] — hover tooltip shown as a small ⓘ label next to the field.
    /// Unity equivalent: `[Tooltip("description")]`.
    pub tooltip: Option<&'static str>,
}

impl std::fmt::Debug for FieldDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FieldDescriptor")
            .field("name", &self.name)
            .field("display_name", &self.display_name)
            .field("field_type", &self.field_type)
            .field("group", &self.group)
            .field("read_only", &self.read_only)
            .finish()
    }
}

impl FieldDescriptor {
    /// Convenience constructor for a plain editable field with no range hint.
    pub const fn new(name: &'static str, display_name: &'static str, field_type: ReflectFieldType) -> Self {
        FieldDescriptor {
            name,
            display_name,
            field_type,
            range: RangeHint::none(),
            render_hint: RenderHint::Default,
            read_only: false,
            enum_variants: None,
            group: None,
            visible_if: None,
            header: None,
            tooltip: None,
        }
    }
    /// Same as `new` but marks the field as read-only.
    pub const fn read_only(name: &'static str, display_name: &'static str, field_type: ReflectFieldType) -> Self {
        FieldDescriptor {
            name,
            display_name,
            field_type,
            range: RangeHint::none(),
            render_hint: RenderHint::Default,
            read_only: true,
            enum_variants: None,
            group: None,
            visible_if: None,
            header: None,
            tooltip: None,
        }
    }
    pub const fn with_range(mut self, range: RangeHint) -> Self {
        self.range = range;
        self
    }
    /// Attach allowed enum variant names to this descriptor (for combo-box rendering).
    pub const fn with_variants(mut self, variants: &'static [&'static str]) -> Self {
        self.enum_variants = Some(variants);
        self
    }
    /// Assign this field to a named inspector group (collapsible sub-section).
    pub const fn with_group(mut self, group: &'static str) -> Self {
        self.group = Some(group);
        self
    }
    /// Set a visibility predicate. The field is hidden when the predicate returns `false`.
    pub const fn with_visible_if(mut self, f: fn(&dyn Reflect) -> bool) -> Self {
        self.visible_if = Some(f);
        self
    }
    /// Set the render hint (slider, uniform_scale, etc.).
    /// Unity equivalent: `[Range]` on float → `RenderHint::Slider`.
    pub const fn with_render_hint(mut self, hint: RenderHint) -> Self {
        self.render_hint = hint;
        self
    }
    /// Set a [Header("...")] label shown above this field in the inspector.
    /// Unity equivalent: `[Header("Section Name")]`.
    pub const fn with_header(mut self, header: &'static str) -> Self {
        self.header = Some(header);
        self
    }
    /// Set a [Tooltip("...")] hover text for this field.
    /// Unity equivalent: `[Tooltip("description")]`.
    pub const fn with_tooltip(mut self, tooltip: &'static str) -> Self {
        self.tooltip = Some(tooltip);
        self
    }
    /// Evaluate whether this field is currently visible given the component's state.
    /// Returns `true` when there is no predicate (always visible).
    pub fn is_visible(&self, component: &dyn Reflect) -> bool {
        match self.visible_if {
            Some(f) => f(component),
            None    => true,
        }
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
        ReflectValue::I32(n)        => Value::from(*n),
        ReflectValue::Vec2(a)       => Value::Array(a.iter().map(|&x| Value::from(x)).collect()),
        ReflectValue::Enum(s)       => Value::String(s.clone()),
        ReflectValue::AssetPath(o)  => o.as_ref().map(|s| Value::String(s.clone())).unwrap_or(Value::Null),
    }
}

/// Map a `ReflectFieldType` to the string tag used by the Rune inspector.
pub fn field_type_str(ft: ReflectFieldType) -> &'static str {
    match ft {
        ReflectFieldType::F32      => "f32",
        ReflectFieldType::Vec3     => "vec3",
        ReflectFieldType::Quat     => "quat",
        ReflectFieldType::Color3   => "color3",
        ReflectFieldType::Color4   => "color4",
        ReflectFieldType::Bool     => "bool",
        ReflectFieldType::U32      => "u32",
        ReflectFieldType::U8       => "u8",
        ReflectFieldType::USize    => "usize",
        ReflectFieldType::Str      => "str",
        ReflectFieldType::OptionStr => "option_str",
        ReflectFieldType::Enum     => "enum",
        ReflectFieldType::Texture  => "texture",
        ReflectFieldType::I32      => "i32",
        ReflectFieldType::Vec2     => "vec2",
        ReflectFieldType::Material => "material",
        ReflectFieldType::Mesh     => "mesh",
        ReflectFieldType::Audio    => "audio",
        ReflectFieldType::Scene    => "scene",
        ReflectFieldType::EntityRef => "entity_ref",
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
    fn invoke_static(method_name: &str, _args: &[ReflectValue]) -> Result<Option<ReflectValue>, String> where Self: Sized {
        Err(format!("Static method '{}' not implemented for type", method_name))
    }
    
    /// Invoke an instance method by name
    fn invoke_method(&self, method_name: &str, _args: &[ReflectValue]) -> Result<Option<ReflectValue>, String> {
        Err(format!("Instance method '{}' not implemented", method_name))
    }
    
    /// Invoke a mutable instance method by name
    fn invoke_method_mut(&mut self, method_name: &str, _args: &[ReflectValue]) -> Result<Option<ReflectValue>, String> {
        Err(format!("Mutable instance method '{}' not implemented", method_name))
    }
}
