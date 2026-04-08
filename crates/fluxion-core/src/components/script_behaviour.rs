// ============================================================
// fluxion-core — ScriptBundle component
//
// Holds N per-entity Rune (.rn) gameplay scripts.
// Each ScriptEntry maps to one RuneBehaviour instance
// and appears as a separate named panel in the editor
// inspector (Unity MonoBehaviour style).
//
// Lifecycle per entry:
//   start() / update(dt) / fixed_update(dt) / on_destroy()
// ============================================================

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::ecs::component::Component;

/// Inspector annotation metadata parsed from `// [inspector: ...]` comments.
///
/// All fields are optional — missing means "use default inspector behaviour".
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScriptFieldMeta {
    /// Widget hint: `"slider"`, `"drag"`, `"toggle"`, `"color"`, `"entity_ref"`,
    /// `"mesh"`, `"material"`, `"audio"`, `"texture"`.  Empty = auto-detect from value type.
    pub hint:    String,
    /// Override the display label shown in the inspector.  Empty = use field name.
    pub label:   String,
    /// Minimum numeric value (for sliders / drag floats).
    pub min:     f64,
    /// Maximum numeric value (for sliders / drag floats).
    pub max:     f64,
    /// Tooltip shown on hover.
    pub tooltip: String,
    /// If true, field is hidden from the inspector (still readable by scripts).
    pub hidden:  bool,
    /// If true, field is shown as read-only in the inspector.
    pub read_only: bool,
}

/// One serializable field extracted from a script struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptField {
    pub name:  String,
    /// JSON-encoded value: null / bool / number / string.
    pub value: JsonValue,
    /// Optional inspector annotation metadata (from `// [inspector: ...]`).
    #[serde(default)]
    pub meta:  ScriptFieldMeta,
}

/// One attached Rune gameplay script entry.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScriptEntry {
    /// Display name shown in the inspector (e.g. `"PlayerController"`).
    pub name: String,
    /// Path to the `.rn` file, relative to `assets/`.
    pub path: String,
    /// Whether this script should tick.
    pub enabled: bool,
    /// Fields extracted from the matching struct definition in the script source.
    #[serde(default)]
    pub fields: Vec<ScriptField>,
}

/// Holds all Rune gameplay scripts attached to an entity.
///
/// Appears in the inspector as one entry per `ScriptEntry`,
/// each rendered as a separate named component panel.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScriptBundle {
    pub scripts: Vec<ScriptEntry>,
}

impl ScriptBundle {
    /// Push a new script entry, deriving the display name from the file stem.
    /// Fields are empty — caller should populate via `scan_struct_fields` if source is available.
    pub fn attach(&mut self, path: impl Into<String>) {
        let path = path.into();
        let name = derive_script_name(&path);
        self.scripts.push(ScriptEntry { name, path, enabled: true, fields: Vec::new() });
    }

    /// Push a new script entry and immediately parse struct fields from source.
    pub fn attach_with_source(&mut self, path: impl Into<String>, source: &str) {
        let path = path.into();
        let name = derive_script_name(&path);
        let fields = scan_struct_fields(source, &name);
        self.scripts.push(ScriptEntry { name, path, enabled: true, fields });
    }

    /// Remove the first entry whose name matches.
    pub fn remove_by_name(&mut self, name: &str) {
        self.scripts.retain(|e| e.name != name);
    }

    /// Remove the first entry whose path matches.
    pub fn remove_by_path(&mut self, path: &str) {
        self.scripts.retain(|e| e.path != path);
    }
}

impl Component for ScriptBundle {}

/// Parse a Rune source file for `struct Name { field, … }` where `Name == struct_name`.
///
/// Also scans `fn new(` body for `field: literal` assignments to infer typed defaults.
/// Falls back to `JsonValue::Null` for fields with no detectable default.
///
/// # Inspector annotations
/// Place a `// [inspector: ...]` comment on the line **directly above** a field to
/// attach metadata.  Supported key=value tokens (comma-separated):
/// - `slider`, `drag`, `toggle`, `color`, `entity_ref`, `mesh`, `material`, `audio`,
///   `texture` — sets the widget hint.
/// - `min=<f64>`, `max=<f64>` — clamp range for numeric widgets.
/// - `label="..."` — override the display label.
/// - `tooltip="..."` — hover tooltip.
/// - `hide` — hide from inspector.
/// - `readonly` — display as read-only.
///
/// Example:
/// ```rune
/// struct Player {
///     // [inspector: slider, min=0, max=100, tooltip="Player health"]
///     health,
///     // [inspector: label="Move Speed", min=0, max=20]
///     speed,
///     // [inspector: hide]
///     _internal_timer,
/// }
/// ```
pub fn scan_struct_fields(source: &str, struct_name: &str) -> Vec<ScriptField> {
    let mut fields: Vec<ScriptField> = Vec::new();

    // ── 1. Find the struct body ───────────────────────────────────────────────
    let struct_header = format!("struct {} {{", struct_name);
    let Some(body_start) = source.find(&struct_header) else { return fields; };
    let after_header = &source[body_start + struct_header.len()..];
    let Some(body_end) = after_header.find('}') else { return fields; };
    let body = &after_header[..body_end];

    // Collect lines and their preceding annotation comment (if any).
    let lines: Vec<&str> = body.lines().collect();
    let mut pending_meta: Option<ScriptFieldMeta> = None;

    for line in &lines {
        let trimmed = line.trim();

        // Check for [inspector: ...] annotation comment.
        if let Some(rest) = trimmed.strip_prefix("// [inspector:") {
            let rest = rest.trim_end_matches(']').trim();
            pending_meta = Some(parse_inspector_annotation(rest));
            continue;
        }

        // Skip other comments and blank lines but preserve pending_meta across blank lines.
        if trimmed.is_empty() { continue; }
        if trimmed.starts_with('/') {
            // Non-annotation comment — reset pending meta.
            pending_meta = None;
            continue;
        }

        // Field lines are bare identifiers (no `:` type annotations in Rune structs).
        let field_name = trimmed.trim_end_matches(',');
        if field_name.chars().all(|c| c.is_alphanumeric() || c == '_') && !field_name.is_empty() {
            let meta = pending_meta.take().unwrap_or_default();
            fields.push(ScriptField { name: field_name.to_string(), value: JsonValue::Null, meta });
        } else {
            pending_meta = None;
        }
    }

    if fields.is_empty() { return fields; }

    // ── 2. Scan fn new() body for typed defaults ──────────────────────────────
    let new_header = format!("{}  {{", struct_name);
    if let Some(fn_new_pos) = source.find("fn new(") {
        let fn_body = &source[fn_new_pos..];
        let lit_header = format!("{} {{", struct_name);
        let lit_pos_opt = fn_body.find(&lit_header)
            .or_else(|| fn_body.find(&new_header));
        if let Some(lit_pos) = lit_pos_opt {
            let lit_after = &fn_body[lit_pos + lit_header.len()..];
            let Some(lit_end) = lit_after.find('}') else { return fields; };
            let lit_body = &lit_after[..lit_end];
            for line in lit_body.lines() {
                let trimmed = line.trim().trim_end_matches(',');
                if let Some(colon) = trimmed.find(':') {
                    let field_name = trimmed[..colon].trim();
                    let raw_val   = trimmed[colon + 1..].trim();
                    if let Some(f) = fields.iter_mut().find(|f| f.name == field_name) {
                        f.value = infer_json_value(raw_val);
                    }
                }
            }
        }
    }

    fields
}

/// Parse a `// [inspector: ...]` annotation body into `ScriptFieldMeta`.
/// `src` is the text **between** `// [inspector:` and `]` (already trimmed).
fn parse_inspector_annotation(src: &str) -> ScriptFieldMeta {
    let mut meta = ScriptFieldMeta::default();

    for token in src.split(',') {
        let token = token.trim();
        if token.is_empty() { continue; }

        // Key=value tokens.
        if let Some(eq) = token.find('=') {
            let key = token[..eq].trim();
            let val = token[eq + 1..].trim().trim_matches('"');
            match key {
                "min"     => { meta.min = val.parse().unwrap_or(0.0); }
                "max"     => { meta.max = val.parse().unwrap_or(0.0); }
                "label"   => { meta.label   = val.to_string(); }
                "tooltip" => { meta.tooltip = val.to_string(); }
                _         => {}
            }
        } else {
            // Bare keyword tokens.
            match token {
                "slider" | "drag" | "toggle" | "color"
                | "entity_ref" | "mesh" | "material"
                | "audio" | "texture" | "scene" => {
                    meta.hint = token.to_string();
                }
                "hide"     => { meta.hidden    = true; }
                "readonly" => { meta.read_only = true; }
                _          => {}
            }
        }
    }

    meta
}

/// Heuristically convert a Rune literal token to a `serde_json::Value`.
fn infer_json_value(raw: &str) -> JsonValue {
    if raw == "true"  { return JsonValue::Bool(true);  }
    if raw == "false" { return JsonValue::Bool(false); }
    if (raw.starts_with('"') && raw.ends_with('"'))
        || (raw.starts_with('\'') && raw.ends_with('\'')) {
        return JsonValue::String(raw[1..raw.len()-1].to_string());
    }
    if let Ok(n) = raw.parse::<i64>() {
        return JsonValue::Number(serde_json::Number::from(n));
    }
    if let Ok(f) = raw.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return JsonValue::Number(n);
        }
    }
    JsonValue::Null
}

/// Convert a file path stem to PascalCase component name.
/// `"scripts/player_controller.rn"` → `"PlayerController"`
pub fn derive_script_name(path: &str) -> String {
    let stem = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Script");
    stem.split(['_', '-', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect()
}
