use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Attribute, FnArg, Ident, ImplItem, ImplItemFn, ItemImpl, LitStr, PatType, Type,
    parse_macro_input,
};

use crate::routes::{generate_extraction, parse_extractor_attr, signature_has_extractors};

/// One `#[get("/x")]`-style attribute declared on a handler method.
struct RouteAttr {
    method: String,
    path: String,
}

/// Information about a handler method: every route it declares, plus the shape
/// of its signature.
struct RouteInfo {
    /// All route attributes on the method, in declaration order. A method may
    /// legitimately carry several (`#[get("/x")] #[head("/x")]`) and each one
    /// gets its own registration — dropping all but the first used to lose the
    /// remaining routes with no diagnostic.
    routes: Vec<RouteAttr>,
    handler_name: Ident,
    is_async: bool,
    has_self: bool,
    has_request_param: bool,
}

/// The route-method attribute names recognized on a handler method.
const ROUTE_ATTRS: [&str; 8] = [
    "get", "post", "put", "delete", "patch", "options", "head", "query",
];

fn is_route_attr(attr: &Attribute) -> Option<String> {
    let ident = attr.path().get_ident()?;
    let name = ident.to_string();
    ROUTE_ATTRS.contains(&name.as_str()).then_some(name)
}

/// Extract route information from a method's attributes
fn extract_route_info(method: &ImplItemFn) -> syn::Result<Option<RouteInfo>> {
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

    let mut routes = Vec::new();
    for attr in &method.attrs {
        let Some(method_name) = is_route_attr(attr) else {
            continue;
        };

        // Parse the path argument. `#[get]` with no argument is the
        // controller-root route; `#[get(123)]` is a mistake, not an empty path.
        let path = if attr.meta.require_list().is_ok() {
            attr.parse_args::<LitStr>()?.value()
        } else {
            String::new()
        };

        routes.push(RouteAttr {
            method: method_name.to_uppercase(),
            path,
        });
    }

    if routes.is_empty() {
        return Ok(None);
    }

    Ok(Some(RouteInfo {
        routes,
        handler_name,
        is_async,
        has_self,
        has_request_param,
    }))
}

/// Rewrite a handler method that uses parameter extractors so it takes a single
/// `HttpRequest` and binds each declared parameter from it.
///
/// `#[routes]` is the outer macro on the impl block, so it expands before any
/// per-method route attribute could — which is why the extractor codegen has to
/// live here rather than only in `routes::route_impl` (that path only ever runs
/// for free functions, which are never registered as controller routes).
fn rewrite_with_extractors(method: &ImplItemFn) -> syn::Result<TokenStream2> {
    let sig = &method.sig;
    let mut extractions = Vec::new();

    for arg in &sig.inputs {
        let FnArg::Typed(pat_type) = arg else {
            continue; // the receiver is re-emitted verbatim below
        };
        let Some((kind, param_name, param_type)) = parse_extractor_attr(pat_type) else {
            return Err(syn::Error::new_spanned(
                pat_type,
                "every parameter of a handler that uses extractors must carry an extractor \
                 attribute (`#[body]`, `#[param(\"..\")]`, `#[query(\"..\")]`, `#[header(\"..\")]`, \
                 `#[headers]`, `#[raw_body]`) or be an `HttpRequest`",
            ));
        };
        extractions.push(generate_extraction(&kind, &param_name, &param_type)?);
    }

    // Keep the receiver exactly as written (`&self`, `&mut self`, `self`)
    // rather than assuming `&self`.
    let receiver = sig.inputs.iter().find_map(|arg| match arg {
        FnArg::Receiver(receiver) => Some(receiver),
        FnArg::Typed(_) => None,
    });
    let self_param = match receiver {
        Some(receiver) => quote! { #receiver, },
        None => quote! {},
    };

    let kept_attrs = strip_route_attrs(&method.attrs);
    let vis = &method.vis;
    let asyncness = &sig.asyncness;
    let ident = &sig.ident;
    let output = &sig.output;
    let (impl_generics, _, where_clause) = sig.generics.split_for_impl();
    let block = &method.block;

    Ok(quote! {
        #(#kept_attrs)*
        #vis #asyncness fn #ident #impl_generics (
            #self_param __request: armature_core::HttpRequest
        ) #output #where_clause {
            // Bind every declared parameter from the request.
            #(#extractions)*

            // Then run the original body, which sees its parameters by name.
            #block
        }
    })
}

/// Remove route attributes from a method (get, post, put, delete, patch, options, head, query)
fn strip_route_attrs(attrs: &[Attribute]) -> Vec<Attribute> {
    attrs
        .iter()
        .filter(|attr| {
            if let Some(ident) = attr.path().get_ident() {
                let name = ident.to_string();
                !matches!(
                    name.as_str(),
                    "get" | "post" | "put" | "delete" | "patch" | "options" | "head" | "query"
                )
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

    match expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand(input: &ItemImpl) -> syn::Result<TokenStream2> {
    // Get the controller type
    let controller_type = &input.self_ty;

    // Collect route information and generate route handlers
    let mut route_handlers: Vec<TokenStream2> = Vec::new();
    let mut route_definitions: Vec<TokenStream2> = Vec::new();
    let mut modified_items: Vec<TokenStream2> = Vec::new();

    for item in &input.items {
        let ImplItem::Fn(method) = item else {
            // Keep other impl items as-is
            modified_items.push(quote! { #item });
            continue;
        };

        let uses_extractors = signature_has_extractors(&method.sig.inputs);

        let Some(route_info) = extract_route_info(method)? else {
            // A non-route method cannot carry extractor attributes: nothing
            // would ever strip them, so they would reach rustc as unknown
            // attributes.
            if uses_extractors {
                return Err(syn::Error::new_spanned(
                    &method.sig,
                    "parameter extractors are only supported on route handler methods — \
                     add a route attribute such as `#[get(\"/path\")]`",
                ));
            }
            // Keep non-route methods as-is
            modified_items.push(quote! { #item });
            continue;
        };

        let handler_name = &route_info.handler_name;
        let handler_name_str = handler_name.to_string();
        // An extractor-based handler is rewritten below to take a single
        // `HttpRequest`, so it is registered as a request-taking handler.
        let has_request_param = route_info.has_request_param || uses_extractors;

        for route in &route_info.routes {
            let method_str = &route.method;
            let path_str = &route.path;

            // Route metadata mirroring what the module route registrar
            // registers (controller-relative paths), surfaced through
            // `Controller::routes()`.
            route_definitions.push(quote! {
                armature_core::RouteDefinition {
                    method: armature_core::HttpMethod::from_str(#method_str)
                        .unwrap_or(armature_core::HttpMethod::GET),
                    path: #path_str.to_string(),
                    handler_name: #handler_name_str.to_string(),
                }
            });

            // Generate the route handler registration based on method signature
            // Four cases: (has_self, has_request_param)
            let handler = match (route_info.has_self, has_request_param, route_info.is_async) {
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
        }

        if uses_extractors {
            modified_items.push(rewrite_with_extractors(method)?);
        } else {
            // Create modified method without route attributes
            let mut modified_method = method.clone();
            modified_method.attrs = strip_route_attrs(&method.attrs);
            modified_items.push(quote! { #modified_method });
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

            /// Route metadata declared on this controller.
            ///
            /// This inherent method shadows the empty default supplied by the
            /// `#[controller]` macro, so `Controller::routes()` reports the same
            /// routes the module registrar registers (controller-relative paths).
            pub fn __collect_routes() -> Vec<armature_core::RouteDefinition> {
                vec![
                    #(#route_definitions),*
                ]
            }
        }
    };

    Ok(expanded)
}
