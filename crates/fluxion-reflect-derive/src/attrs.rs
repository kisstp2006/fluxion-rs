// ============================================================
// fluxion-reflect-derive — attribute parsing
//
// Parses the #[reflect(...)] helper attributes on struct fields.
// ============================================================

use syn::{Attribute, Lit};

/// Parsed per-field `#[reflect(...)]` attributes.
#[derive(Default, Debug)]
pub struct FieldAttrs {
    /// Skip this field entirely (not exposed to reflection).
    pub skip: bool,
    /// Field is shown in the inspector but cannot be edited.
    pub read_only: bool,
    /// Override the display name (default: auto-generated from field name).
    pub display_name: Option<String>,
    /// Treat `[f32; 3]` / `[f32; 4]` as a color (Color3 / Color4) instead of Vec3/Quat.
    pub color: bool,
    /// Range hint for numeric fields: `#[reflect(range(min=0.0, max=1.0))]`.
    pub range_min: Option<f32>,
    pub range_max: Option<f32>,
    pub range_step: Option<f32>,
    /// Inspector category / group header.
    pub category: Option<String>,
    /// Known enum variant names (for combo-box). Auto-detected if empty.
    pub variants: Vec<String>,
}

impl FieldAttrs {
    /// Parse every `#[reflect(...)]` attribute on a field.
    pub fn from_attrs(attrs: &[Attribute]) -> Self {
        let mut out = FieldAttrs::default();
        for attr in attrs {
            if !attr.path().is_ident("reflect") {
                continue;
            }
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("skip") {
                    out.skip = true;
                } else if meta.path.is_ident("read_only") {
                    out.read_only = true;
                } else if meta.path.is_ident("color") {
                    out.color = true;
                } else if meta.path.is_ident("display_name") {
                    let value = meta.value()?;
                    let s: syn::LitStr = value.parse()?;
                    out.display_name = Some(s.value());
                } else if meta.path.is_ident("category") {
                    let value = meta.value()?;
                    let s: syn::LitStr = value.parse()?;
                    out.category = Some(s.value());
                } else if meta.path.is_ident("range") {
                    meta.parse_nested_meta(|range| {
                        let value = range.value()?;
                        let lit: Lit = value.parse()?;
                        let f = lit_to_f32(&lit);
                        if range.path.is_ident("min") {
                            out.range_min = Some(f);
                        } else if range.path.is_ident("max") {
                            out.range_max = Some(f);
                        } else if range.path.is_ident("step") {
                            out.range_step = Some(f);
                        }
                        Ok(())
                    })?;
                } else if meta.path.is_ident("variants") {
                    meta.parse_nested_meta(|v| {
                        // `#[reflect(variants("A", "B", "C"))]`
                        if let Ok(s) = v.input.parse::<syn::LitStr>() {
                            out.variants.push(s.value());
                        }
                        Ok(())
                    })?;
                }
                Ok(())
            });
        }
        out
    }
}

/// Parsed per-struct `#[reflect(...)]` attributes.
#[derive(Default, Debug)]
pub struct StructAttrs {
    /// Override the reflected type name (default: struct ident).
    pub type_name: Option<String>,
}

impl StructAttrs {
    pub fn from_attrs(attrs: &[Attribute]) -> Self {
        let mut out = StructAttrs::default();
        for attr in attrs {
            if !attr.path().is_ident("reflect") {
                continue;
            }
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("type_name") {
                    let value = meta.value()?;
                    let s: syn::LitStr = value.parse()?;
                    out.type_name = Some(s.value());
                }
                Ok(())
            });
        }
        out
    }
}

fn lit_to_f32(lit: &Lit) -> f32 {
    match lit {
        Lit::Float(f) => f.base10_parse().unwrap_or(0.0),
        Lit::Int(i) => i.base10_parse::<i64>().unwrap_or(0) as f32,
        _ => 0.0,
    }
}

/// Auto-generate a display name from a snake_case identifier.
/// "field_of_view" → "Field Of View"
pub fn auto_display_name(ident: &str) -> String {
    ident
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
