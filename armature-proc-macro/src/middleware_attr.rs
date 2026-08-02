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
        // `#input` is the whole `ItemFn` — it already carries its own
        // attributes and visibility, so prefixing them again would emit
        // duplicated attributes and `pub pub async fn`.
        return TokenStream::from(quote! { #input });
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

    // Anything else — a `self` receiver, or a handler with no request
    // parameter at all — cannot be wrapped. The chain's `HandlerFn` is a
    // `'static` `Arc<dyn Fn..>`, so it can capture neither `self` nor the
    // handler's original parameter bindings; the code this arm used to emit
    // could never compile for either case. Reject it explicitly instead.
    let reason = if has_self_receiver(input) {
        "handlers with a `self` receiver are not supported (move the middleware onto the controller struct: `#[middleware(..)] struct MyController;`)"
    } else {
        "the handler must take an `armature_core::HttpRequest` parameter"
    };

    syn::Error::new(
        func_name.span(),
        format!("#[middleware] cannot wrap `{func_name}`: {reason}"),
    )
    .to_compile_error()
    .into()
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

            let constructions: Vec<proc_macro2::TokenStream> = args
                .middlewares
                .iter()
                .map(|mw| quote! { Box::new(#mw) })
                .collect();

            let metadata_impl = crate::struct_factory::factory_metadata_impl(
                &item_struct,
                "middleware",
                &quote! { armature_core::middleware::Middleware },
                &constructions,
            );

            // Re-emit the struct verbatim (preserving its exact form — unit,
            // tuple, or named — including any trailing `;`) and attach the
            // metadata impl. The module route registrar calls
            // `__get_middleware_factories()` to wrap every route on this
            // controller with these middlewares.
            quote! {
                #item_struct

                #metadata_impl
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
