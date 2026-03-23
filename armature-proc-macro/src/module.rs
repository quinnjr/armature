use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Ident, ItemStruct, Token, Type,
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
};

struct ModuleArgs {
    providers: Vec<Type>,
    controllers: Vec<Type>,
    imports: Vec<Type>,
    exports: Vec<Type>,
}

impl Parse for ModuleArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut providers = Vec::new();
        let mut controllers = Vec::new();
        let mut imports = Vec::new();
        let mut exports = Vec::new();

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![:]>()?;

            let content;
            syn::bracketed!(content in input);
            let types: Punctuated<Type, Token![,]> =
                content.parse_terminated(Type::parse, Token![,])?;

            match key.to_string().as_str() {
                "providers" => providers = types.into_iter().collect(),
                "controllers" => controllers = types.into_iter().collect(),
                "imports" => imports = types.into_iter().collect(),
                "exports" => exports = types.into_iter().collect(),
                _ => {}
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(ModuleArgs {
            providers,
            controllers,
            imports,
            exports,
        })
    }
}

pub fn module_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemStruct);
    let struct_name = &input.ident;

    let args = parse_macro_input!(attr as ModuleArgs);

    let providers = &args.providers;
    let controllers = &args.controllers;
    let imports = &args.imports;
    let exports = &args.exports;

    let provider_registrations = providers.iter().map(|ty| {
        quote! {
            armature_core::ProviderRegistration {
                type_id: std::any::TypeId::of::<#ty>(),
                type_name: std::any::type_name::<#ty>(),
                register_fn: |container| {
                    if container.has::<#ty>() {
                        return;
                    }
                    match #ty::from_container(container) {
                        Ok(instance) => container.register(instance),
                        Err(e) => {
                            tracing::debug!(
                                "Could not resolve {} from container ({}), this provider must be registered manually",
                                std::any::type_name::<#ty>(),
                                e
                            );
                        }
                    }
                },
            }
        }
    });

    let controller_registrations = controllers.iter().map(|ty| {
        quote! {
            armature_core::ControllerRegistration {
                type_id: std::any::TypeId::of::<#ty>(),
                type_name: std::any::type_name::<#ty>(),
                base_path: #ty::BASE_PATH,
                factory: |container| {
                    let instance = #ty::new_with_di(container)?;
                    Ok(Box::new(instance) as Box<dyn std::any::Any + Send + Sync>)
                },
                route_registrar: |_container, router, controller_any| {
                    // Get base path from constant
                    let base_path = #ty::BASE_PATH;

                    // Downcast the controller
                    let controller = controller_any.downcast::<#ty>()
                        .map_err(|_| armature_core::Error::Internal("Failed to downcast controller".to_string()))?;

                    // Wrap in Arc for shared ownership
                    let controller = std::sync::Arc::new(*controller);

                    // Get route handlers from the controller
                    let route_handlers = #ty::__route_handlers(controller);

                    // Register each route
                    for (method, path, handler) in route_handlers {
                        let full_path = if path.is_empty() {
                            base_path.to_string()
                        } else if base_path.ends_with('/') || path.starts_with('/') {
                            format!("{}{}", base_path.trim_end_matches('/'), path)
                        } else {
                            format!("{}/{}", base_path, path)
                        };

                        let route = armature_core::routing::Route {
                            method: armature_core::HttpMethod::from_str(method)
                                .unwrap_or(armature_core::HttpMethod::GET),
                            path: full_path,
                            handler: armature_core::handler::from_legacy_handler(handler.clone()),
                            constraints: None,
                        };
                        router.add_route(route);
                    }

                    Ok(())
                },
            }
        }
    });

    let import_instances = imports.iter().map(|ty| {
        quote! {
            Box::new(#ty::default()) as Box<dyn armature_core::Module>
        }
    });

    let export_ids = exports.iter().map(|ty| {
        quote! {
            std::any::TypeId::of::<#ty>()
        }
    });

    let expanded = quote! {
        #input

        impl armature_core::Module for #struct_name {
            fn providers(&self) -> Vec<armature_core::ProviderRegistration> {
                vec![
                    #(#provider_registrations),*
                ]
            }

            fn controllers(&self) -> Vec<armature_core::ControllerRegistration> {
                vec![
                    #(#controller_registrations),*
                ]
            }

            fn imports(&self) -> Vec<Box<dyn armature_core::Module>> {
                vec![
                    #(#import_instances),*
                ]
            }

            fn exports(&self) -> Vec<std::any::TypeId> {
                vec![
                    #(#export_ids),*
                ]
            }
        }
    };

    TokenStream::from(expanded)
}
