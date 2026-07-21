use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Expr, FnArg, ItemFn, Pat, parse::Parse, parse::ParseStream, parse_macro_input,
    punctuated::Punctuated, token::Comma,
};

/// Arguments for `#[use_middleware(expr1, expr2, ...)]` / `#[middleware(...)]`.
///
/// Middlewares are given as *expressions* (e.g. `LoggerMiddleware::new()` or a
/// unit-struct name), matching the documented usage.
struct UseMiddlewareArgs {
    middlewares: Punctuated<Expr, Comma>,
}

impl Parse for UseMiddlewareArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            middlewares: Punctuated::parse_terminated(input)?,
        })
    }
}

/// Find the identifier of the handler's first non-receiver parameter (the
/// request binding), if any.
fn request_ident(input: &ItemFn) -> Option<syn::Ident> {
    input.sig.inputs.iter().find_map(|arg| match arg {
        FnArg::Typed(pat_type) => match pat_type.pat.as_ref() {
            Pat::Ident(pat_ident) => Some(pat_ident.ident.clone()),
            _ => None,
        },
        FnArg::Receiver(_) => None,
    })
}

fn has_self_receiver(input: &ItemFn) -> bool {
    input
        .sig
        .inputs
        .first()
        .is_some_and(|arg| matches!(arg, FnArg::Receiver(_)))
}

/// Build the middleware-wrapped handler.
///
/// The generated wrapper preserves the handler's real signature, builds a
/// [`MiddlewareChain`](armature_core::middleware::MiddlewareChain) from the
/// supplied middleware expressions, and applies it to the request. The original
/// body runs as the innermost handler, receiving the (possibly transformed)
/// request under the handler's own parameter name.
pub fn use_middleware_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as UseMiddlewareArgs);
    let input = parse_macro_input!(item as ItemFn);
    build_middleware_wrapper(&args, &input)
}

fn build_middleware_wrapper(args: &UseMiddlewareArgs, input: &ItemFn) -> TokenStream {
    let func_name = &input.sig.ident;
    let func_vis = &input.vis;
    let func_attrs = &input.attrs;
    let func_output = &input.sig.output;
    let func_body = &input.block;
    let func_inputs = &input.sig.inputs;
    let is_async = input.sig.asyncness.is_some();

    let middlewares: Vec<_> = args.middlewares.iter().collect();
    if middlewares.is_empty() {
        return TokenStream::from(quote! {
            #(#func_attrs)*
            #func_vis #input
        });
    }

    let async_marker = if is_async {
        quote! { async }
    } else {
        quote! {}
    };

    let middleware_setup = middlewares.iter().map(|mw| {
        quote! { __middleware_chain.use_middleware(#mw); }
    });

    let inner_fn_name = syn::Ident::new(&format!("__{func_name}_inner"), func_name.span());
    let inner_call = if is_async {
        quote! { #inner_fn_name(__req).await }
    } else {
        quote! { #inner_fn_name(__req) }
    };

    let req = request_ident(input);

    // The common, documented case: a free function with an explicit request
    // parameter. The body is re-emitted as an inner function so it binds the
    // request under its real name.
    if let (Some(req_ident), false) = (&req, has_self_receiver(input)) {
        return TokenStream::from(quote! {
            #(#func_attrs)*
            #func_vis #async_marker fn #func_name(#func_inputs) #func_output {
                use armature_core::middleware::{MiddlewareChain, Middleware};
                use std::sync::Arc;

                let mut __middleware_chain = MiddlewareChain::new();
                #(#middleware_setup)*

                #async_marker fn #inner_fn_name(#func_inputs) #func_output
                    #func_body

                let __inner_handler: armature_core::middleware::HandlerFn = Arc::new(
                    move |__req: armature_core::HttpRequest| {
                        Box::pin(async move { #inner_call })
                            as std::pin::Pin<Box<dyn std::future::Future<
                                Output = Result<armature_core::HttpResponse, armature_core::Error>,
                            > + Send>>
                    },
                );

                __middleware_chain.apply(#req_ident, __inner_handler).await
            }
        });
    }

    // Fallback (self-receiver or extractor-only handlers): inject a request
    // parameter and run the body directly inside the chain handler.
    let (wrapper_inputs, req_expr) = if has_self_receiver(input) {
        (
            quote! { &self, __request: armature_core::HttpRequest },
            quote! { __request },
        )
    } else {
        (
            quote! { __request: armature_core::HttpRequest },
            quote! { __request },
        )
    };

    TokenStream::from(quote! {
        #(#func_attrs)*
        #func_vis #async_marker fn #func_name(#wrapper_inputs) #func_output {
            use armature_core::middleware::{MiddlewareChain, Middleware};
            use std::sync::Arc;

            let mut __middleware_chain = MiddlewareChain::new();
            #(#middleware_setup)*

            let __inner_handler: armature_core::middleware::HandlerFn = Arc::new(
                move |__req: armature_core::HttpRequest| {
                    Box::pin(async move { #func_body })
                        as std::pin::Pin<Box<dyn std::future::Future<
                            Output = Result<armature_core::HttpResponse, armature_core::Error>,
                        > + Send>>
                },
            );

            __middleware_chain.apply(#req_expr, __inner_handler).await
        }
    })
}

/// Implementation of `#[middleware(...)]`.
///
/// On a function this applies middleware; on a struct it stores middleware
/// factory metadata for a controller.
pub fn middleware_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input: syn::Item = parse_macro_input!(item);

    match input {
        syn::Item::Struct(item_struct) => {
            let args = match syn::parse::<UseMiddlewareArgs>(attr) {
                Ok(a) => a,
                Err(e) => return e.to_compile_error().into(),
            };

            let struct_name = &item_struct.ident;
            let vis = &item_struct.vis;
            let attrs = &item_struct.attrs;
            let fields = &item_struct.fields;
            let generics = &item_struct.generics;

            let middlewares: Vec<_> = args.middlewares.iter().collect();

            let middleware_const_name = syn::Ident::new(
                &format!("__MIDDLEWARES_{}", struct_name.to_string().to_uppercase()),
                struct_name.span(),
            );

            let middleware_factories: Vec<_> = middlewares
                .iter()
                .enumerate()
                .map(|(i, mw)| {
                    let factory_name = syn::Ident::new(
                        &format!("__middleware_factory_{i}"),
                        proc_macro2::Span::call_site(),
                    );
                    quote! {
                        fn #factory_name() -> Box<dyn armature_core::middleware::Middleware> {
                            Box::new(#mw)
                        }
                    }
                })
                .collect();

            let factory_names: Vec<_> = (0..middlewares.len())
                .map(|i| {
                    syn::Ident::new(
                        &format!("__middleware_factory_{i}"),
                        proc_macro2::Span::call_site(),
                    )
                })
                .collect();

            let middleware_count = middlewares.len();

            quote! {
                #(#attrs)*
                #vis struct #struct_name #generics #fields

                impl #struct_name {
                    /// Get the middleware factories for this controller
                    pub fn __get_middleware_factories() -> Vec<fn() -> Box<dyn armature_core::middleware::Middleware>> {
                        vec![#(Self::#factory_names),*]
                    }

                    #(#middleware_factories)*
                }

                /// Number of middlewares for this controller
                pub const #middleware_const_name: usize = #middleware_count;
            }
            .into()
        }
        syn::Item::Fn(item_fn) => {
            let args = match syn::parse::<UseMiddlewareArgs>(attr) {
                Ok(a) => a,
                Err(e) => return e.to_compile_error().into(),
            };
            build_middleware_wrapper(&args, &item_fn)
        }
        _ => syn::Error::new_spanned(
            quote! {},
            "middleware can only be applied to functions or structs",
        )
        .to_compile_error()
        .into(),
    }
}
