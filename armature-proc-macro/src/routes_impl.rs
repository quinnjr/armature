use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Attribute, FnArg, Ident, ImplItem, ImplItemFn, ItemImpl, LitStr, PatType, Type,
    parse_macro_input,
};

/// Information about a route extracted from method attributes
struct RouteInfo {
    method: String,
    path: String,
    handler_name: Ident,
    is_async: bool,
    has_self: bool,
    has_request_param: bool,
}

/// Extract route information from a method's attributes
fn extract_route_info(method: &ImplItemFn) -> Option<RouteInfo> {
    let handler_name = method.sig.ident.clone();
    let is_async = method.sig.asyncness.is_some();

    // Check if method has &self receiver
    let has_self = method
        .sig
        .inputs
        .iter()
        .any(|arg| matches!(arg, FnArg::Receiver(_)));

    // Check if method has HttpRequest parameter
    let has_request_param = method.sig.inputs.iter().any(|arg| {
        if let FnArg::Typed(PatType { ty, .. }) = arg
            && let Type::Path(type_path) = ty.as_ref()
            && let Some(segment) = type_path.path.segments.last()
        {
            return segment.ident == "HttpRequest";
        }
        false
    });

    for attr in &method.attrs {
        let path_ident = attr.path();
        if let Some(ident) = path_ident.get_ident() {
            let method_name = ident.to_string();
            if matches!(
                method_name.as_str(),
                "get" | "post" | "put" | "delete" | "patch"
            ) {
                // Parse the path argument
                let route_path = if attr.meta.require_list().is_ok() {
                    attr.parse_args::<LitStr>()
                        .map(|lit| lit.value())
                        .unwrap_or_default()
                } else {
                    String::new()
                };

                return Some(RouteInfo {
                    method: method_name.to_uppercase(),
                    path: route_path,
                    handler_name,
                    is_async,
                    has_self,
                    has_request_param,
                });
            }
        }
    }

    None
}

/// Remove route attributes from a method (get, post, put, delete, patch)
fn strip_route_attrs(attrs: &[Attribute]) -> Vec<Attribute> {
    attrs
        .iter()
        .filter(|attr| {
            if let Some(ident) = attr.path().get_ident() {
                let name = ident.to_string();
                !matches!(name.as_str(), "get" | "post" | "put" | "delete" | "patch")
            } else {
                true
            }
        })
        .cloned()
        .collect()
}

pub fn routes_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let _ = attr; // No attributes expected
    let input = parse_macro_input!(item as ItemImpl);

    // Get the controller type
    let controller_type = &input.self_ty;

    // Collect route information and generate route handlers
    let mut route_handlers: Vec<TokenStream2> = Vec::new();
    let mut modified_items: Vec<ImplItem> = Vec::new();

    for item in &input.items {
        if let ImplItem::Fn(method) = item {
            if let Some(route_info) = extract_route_info(method) {
                let method_str = &route_info.method;
                let path_str = &route_info.path;
                let handler_name = &route_info.handler_name;

                // Generate the route handler registration based on method signature
                // Four cases: (has_self, has_request_param)
                let handler = match (
                    route_info.has_self,
                    route_info.has_request_param,
                    route_info.is_async,
                ) {
                    // Instance method with request: controller.method(req)
                    (true, true, true) => quote! {
                        (
                            #method_str,
                            #path_str,
                            std::sync::Arc::new(move |req: armature_core::HttpRequest| {
                                let controller = controller.clone();
                                Box::pin(async move {
                                    controller.#handler_name(req).await
                                }) as std::pin::Pin<Box<dyn std::future::Future<Output = Result<armature_core::HttpResponse, armature_core::Error>> + Send>>
                            }) as armature_core::route_registry::RouteHandlerFn
                        )
                    },
                    (true, true, false) => quote! {
                        (
                            #method_str,
                            #path_str,
                            std::sync::Arc::new(move |req: armature_core::HttpRequest| {
                                let controller = controller.clone();
                                Box::pin(async move {
                                    controller.#handler_name(req)
                                }) as std::pin::Pin<Box<dyn std::future::Future<Output = Result<armature_core::HttpResponse, armature_core::Error>> + Send>>
                            }) as armature_core::route_registry::RouteHandlerFn
                        )
                    },
                    // Instance method without request: controller.method()
                    (true, false, true) => quote! {
                        (
                            #method_str,
                            #path_str,
                            std::sync::Arc::new(move |_req: armature_core::HttpRequest| {
                                let controller = controller.clone();
                                Box::pin(async move {
                                    controller.#handler_name().await
                                }) as std::pin::Pin<Box<dyn std::future::Future<Output = Result<armature_core::HttpResponse, armature_core::Error>> + Send>>
                            }) as armature_core::route_registry::RouteHandlerFn
                        )
                    },
                    (true, false, false) => quote! {
                        (
                            #method_str,
                            #path_str,
                            std::sync::Arc::new(move |_req: armature_core::HttpRequest| {
                                let controller = controller.clone();
                                Box::pin(async move {
                                    controller.#handler_name()
                                }) as std::pin::Pin<Box<dyn std::future::Future<Output = Result<armature_core::HttpResponse, armature_core::Error>> + Send>>
                            }) as armature_core::route_registry::RouteHandlerFn
                        )
                    },
                    // Associated function with request: Type::method(req)
                    (false, true, true) => quote! {
                        (
                            #method_str,
                            #path_str,
                            std::sync::Arc::new(move |req: armature_core::HttpRequest| {
                                Box::pin(async move {
                                    #controller_type::#handler_name(req).await
                                }) as std::pin::Pin<Box<dyn std::future::Future<Output = Result<armature_core::HttpResponse, armature_core::Error>> + Send>>
                            }) as armature_core::route_registry::RouteHandlerFn
                        )
                    },
                    (false, true, false) => quote! {
                        (
                            #method_str,
                            #path_str,
                            std::sync::Arc::new(move |req: armature_core::HttpRequest| {
                                Box::pin(async move {
                                    #controller_type::#handler_name(req)
                                }) as std::pin::Pin<Box<dyn std::future::Future<Output = Result<armature_core::HttpResponse, armature_core::Error>> + Send>>
                            }) as armature_core::route_registry::RouteHandlerFn
                        )
                    },
                    // Associated function without request: Type::method()
                    (false, false, true) => quote! {
                        (
                            #method_str,
                            #path_str,
                            std::sync::Arc::new(move |_req: armature_core::HttpRequest| {
                                Box::pin(async move {
                                    #controller_type::#handler_name().await
                                }) as std::pin::Pin<Box<dyn std::future::Future<Output = Result<armature_core::HttpResponse, armature_core::Error>> + Send>>
                            }) as armature_core::route_registry::RouteHandlerFn
                        )
                    },
                    (false, false, false) => quote! {
                        (
                            #method_str,
                            #path_str,
                            std::sync::Arc::new(move |_req: armature_core::HttpRequest| {
                                Box::pin(async move {
                                    #controller_type::#handler_name()
                                }) as std::pin::Pin<Box<dyn std::future::Future<Output = Result<armature_core::HttpResponse, armature_core::Error>> + Send>>
                            }) as armature_core::route_registry::RouteHandlerFn
                        )
                    },
                };

                route_handlers.push(handler);

                // Create modified method without route attributes
                let mut modified_method = method.clone();
                modified_method.attrs = strip_route_attrs(&method.attrs);
                modified_items.push(ImplItem::Fn(modified_method));
            } else {
                // Keep non-route methods as-is
                modified_items.push(item.clone());
            }
        } else {
            // Keep other impl items as-is
            modified_items.push(item.clone());
        }
    }

    // Reconstruct the impl block with modified items
    let attrs = &input.attrs;
    let unsafety = &input.unsafety;
    let generics = &input.generics;
    let trait_ = input.trait_.as_ref().map(|(bang, path, for_)| {
        quote! { #bang #path #for_ }
    });

    let expanded = quote! {
        #(#attrs)*
        #unsafety impl #generics #trait_ #controller_type {
            #(#modified_items)*

            /// Returns the route handlers for this controller.
            /// Generated by the #[routes] macro.
            #[allow(clippy::redundant_clone)]
            pub fn __route_handlers(controller: std::sync::Arc<Self>) -> Vec<(&'static str, &'static str, armature_core::route_registry::RouteHandlerFn)> {
                // Create individual clones for each route handler closure
                let mut handlers = Vec::new();
                #({
                    let controller = controller.clone();
                    handlers.push(#route_handlers);
                })*
                handlers
            }
        }
    };

    TokenStream::from(expanded)
}
