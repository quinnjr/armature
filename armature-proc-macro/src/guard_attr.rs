use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Expr, FnArg, ItemFn, Pat, Type, parse::Parse, parse::ParseStream, parse_macro_input,
    punctuated::Punctuated, token::Comma,
};

/// Arguments for `#[use_guard(Guard1, Guard2, ...)]` — guard *types*.
struct UseGuardArgs {
    guards: Punctuated<Type, Comma>,
}

impl Parse for UseGuardArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            guards: Punctuated::parse_terminated(input)?,
        })
    }
}

/// Arguments for `#[guard(expr1, expr2, ...)]` — guard *instances*.
struct GuardExprArgs {
    guards: Punctuated<Expr, Comma>,
}

impl Parse for GuardExprArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            guards: Punctuated::parse_terminated(input)?,
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

/// Build the wrapped handler from a set of guard *bindings*.
///
/// Each binding is a statement that introduces a `__guard` value in scope
/// (e.g. `let __guard: MyGuard = Default::default();` or `let __guard = expr;`).
/// The generated wrapper keeps the handler's real signature so the body can
/// still reference its parameters by name, evaluates every guard against the
/// request, and only then runs the original body.
fn build_guard_wrapper(input: &ItemFn, bindings: &[TokenStream2]) -> TokenStream {
    let func_name = &input.sig.ident;
    let func_vis = &input.vis;
    let func_attrs = &input.attrs;
    let func_output = &input.sig.output;
    let func_body = &input.block;
    let func_inputs = &input.sig.inputs;
    let is_async = input.sig.asyncness.is_some();

    if bindings.is_empty() {
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

    let req = request_ident(input);
    // Signature to emit and the expression naming the request. If the handler
    // has no explicit request parameter, inject one named `__request`.
    let (wrapper_inputs, req_expr) = match &req {
        Some(ident) => (quote! { #func_inputs }, quote! { #ident }),
        None => {
            if has_self_receiver(input) {
                (
                    quote! { &self, __request: armature_core::HttpRequest },
                    quote! { __request },
                )
            } else {
                (
                    quote! { __request: armature_core::HttpRequest },
                    quote! { __request },
                )
            }
        }
    };

    let guard_checks = bindings.iter().map(|binding| {
        quote! {
            {
                #binding
                let __context = armature_core::guard::GuardContext::new(#req_expr.clone());
                match __guard.can_activate(&__context).await {
                    Ok(true) => {}
                    Ok(false) => {
                        return Err(armature_core::Error::Forbidden(
                            "Access denied by guard".to_string()
                        ));
                    }
                    Err(e) => return Err(e),
                }
            }
        }
    });

    TokenStream::from(quote! {
        #(#func_attrs)*
        #func_vis #async_marker fn #func_name(#wrapper_inputs) #func_output {
            use armature_core::guard::Guard;

            #(#guard_checks)*

            #func_body
        }
    })
}

/// Implementation of `#[use_guard(...)]` — guard types constructed via `Default`.
pub fn use_guard_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as UseGuardArgs);
    let input = parse_macro_input!(item as ItemFn);

    let bindings: Vec<TokenStream2> = args
        .guards
        .iter()
        .map(|guard| quote! { let __guard: #guard = Default::default(); })
        .collect();

    build_guard_wrapper(&input, &bindings)
}

/// Implementation of `#[guard(...)]`.
///
/// On a function this applies guard *instances*; on a struct it stores guard
/// factory metadata for a controller.
pub fn guard_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input: syn::Item = parse_macro_input!(item);

    match input {
        syn::Item::Struct(item_struct) => {
            let args = match syn::parse::<UseGuardArgs>(attr) {
                Ok(a) => a,
                Err(e) => return e.to_compile_error().into(),
            };

            let struct_name = &item_struct.ident;
            let vis = &item_struct.vis;
            let attrs = &item_struct.attrs;
            let fields = &item_struct.fields;
            let generics = &item_struct.generics;

            let guards: Vec<_> = args.guards.iter().collect();

            let guard_const_name = syn::Ident::new(
                &format!("__GUARDS_{}", struct_name.to_string().to_uppercase()),
                struct_name.span(),
            );

            let guard_factories: Vec<_> = guards
                .iter()
                .enumerate()
                .map(|(i, guard)| {
                    let factory_name =
                        syn::Ident::new(&format!("__guard_factory_{i}"), proc_macro2::Span::call_site());
                    quote! {
                        fn #factory_name() -> Box<dyn armature_core::guard::Guard> {
                            Box::new(<#guard as Default>::default())
                        }
                    }
                })
                .collect();

            let factory_names: Vec<_> = (0..guards.len())
                .map(|i| syn::Ident::new(&format!("__guard_factory_{i}"), proc_macro2::Span::call_site()))
                .collect();

            let guard_count = guards.len();

            quote! {
                #(#attrs)*
                #vis struct #struct_name #generics #fields

                impl #struct_name {
                    /// Get the guard factories for this controller
                    pub fn __get_guard_factories() -> Vec<fn() -> Box<dyn armature_core::guard::Guard>> {
                        vec![#(Self::#factory_names),*]
                    }

                    #(#guard_factories)*
                }

                /// Number of guards for this controller
                pub const #guard_const_name: usize = #guard_count;
            }
            .into()
        }
        syn::Item::Fn(item_fn) => {
            let args = match syn::parse::<GuardExprArgs>(attr) {
                Ok(a) => a,
                Err(e) => return e.to_compile_error().into(),
            };
            let bindings: Vec<TokenStream2> = args
                .guards
                .iter()
                .map(|guard| quote! { let __guard = #guard; })
                .collect();
            build_guard_wrapper(&item_fn, &bindings)
        }
        _ => syn::Error::new_spanned(
            quote! {},
            "guard can only be applied to functions or structs",
        )
        .to_compile_error()
        .into(),
    }
}
