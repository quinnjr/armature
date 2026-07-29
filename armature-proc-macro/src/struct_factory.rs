//! Shared codegen for controller-struct factory metadata.
//!
//! `#[guard(...)]` and `#[middleware(...)]` applied to a controller *struct*
//! both emit the same shape: one numbered `__<kind>_factory_{i}` associated
//! function per attribute argument, plus a `__get_<kind>_factories()` accessor
//! listing them. Only the trait being boxed and the per-item construction
//! expression differ, so both attributes share the generator below.

use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{Ident, ItemStruct};

/// Emit the inherent impl carrying a controller's factory metadata.
///
/// * `kind` names the metadata (`"guard"` / `"middleware"`) and is spliced into
///   the generated identifiers.
/// * `trait_path` is the boxed trait object type (e.g.
///   `armature_core::guard::Guard`).
/// * `constructions` holds one expression per attribute argument, each
///   evaluating to `Box<dyn #trait_path>`.
pub(crate) fn factory_metadata_impl(
    item_struct: &ItemStruct,
    kind: &str,
    trait_path: &TokenStream2,
    constructions: &[TokenStream2],
) -> TokenStream2 {
    let struct_name = &item_struct.ident;
    let (impl_generics, ty_generics, where_clause) = item_struct.generics.split_for_impl();

    let factory_names: Vec<Ident> = (0..constructions.len())
        .map(|i| Ident::new(&format!("__{kind}_factory_{i}"), Span::call_site()))
        .collect();

    let factory_fns = factory_names.iter().zip(constructions).map(|(name, ctor)| {
        quote! {
            fn #name() -> Box<dyn #trait_path> {
                #ctor
            }
        }
    });

    let accessor_name = Ident::new(&format!("__get_{kind}_factories"), Span::call_site());
    let accessor_doc = format!(
        " {kind} factories for this controller.\n\n\
         Consumed by the `#[module]` route registrar, which calls each factory\n\
         once at route-registration time and applies the resulting {kind}s to\n\
         every route declared on this controller."
    );

    quote! {
        impl #impl_generics #struct_name #ty_generics #where_clause {
            #[doc = #accessor_doc]
            pub fn #accessor_name() -> Vec<fn() -> Box<dyn #trait_path>> {
                vec![#(Self::#factory_names),*]
            }

            #(#factory_fns)*
        }
    }
}
