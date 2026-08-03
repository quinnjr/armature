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

        // A mistyped or repeated key used to be dropped on the floor, producing
        // a module that registers nothing and only fails at request time with a
        // DI or 404 error. Both are compile errors now.
        let mut seen: Vec<String> = Vec::new();

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![:]>()?;

            let content;
            syn::bracketed!(content in input);
            let types: Punctuated<Type, Token![,]> =
                content.parse_terminated(Type::parse, Token![,])?;

            let key_name = key.to_string();
            if seen.contains(&key_name) {
                return Err(syn::Error::new(
                    key.span(),
                    format!("duplicate module key `{key_name}`"),
                ));
            }

            let slot = match key_name.as_str() {
                "providers" => &mut providers,
                "controllers" => &mut controllers,
                "imports" => &mut imports,
                "exports" => &mut exports,
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown module key `{other}` (expected providers, controllers, imports or exports)"
                        ),
                    ));
                }
            };
            *slot = types.into_iter().collect();
            seen.push(key_name);

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
                        Ok(instance) => {
                            // Wrap first (instead of `container.register(instance)`
                            // directly) so the lifecycle probe below and the final
                            // registration share the exact same `Arc`, and so the
                            // probe runs at a monomorphic call site (required for
                            // the autoref-specialization trick it relies on to
                            // detect which lifecycle hook traits `#ty` implements).
                            let instance = std::sync::Arc::new(instance);
                            if let Some(__armature_lifecycle) = container.lifecycle_manager() {
                                armature_core::__armature_register_lifecycle_hooks!(
                                    &__armature_lifecycle,
                                    std::any::type_name::<#ty>(),
                                    instance
                                );
                            }
                            container.register_by_id(
                                std::any::TypeId::of::<#ty>(),
                                instance as std::sync::Arc<dyn std::any::Any + Send + Sync>,
                            );
                        }
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
                    // Fallbacks so controllers WITHOUT `#[guard]` / `#[middleware]`
                    // still resolve the accessors below (returning empty). When
                    // those attributes ARE present, the inherent
                    // `__get_guard_factories` / `__get_middleware_factories`
                    // methods generated on the struct shadow these defaults.
                    #[allow(dead_code)]
                    trait __ArmatureGuardMeta {
                        fn __get_guard_factories()
                            -> Vec<fn() -> Box<dyn armature_core::guard::Guard>> {
                            Vec::new()
                        }
                    }
                    impl<__T> __ArmatureGuardMeta for __T {}
                    #[allow(dead_code)]
                    trait __ArmatureMiddlewareMeta {
                        fn __get_middleware_factories()
                            -> Vec<fn() -> Box<dyn armature_core::middleware::Middleware>> {
                            Vec::new()
                        }
                    }
                    impl<__T> __ArmatureMiddlewareMeta for __T {}

                    // Get base path from constant
                    let base_path = #ty::BASE_PATH;

                    // Downcast the controller
                    let controller = controller_any.downcast::<#ty>()
                        .map_err(|_| armature_core::Error::Internal("Failed to downcast controller".to_string()))?;

                    // Wrap in Arc for shared ownership
                    let controller = std::sync::Arc::new(*controller);

                    // Controller-struct-level guards and middleware apply to every
                    // route declared on this controller. Both are constructed
                    // exactly once, here at registration time — the per-request
                    // closures below only clone the resulting `Arc`s.
                    let __guards: std::sync::Arc<
                        Vec<std::sync::Arc<dyn armature_core::guard::Guard>>,
                    > = std::sync::Arc::new(
                        #ty::__get_guard_factories()
                            .into_iter()
                            .map(|__factory| {
                                std::sync::Arc::from(__factory())
                                    as std::sync::Arc<dyn armature_core::guard::Guard>
                            })
                            .collect(),
                    );
                    let __middlewares: Vec<
                        std::sync::Arc<dyn armature_core::middleware::Middleware>,
                    > = #ty::__get_middleware_factories()
                        .into_iter()
                        .map(|__factory| {
                            std::sync::Arc::from(__factory())
                                as std::sync::Arc<dyn armature_core::middleware::Middleware>
                        })
                        .collect();
                    let __enforce = !__guards.is_empty() || !__middlewares.is_empty();

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

                        // When the controller declares struct-level guards or
                        // middleware, wrap the handler so they run per request:
                        // guards first (a non-activating guard yields 403, an
                        // erroring one propagates its error), then the middleware
                        // chain around the handler. Otherwise use the plain handler
                        // with zero overhead.
                        let __boxed_handler = if __enforce {
                            // Build the middleware chain ONCE, here at registration
                            // time, from the innermost handler outward so the
                            // first-listed middleware ends up outermost and runs
                            // first. `HandlerFn` is `Arc`-based, so the per-request
                            // closure below only pays a refcount bump.
                            let mut __chain: armature_core::middleware::HandlerFn = {
                                let __orig_handler = handler.clone();
                                std::sync::Arc::new(move |__r: armature_core::HttpRequest| {
                                    let __orig_handler = __orig_handler.clone();
                                    Box::pin(async move { __orig_handler(__r).await })
                                        as std::pin::Pin<Box<dyn std::future::Future<
                                            Output = Result<
                                                armature_core::HttpResponse,
                                                armature_core::Error,
                                            >,
                                        > + Send>>
                                })
                            };
                            for __mw in __middlewares.iter().rev().cloned() {
                                let __next_handler = __chain.clone();
                                __chain = std::sync::Arc::new(
                                    move |__r: armature_core::HttpRequest| {
                                        let __mw = __mw.clone();
                                        let __next_handler = __next_handler.clone();
                                        Box::pin(async move {
                                            let __next: armature_core::middleware::Next =
                                                Box::new(move |__rr: armature_core::HttpRequest| {
                                                    __next_handler(__rr)
                                                });
                                            armature_core::middleware::Middleware::handle(
                                                &*__mw, __r, __next,
                                            )
                                            .await
                                        })
                                            as std::pin::Pin<Box<dyn std::future::Future<
                                                Output = Result<
                                                    armature_core::HttpResponse,
                                                    armature_core::Error,
                                                >,
                                            > + Send>>
                                    },
                                );
                            }

                            let __guards = __guards.clone();
                            let __wrapped: armature_core::route_registry::RouteHandlerFn =
                                std::sync::Arc::new(move |__req: armature_core::HttpRequest| {
                                    let __guards = __guards.clone();
                                    let __chain = __chain.clone();
                                    Box::pin(async move {
                                        // 1. Guards run before the handler. One
                                        //    context (one request clone) is shared
                                        //    read-only by every guard.
                                        if !__guards.is_empty() {
                                            let __context =
                                                armature_core::guard::GuardContext::new(__req.clone());
                                            for __guard in __guards.iter() {
                                                match armature_core::guard::Guard::can_activate(
                                                    &**__guard,
                                                    &__context,
                                                )
                                                .await
                                                {
                                                    Ok(true) => {}
                                                    Ok(false) => {
                                                        return Ok(armature_core::HttpResponse::new(403)
                                                            .with_body(b"Forbidden".to_vec()));
                                                    }
                                                    Err(e) => return Err(e),
                                                }
                                            }
                                        }

                                        // 2. Run the pre-built middleware chain.
                                        __chain(__req).await
                                    })
                                        as std::pin::Pin<Box<dyn std::future::Future<
                                            Output = Result<
                                                armature_core::HttpResponse,
                                                armature_core::Error,
                                            >,
                                        > + Send>>
                                });
                            armature_core::handler::from_legacy_handler(__wrapped)
                        } else {
                            armature_core::handler::from_legacy_handler(handler.clone())
                        };

                        let route = armature_core::routing::Route {
                            method: armature_core::HttpMethod::from_str(method)
                                .unwrap_or(armature_core::HttpMethod::GET),
                            path: full_path,
                            handler: __boxed_handler,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_keys_are_collected() {
        let args: ModuleArgs =
            syn::parse_str("providers: [A, B], controllers: [C]").expect("must parse");
        assert_eq!(args.providers.len(), 2);
        assert_eq!(args.controllers.len(), 1);
        assert!(args.imports.is_empty());
        assert!(args.exports.is_empty());
    }

    #[test]
    fn a_mistyped_key_is_rejected() {
        // `provider:` (singular) used to register nothing and only fail at
        // request time with a DI error.
        assert!(syn::parse_str::<ModuleArgs>("provider: [A]").is_err());
    }

    #[test]
    fn a_duplicate_key_is_rejected() {
        assert!(syn::parse_str::<ModuleArgs>("providers: [A], providers: [B]").is_err());
    }
}
