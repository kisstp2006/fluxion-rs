// ============================================================
// fluxion-scripting — ScriptBindingRegistry
//
// Language-agnostic registry for all script-callable engine APIs.
// Stores both metadata (for editor/docs) and closures (for runtime).
//
// Architecture:
//   - ScriptBindingRegistry holds handler closures + MethodDescriptor metadata
//   - auto_binding.rs reads this registry to:
//       1. Register a single __native_invoke Rust→JS bridge (rquickjs)
//       2. Generate the JS global objects (Input, Physics, …) as code strings
//   - Same registry can feed Rune, editor autocomplete, API docs
//
// No rquickjs imports here — this module stays language-agnostic.
// ============================================================

use std::collections::HashMap;
use std::sync::Arc;

use fluxion_core::ReflectValue;

// ── Handler types ──────────────────────────────────────────────────────────────

/// A boxed script-callable function.
/// Receives positional args as ReflectValues, returns an optional result.
pub type NativeHandler = Arc<
    dyn Fn(&[ReflectValue]) -> Result<Option<ReflectValue>, String> + Send + Sync + 'static,
>;

// ── Parameter & return type descriptors ───────────────────────────────────────

/// JS-visible type tag — used for documentation and Rune type generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptType {
    Bool,
    Int,
    Float,
    String,
    Vec2,
    Vec3,
    Vec4,
    Quat,
    Void,
    Object,
    Array,
}

impl ScriptType {
    pub fn as_ts_str(self) -> &'static str {
        match self {
            ScriptType::Bool   => "boolean",
            ScriptType::Int    => "number",
            ScriptType::Float  => "number",
            ScriptType::String => "string",
            ScriptType::Vec2   => "Vector2",
            ScriptType::Vec3   => "Vector3",
            ScriptType::Vec4   => "Vector4",
            ScriptType::Quat   => "Quaternion",
            ScriptType::Void   => "void",
            ScriptType::Object => "object",
            ScriptType::Array  => "any[]",
        }
    }
    pub fn as_rune_str(self) -> &'static str {
        match self {
            ScriptType::Bool   => "bool",
            ScriptType::Int    => "i64",
            ScriptType::Float  => "f64",
            ScriptType::String => "String",
            ScriptType::Vec2   => "Vec2",
            ScriptType::Vec3   => "Vec3",
            ScriptType::Vec4   => "Vec4",
            ScriptType::Quat   => "Quat",
            ScriptType::Void   => "()",
            ScriptType::Object => "Value",
            ScriptType::Array  => "Vec<Value>",
        }
    }
}

/// Metadata for one parameter of a registered function.
#[derive(Debug, Clone)]
pub struct ParamMeta {
    pub name:        &'static str,
    pub param_type:  ScriptType,
    pub optional:    bool,
    pub description: &'static str,
}

impl ParamMeta {
    pub const fn new(name: &'static str, t: ScriptType) -> Self {
        ParamMeta { name, param_type: t, optional: false, description: "" }
    }
    pub const fn optional(mut self) -> Self { self.optional = true; self }
    pub const fn doc(mut self, d: &'static str) -> Self { self.description = d; self }
}

// ── Binding entry ──────────────────────────────────────────────────────────────

/// One registered callable in the engine API.
#[derive(Clone)]
pub struct BindingEntry {
    /// Short function name within the module (e.g. "GetKeyDown").
    pub name:        &'static str,
    /// Human-readable description (for API docs).
    pub description: &'static str,
    /// Parameter list (in order).
    pub params:      Vec<ParamMeta>,
    /// Return type (None = void).
    pub return_type: Option<ScriptType>,
    /// The Rust implementation.
    pub handler:     NativeHandler,
}

impl BindingEntry {
    /// Build a new entry with the given implementation.
    pub fn new(
        name: &'static str,
        description: &'static str,
        params: Vec<ParamMeta>,
        return_type: Option<ScriptType>,
        handler: impl Fn(&[ReflectValue]) -> Result<Option<ReflectValue>, String>
            + Send + Sync + 'static,
    ) -> Self {
        BindingEntry {
            name,
            description,
            params,
            return_type,
            handler: Arc::new(handler),
        }
    }
}

#[cfg(test)]
mod tests_binding {
    use super::*;
    use fluxion_core::ReflectValue;

    fn make_reg() -> ScriptBindingRegistry {
        let mut reg = ScriptBindingRegistry::new();
        reg.register("Math", BindingEntry::new(
            "Abs",
            "Returns absolute value",
            vec![ParamMeta::new("x", ScriptType::Float)],
            Some(ScriptType::Float),
            |args| match args.first() {
                Some(ReflectValue::F32(f)) => Ok(Some(ReflectValue::F32(f.abs()))),
                _ => Ok(Some(ReflectValue::F32(0.0))),
            },
        ));
        reg.register("Math", BindingEntry::new(
            "Max",
            "Returns larger of two numbers",
            vec![
                ParamMeta::new("a", ScriptType::Float),
                ParamMeta::new("b", ScriptType::Float),
            ],
            Some(ScriptType::Float),
            |args| {
                let a = match args.first() { Some(ReflectValue::F32(f)) => *f, _ => 0.0 };
                let b = match args.get(1)  { Some(ReflectValue::F32(f)) => *f, _ => 0.0 };
                Ok(Some(ReflectValue::F32(a.max(b))))
            },
        ));
        reg.register("Debug", BindingEntry::new(
            "Log",
            "Logs a message",
            vec![ParamMeta::new("msg", ScriptType::String)],
            None,
            |_| Ok(None),
        ));
        reg
    }

    #[test]
    fn test_module_names_sorted() {
        let reg = make_reg();
        let names = reg.module_names();
        assert_eq!(names, vec!["Debug", "Math"]);
    }

    #[test]
    fn test_find_handler_and_invoke() {
        let reg = make_reg();
        let result = reg.invoke("Math.Abs", &[ReflectValue::F32(-5.5)]);
        assert_eq!(result, Ok(Some(ReflectValue::F32(5.5))));
    }

    #[test]
    fn test_invoke_two_args() {
        let reg = make_reg();
        let result = reg.invoke("Math.Max", &[ReflectValue::F32(3.0), ReflectValue::F32(7.0)]);
        assert_eq!(result, Ok(Some(ReflectValue::F32(7.0))));
    }

    #[test]
    fn test_invoke_void_returns_none() {
        let reg = make_reg();
        let result = reg.invoke("Debug.Log", &[ReflectValue::Str("hi".into())]);
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn test_unknown_path_errors() {
        let reg = make_reg();
        let result = reg.invoke("Fake.DoThing", &[]);
        assert!(result.is_err(), "Expected Err for unknown path");
    }

    #[test]
    fn test_module_entries_count() {
        let reg = make_reg();
        assert_eq!(reg.module_entries("Math").len(), 2);
        assert_eq!(reg.module_entries("Debug").len(), 1);
        assert_eq!(reg.module_entries("Missing").len(), 0);
    }

    #[test]
    fn test_dts_generation() {
        let reg = make_reg();
        let dts = reg.generate_dts();
        assert!(dts.contains("declare namespace Math {"));
        assert!(dts.contains("function Abs(x: number): number;"));
        assert!(dts.contains("function Max(a: number, b: number): number;"));
        assert!(dts.contains("declare namespace Debug {"));
        assert!(dts.contains("function Log(msg: string): void;"));
    }

    #[test]
    fn test_param_meta_optional() {
        let p = ParamMeta::new("radius", ScriptType::Float)
            .optional()
            .doc("Sphere radius");
        assert!(p.optional);
        assert_eq!(p.description, "Sphere radius");
    }
}

// ── Registry ───────────────────────────────────────────────────────────────────

/// Central registry for all script-callable engine functions.
///
/// Populated once at startup by `api::register_all()`.
/// Used by `auto_binding` to wire up the JS / Rune VMs.
#[derive(Clone, Default)]
pub struct ScriptBindingRegistry {
    /// module_name → list of entries (insertion order preserved).
    modules: HashMap<String, Vec<BindingEntry>>,
}

impl ScriptBindingRegistry {
    pub fn new() -> Self { Self::default() }

    /// Register one callable under `module` (e.g. "Input", "Physics").
    pub fn register(&mut self, module: &'static str, entry: BindingEntry) {
        self.modules.entry(module.to_string()).or_default().push(entry);
    }

    /// Returns all entries in `module`, or an empty slice.
    pub fn module_entries(&self, module: &str) -> &[BindingEntry] {
        self.modules.get(module).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Sorted list of all registered module names.
    pub fn module_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.modules.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    /// Look up a handler by "Module.FnName" path (used by __native_invoke).
    pub fn find_handler(&self, full_path: &str) -> Option<&NativeHandler> {
        let dot = full_path.find('.')?;
        let module = &full_path[..dot];
        let name   = &full_path[dot + 1..];
        self.modules
            .get(module)?
            .iter()
            .find(|e| e.name == name)
            .map(|e| &e.handler)
    }

    /// Invoke a handler by "Module.FnName" path.
    pub fn invoke(
        &self,
        full_path: &str,
        args: &[ReflectValue],
    ) -> Result<Option<ReflectValue>, String> {
        let handler = self
            .find_handler(full_path)
            .ok_or_else(|| format!("Unknown API: '{}'", full_path))?;
        handler(args)
    }

    /// Generate a TypeScript `.d.ts` declaration string for all registered modules.
    pub fn generate_dts(&self) -> String {
        let mut out = String::from(
            "// Auto-generated TypeScript declarations — do not edit\n\n",
        );
        for module in self.module_names() {
            out.push_str(&format!("declare namespace {} {{\n", module));
            for e in self.module_entries(module) {
                // doc comment
                if !e.description.is_empty() {
                    out.push_str(&format!("  /** {} */\n", e.description));
                }
                // function signature
                let params: Vec<String> = e
                    .params
                    .iter()
                    .map(|p| {
                        if p.optional {
                            format!("{}?: {}", p.name, p.param_type.as_ts_str())
                        } else {
                            format!("{}: {}", p.name, p.param_type.as_ts_str())
                        }
                    })
                    .collect();
                let ret = e
                    .return_type
                    .map(|t| t.as_ts_str())
                    .unwrap_or("void");
                out.push_str(&format!(
                    "  function {}({}): {};\n",
                    e.name,
                    params.join(", "),
                    ret,
                ));
            }
            out.push_str("}\n\n");
        }
        out
    }

    /// Generate a Rune module stub string for all registered modules.
    pub fn generate_rune_stubs(&self) -> String {
        let mut out = String::from(
            "// Auto-generated Rune stubs — do not edit\n\n",
        );
        for module in self.module_names() {
            out.push_str(&format!("pub mod {} {{\n", module.to_lowercase()));
            for e in self.module_entries(module) {
                if !e.description.is_empty() {
                    out.push_str(&format!("    /// {}\n", e.description));
                }
                let params: Vec<String> = e
                    .params
                    .iter()
                    .map(|p| format!("{}: {}", p.name, p.param_type.as_rune_str()))
                    .collect();
                let ret = e
                    .return_type
                    .map(|t| t.as_rune_str())
                    .unwrap_or("()");
                out.push_str(&format!(
                    "    pub fn {}({}) -> {} {{ __native_invoke(\"{}.{}\", &[{}]) }}\n",
                    e.name,
                    params.join(", "),
                    ret,
                    module,
                    e.name,
                    e.params.iter().map(|p| p.name).collect::<Vec<_>>().join(", "),
                ));
            }
            out.push_str("}\n\n");
        }
        out
    }
}

