use proc_macro::TokenStream;
use quote::quote;
use syn::{Fields, ItemStruct, LitStr, parse_macro_input};

use crate::route_validation::validate_controller_path;

pub fn controller_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemStruct);
    let struct_name = &input.ident;
    let base_path = parse_macro_input!(attr as LitStr);
    let base_path_value = base_path.value();

    // Validate controller base path at compile time
    if let Err(e) = validate_controller_path(&base_path_value, base_path.span()) {
        return e.to_compile_error().into();
    }

    // Extract field types for dependency injection
    let field_types: Vec<_> = match &input.fields {
        Fields::Named(fields) => fields.named.iter().map(|f| &f.ty).collect(),
        _ => vec![],
    };

    let field_names: Vec<_> = match &input.fields {
        Fields::Named(fields) => fields.named.iter().map(|f| &f.ident).collect(),
        _ => vec![],
    };

    // Generate constructor that resolves dependencies
    let constructor = if !field_types.is_empty() {
        quote! {
            pub fn new_with_di(container: &armature_core::Container) -> Result<Self, armature_core::Error> {
                Ok(Self {
                    #(#field_names: (*container.resolve::<#field_types>()?).clone()),*
                })
            }
        }
    } else {
        quote! {
            pub fn new_with_di(_container: &armature_core::Container) -> Result<Self, armature_core::Error> {
                Ok(Self::default())
            }
        }
    };

    // Per-controller trait supplying a default (empty) `__collect_routes`.
    // The `#[routes]` macro emits an *inherent* `__collect_routes` on the same
    // type; inherent methods shadow trait methods, so `Self::__collect_routes()`
    // resolves to the real route list when `#[routes]` is present and to this
    // empty default otherwise. This keeps controllers without a `#[routes]` impl
    // compiling while removing the silent `vec![]` for those that have one.
    let routes_trait = syn::Ident::new(
        &format!("__ArmatureCollectRoutes{struct_name}"),
        struct_name.span(),
    );

    let expanded = quote! {
        #input

        // Provider trait is now a blanket impl for Send + Sync + 'static types,
        // so no explicit impl is needed.

        #[doc(hidden)]
        trait #routes_trait {
            fn __collect_routes() -> Vec<armature_core::RouteDefinition> {
                ::std::vec::Vec::new()
            }
        }

        impl #routes_trait for #struct_name {}

        #[async_trait::async_trait]
        impl armature_core::Controller for #struct_name {
            fn base_path(&self) -> &'static str {
                #base_path_value
            }

            /// Collect all routes defined on this controller.
            ///
            /// Populated from the `#[routes]`-generated metadata (controller-relative
            /// paths, matching the list registered by the module route registrar).
            /// Returns an empty list when the controller has no `#[routes]` impl.
            fn routes(&self) -> Vec<armature_core::RouteDefinition> {
                Self::__collect_routes()
            }
        }

        impl #struct_name {
            pub const BASE_PATH: &'static str = #base_path_value;

            #constructor
        }
    };

    TokenStream::from(expanded)
}
