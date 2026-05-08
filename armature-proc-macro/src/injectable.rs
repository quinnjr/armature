use proc_macro::TokenStream;
use quote::quote;
use syn::{Fields, GenericArgument, ItemStruct, PathArguments, Type, parse_macro_input};

/// If `ty` is `Arc<T>` (or `std::sync::Arc<T>`), return the inner `T`.
/// `container.resolve::<T>()` returns `Arc<T>`, so for Arc-typed fields
/// we want to resolve the inner type — resolving `Arc<T>` directly
/// produces `Arc<Arc<T>>`.
fn arc_inner(ty: &Type) -> Option<Type> {
    let Type::Path(tp) = ty else {
        return None;
    };
    let last = tp.path.segments.last()?;
    if last.ident != "Arc" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    let Some(GenericArgument::Type(inner)) = args.args.first() else {
        return None;
    };
    Some(inner.clone())
}

pub fn injectable_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemStruct);
    let struct_name = &input.ident;

    let field_resolvers = match &input.fields {
        Fields::Named(fields) => fields
            .named
            .iter()
            .map(|f| {
                let field_name = &f.ident;
                let field_type = &f.ty;
                if let Some(inner) = arc_inner(field_type) {
                    // Arc<T> field: resolve T to get Arc<T> directly.
                    quote! {
                        #field_name: container.resolve::<#inner>()
                            .map_err(|e| armature_core::Error::ProviderNotFound(
                                format!("Failed to resolve {} for {}: {}", stringify!(#field_type), stringify!(#struct_name), e)
                            ))?,
                    }
                } else {
                    // Plain T: container.resolve gives Arc<T>; deref + clone
                    // to produce a T value for the field.
                    quote! {
                        #field_name: (*container.resolve::<#field_type>()
                            .map_err(|e| armature_core::Error::ProviderNotFound(
                                format!("Failed to resolve {} for {}: {}", stringify!(#field_type), stringify!(#struct_name), e)
                            ))?).clone(),
                    }
                }
            })
            .collect::<Vec<_>>(),
        _ => vec![],
    };

    let expanded = quote! {
        #input

        impl #struct_name {
            pub fn from_container(container: &armature_core::Container) -> Result<Self, armature_core::Error> {
                Ok(Self {
                    #(#field_resolvers)*
                })
            }
        }
    };

    TokenStream::from(expanded)
}
