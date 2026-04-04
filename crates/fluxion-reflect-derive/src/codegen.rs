// ============================================================
// fluxion-reflect-derive — code generation
//
// Given a parsed struct, emits:
//   1. `static FOO_FIELDS: LazyLock<Vec<FieldDescriptor>>`
//   2. `impl Reflect for Foo { … }`
// ============================================================

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, Type};

use crate::attrs::{auto_display_name, FieldAttrs, StructAttrs};

/// Which `ReflectFieldType` variant + how to get/set the value.
#[derive(Debug, Clone, PartialEq)]
enum ReflectKind {
    F32,
    Vec3,    // glam::Vec3
    Quat,    // glam::Quat
    Color3,  // [f32; 3]
    Color4,  // [f32; 4]
    Bool,
    U32,
    U8,
    USize,
    Str,
    OptionStr,
    Enum,    // serde round-trip
}

impl ReflectKind {
    /// `ReflectFieldType::Xxx` token
    fn field_type_token(&self, core: &TokenStream) -> TokenStream {
        match self {
            Self::F32      => quote!(#core::reflect::ReflectFieldType::F32),
            Self::Vec3     => quote!(#core::reflect::ReflectFieldType::Vec3),
            Self::Quat     => quote!(#core::reflect::ReflectFieldType::Quat),
            Self::Color3   => quote!(#core::reflect::ReflectFieldType::Color3),
            Self::Color4   => quote!(#core::reflect::ReflectFieldType::Color4),
            Self::Bool     => quote!(#core::reflect::ReflectFieldType::Bool),
            Self::U32      => quote!(#core::reflect::ReflectFieldType::U32),
            Self::U8       => quote!(#core::reflect::ReflectFieldType::U8),
            Self::USize    => quote!(#core::reflect::ReflectFieldType::USize),
            Self::Str      => quote!(#core::reflect::ReflectFieldType::Str),
            Self::OptionStr => quote!(#core::reflect::ReflectFieldType::OptionStr),
            Self::Enum     => quote!(#core::reflect::ReflectFieldType::Enum),
        }
    }

    /// Expression to get the field value: `Some(ReflectValue::Xxx(...))`
    fn get_expr(&self, field_expr: &TokenStream, core: &TokenStream) -> TokenStream {
        match self {
            Self::F32      => quote!(Some(#core::reflect::ReflectValue::F32(#field_expr))),
            Self::Vec3     => quote!(Some(#core::reflect::ReflectValue::Vec3(#field_expr.to_array()))),
            Self::Quat     => quote!(Some(#core::reflect::ReflectValue::Quat(#field_expr.to_array()))),
            Self::Color3   => quote!(Some(#core::reflect::ReflectValue::Color3(#field_expr))),
            Self::Color4   => quote!(Some(#core::reflect::ReflectValue::Color4(#field_expr))),
            Self::Bool     => quote!(Some(#core::reflect::ReflectValue::Bool(#field_expr))),
            Self::U32      => quote!(Some(#core::reflect::ReflectValue::U32(#field_expr))),
            Self::U8       => quote!(Some(#core::reflect::ReflectValue::U8(#field_expr))),
            Self::USize    => quote!(Some(#core::reflect::ReflectValue::USize(#field_expr))),
            Self::Str      => quote!(Some(#core::reflect::ReflectValue::Str(#field_expr.clone()))),
            Self::OptionStr => quote!(Some(#core::reflect::ReflectValue::OptionStr(#field_expr.clone()))),
            Self::Enum     => quote!(Some(#core::reflect::ReflectValue::Enum(
                ::serde_json::to_string(&#field_expr)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_string()
            ))),
        }
    }

    /// Pattern + body for a `set_field` match arm.
    fn set_arm(
        &self,
        name_str: &str,
        field_ident: &TokenStream,
        core: &TokenStream,
    ) -> TokenStream {
        match self {
            Self::F32  => quote! {
                (#name_str, #core::reflect::ReflectValue::F32(v)) => { #field_ident = v; }
            },
            Self::Vec3 => quote! {
                (#name_str, #core::reflect::ReflectValue::Vec3(v)) => {
                    #field_ident = ::glam::Vec3::from(v);
                }
            },
            Self::Quat => quote! {
                (#name_str, #core::reflect::ReflectValue::Quat(v)) => {
                    #field_ident = ::glam::Quat::from_array(v).normalize();
                }
            },
            Self::Color3 => quote! {
                (#name_str, #core::reflect::ReflectValue::Color3(v)) => { #field_ident = v; }
            },
            Self::Color4 => quote! {
                (#name_str, #core::reflect::ReflectValue::Color4(v)) => { #field_ident = v; }
            },
            Self::Bool => quote! {
                (#name_str, #core::reflect::ReflectValue::Bool(v)) => { #field_ident = v; }
            },
            Self::U32 => quote! {
                (#name_str, #core::reflect::ReflectValue::U32(v)) => { #field_ident = v; }
            },
            Self::U8 => quote! {
                (#name_str, #core::reflect::ReflectValue::U8(v)) => { #field_ident = v; }
            },
            Self::USize => quote! {
                (#name_str, #core::reflect::ReflectValue::USize(v)) => { #field_ident = v; }
            },
            Self::Str => quote! {
                (#name_str, #core::reflect::ReflectValue::Str(v)) => { #field_ident = v; }
            },
            Self::OptionStr => quote! {
                (#name_str, #core::reflect::ReflectValue::OptionStr(v)) => { #field_ident = v; }
            },
            Self::Enum => quote! {
                (#name_str, #core::reflect::ReflectValue::Enum(s)) => {
                    #field_ident = ::serde_json::from_str(&::std::format!("\"{}\"", s))
                        .map_err(|_| ::std::format!(
                            "Unknown variant '{}' for field '{}'", s, #name_str
                        ))?;
                }
            },
        }
    }
}

/// Detect the reflect kind from a Rust type + field attributes.
fn detect_kind(ty: &Type, attrs: &FieldAttrs) -> ReflectKind {
    match ty {
        Type::Path(tp) => {
            let segs: Vec<_> = tp.path.segments.iter().collect();
            let last = segs.last().map(|s| s.ident.to_string());
            match last.as_deref() {
                Some("f32")    => ReflectKind::F32,
                Some("bool")   => ReflectKind::Bool,
                Some("u32")    => ReflectKind::U32,
                Some("u8")     => ReflectKind::U8,
                Some("usize")  => ReflectKind::USize,
                Some("String") => ReflectKind::Str,
                Some("Vec3")   => ReflectKind::Vec3,
                Some("Quat")   => ReflectKind::Quat,
                Some("Option") => ReflectKind::OptionStr,
                _ => ReflectKind::Enum,
            }
        }
        Type::Array(arr) => {
            // [f32; 3] → Color3, [f32; 4] → Color4 (default for arrays)
            if let syn::Expr::Lit(el) = &arr.len {
                if let syn::Lit::Int(n) = &el.lit {
                    let n: usize = n.base10_parse().unwrap_or(0);
                    if attrs.color || true {
                        // arrays are always treated as Color by default
                        return match n {
                            3 => ReflectKind::Color3,
                            4 => ReflectKind::Color4,
                            _ => ReflectKind::Enum,
                        };
                    }
                }
            }
            ReflectKind::Enum
        }
        _ => ReflectKind::Enum,
    }
}

/// Returns the path to `fluxion_core` (or `crate` if inside fluxion-core itself).
fn core_path() -> TokenStream {
    use proc_macro_crate::{crate_name, FoundCrate};
    match crate_name("fluxion-core") {
        Ok(FoundCrate::Itself) => quote!(crate),
        Ok(FoundCrate::Name(n)) => {
            let ident = syn::Ident::new(&n, Span::call_site());
            quote!(#ident)
        }
        Err(_) => quote!(fluxion_core),
    }
}

pub fn derive_reflect_impl(input: DeriveInput) -> TokenStream {
    let core = core_path();

    let struct_name = &input.ident;
    let struct_attrs = StructAttrs::from_attrs(&input.attrs);
    let type_name_lit = struct_attrs
        .type_name
        .unwrap_or_else(|| struct_name.to_string());

    // Only support named structs
    let named_fields = match &input.data {
        Data::Struct(ds) => match &ds.fields {
            Fields::Named(f) => &f.named,
            _ => {
                return syn::Error::new_spanned(struct_name, "Reflect can only be derived for structs with named fields")
                    .to_compile_error();
            }
        },
        _ => {
            return syn::Error::new_spanned(struct_name, "Reflect can only be derived for structs")
                .to_compile_error();
        }
    };

    // Per-field metadata
    struct FieldInfo {
        ident_str: String,
        display:   String,
        kind:      ReflectKind,
        attrs:     FieldAttrs,
    }

    let mut fields_info: Vec<FieldInfo> = Vec::new();
    for field in named_fields {
        let ident = field.ident.as_ref().unwrap();
        let ident_str = ident.to_string();
        let attrs = FieldAttrs::from_attrs(&field.attrs);
        if attrs.skip {
            continue;
        }
        let display = attrs
            .display_name
            .clone()
            .unwrap_or_else(|| auto_display_name(&ident_str));
        let kind = detect_kind(&field.ty, &attrs);
        fields_info.push(FieldInfo { ident_str, display, kind, attrs });
    }

    // ── static FIELDS array ───────────────────────────────────────────────────
    let static_ident = format_ident!("{}_REFLECT_FIELDS", struct_name.to_string().to_uppercase());

    let field_descriptor_items: Vec<TokenStream> = fields_info.iter().map(|fi| {
        let name_lit  = &fi.ident_str;
        let disp_lit  = &fi.display;
        let ft        = fi.kind.field_type_token(&core);
        let read_only = fi.attrs.read_only;

        let range_tokens = match (fi.attrs.range_min, fi.attrs.range_max, fi.attrs.range_step) {
            (Some(mn), Some(mx), _) => quote! {
                .with_range(#core::reflect::RangeHint::min_max(#mn, #mx))
            },
            (_, _, Some(st)) => quote! {
                .with_range(#core::reflect::RangeHint::step(#st))
            },
            _ => quote!(),
        };

        if read_only {
            quote! {
                #core::reflect::FieldDescriptor::read_only(#name_lit, #disp_lit, #ft) #range_tokens
            }
        } else {
            quote! {
                #core::reflect::FieldDescriptor::new(#name_lit, #disp_lit, #ft) #range_tokens
            }
        }
    }).collect();

    // ── get_field match arms ──────────────────────────────────────────────────
    let get_arms: Vec<TokenStream> = fields_info.iter().map(|fi| {
        let name_lit  = &fi.ident_str;
        let ident     = format_ident!("{}", fi.ident_str);
        let field_ref = quote!(self.#ident);
        let expr      = fi.kind.get_expr(&field_ref, &core);
        quote!(#name_lit => #expr,)
    }).collect();

    // ── set_field match arms ──────────────────────────────────────────────────
    let set_arms: Vec<TokenStream> = fields_info.iter().filter(|fi| !fi.attrs.read_only).map(|fi| {
        let name_lit  = &fi.ident_str;
        let ident     = format_ident!("{}", fi.ident_str);
        let self_field = quote!(self.#ident);
        fi.kind.set_arm(name_lit, &self_field, &core)
    }).collect();

    // ── final impl ────────────────────────────────────────────────────────────
    quote! {
        static #static_ident: ::std::sync::OnceLock<::std::vec::Vec<#core::reflect::FieldDescriptor>> =
            ::std::sync::OnceLock::new();

        impl #core::reflect::Reflect for #struct_name {
            fn reflect_type_name(&self) -> &'static str { #type_name_lit }

            fn fields(&self) -> &'static [#core::reflect::FieldDescriptor] {
                #static_ident.get_or_init(|| vec![ #(#field_descriptor_items),* ])
            }

            fn get_field(&self, name: &str) -> ::std::option::Option<#core::reflect::ReflectValue> {
                match name {
                    #(#get_arms)*
                    _ => None,
                }
            }

            fn set_field(
                &mut self,
                name: &str,
                value: #core::reflect::ReflectValue,
            ) -> ::std::result::Result<(), ::std::string::String> {
                match (name, value) {
                    #(#set_arms)*
                    (n, _) => return Err(::std::format!(
                        "Unknown or type-mismatched field '{}' on {}", n, #type_name_lit
                    )),
                }
                Ok(())
            }
        }
    }
}
