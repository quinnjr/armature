use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Fields, ItemStruct};

pub fn injectable_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemStruct);
    let struct_name = &input.ident;

    let field_resolvers = match &input.fields {
        Fields::Named(fields) => {
            fields.named.iter().map(|f| {
                let field_name = &f.ident;
                let field_type = &f.ty;
                let type_str = quote!(#field_type).to_string();
                if type_str.starts_with("Arc <") || type_str.starts_with("std :: sync :: Arc <") {
                    quote! {
                        #field_name: container.resolve::<#field_type>()
                            .map_err(|e| armature_core::Error::ProviderNotFound(
                                format!("Failed to resolve {} for {}: {}", stringify!(#field_type), stringify!(#struct_name), e)
                            ))?,
                    }
                } else {
                    quote! {
                        #field_name: (*container.resolve::<#field_type>()
                            .map_err(|e| armature_core::Error::ProviderNotFound(
                                format!("Failed to resolve {} for {}: {}", stringify!(#field_type), stringify!(#struct_name), e)
                            ))?).clone(),
                    }
                }
            }).collect::<Vec<_>>()
        }
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
