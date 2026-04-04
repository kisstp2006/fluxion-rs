// ============================================================
// fluxion-reflect-derive
//
// Procedural macro crate that derives the `Reflect` trait for
// structs, eliminating the hand-written boilerplate in reflect_impls.rs.
//
// Usage:
//   #[derive(Reflect)]
//   pub struct MyComponent {
//       #[reflect(display_name = "Speed", range(min = 0.0, max = 100.0))]
//       pub speed: f32,
//
//       #[reflect(skip)]          // not exposed
//       internal: u32,
//
//       #[reflect(read_only)]
//       pub computed: f32,
//
//       #[reflect(color)]         // [f32;3] treated as Color3
//       pub tint: [f32; 3],
//   }
// ============================================================

mod attrs;
mod codegen;

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

/// Derive the `fluxion_core::reflect::Reflect` trait for a named struct.
///
/// Generates:
/// - `static MY_STRUCT_REFLECT_FIELDS: OnceLock<Vec<FieldDescriptor>>`
/// - `impl Reflect for MyStruct { reflect_type_name, fields, get_field, set_field }`
///
/// `to_serialized_data` is inherited from the default impl (iterates fields).
/// Override it manually if the struct needs custom JSON layout (e.g. Transform Euler).
#[proc_macro_derive(Reflect, attributes(reflect))]
pub fn derive_reflect(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    codegen::derive_reflect_impl(input).into()
}
