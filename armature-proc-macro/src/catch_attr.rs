//! `#[catch]` attribute macro for creating exception filters.
//!
//! This macro simplifies the creation of exception filters by allowing
//! you to annotate functions directly.
//!
//! # Examples
//!
//! ```ignore
//! use armature_proc_macro::catch;
//!
//! // Catch all errors
//! #[catch]
//! async fn handle_all(error: &Error, ctx: &ExceptionContext) -> HttpResponse {
//!     HttpResponse::internal_server_error()
//! }
//!
//! // Catch specific error types
//! #[catch(NotFound, RouteNotFound)]
//! async fn handle_not_found(error: &Error, ctx: &ExceptionContext) -> HttpResponse {
//!     HttpResponse::not_found()
//! }
//!
//! // With priority
//! #[catch(Validation, priority = 100)]
//! async fn handle_validation(error: &Error, ctx: &ExceptionContext) -> HttpResponse {
//!     HttpResponse::unprocessable_entity()
//! }
//! ```

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Ident, ItemFn, Lit, Meta, Token,
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
};

/// Arguments for the `#[catch]` attribute.
struct CatchArgs {
    /// Error types to catch (empty = catch all)
    error_types: Vec<Ident>,
    /// Filter priority
    priority: Option<i32>,
    /// Filter name
    name: Option<String>,
}

impl Parse for CatchArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut error_types = Vec::new();
        let mut priority = None;
        let mut name = None;

        if input.is_empty() {
            return Ok(Self {
                error_types,
                priority,
                name,
            });
        }

        // Parse comma-separated items
        let items = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;

        // Anything unrecognized is rejected rather than dropped: a mistyped
        // `priorty = 100` silently leaving the filter at priority 0 changes the
        // order filters are consulted in, which is invisible at compile time.
        for item in items {
            match item {
                Meta::Path(path) => {
                    // This is an error type like NotFound
                    let Some(ident) = path.get_ident() else {
                        return Err(syn::Error::new_spanned(
                            &path,
                            "expected a bare error type name (e.g. `NotFound`)",
                        ));
                    };
                    error_types.push(ident.clone());
                }
                Meta::NameValue(nv) => {
                    // This is a key=value like priority = 100
                    let Some(ident) = nv.path.get_ident() else {
                        return Err(syn::Error::new_spanned(
                            &nv.path,
                            "unknown catch option (expected `priority` or `name`)",
                        ));
                    };
                    match ident.to_string().as_str() {
                        "priority" => {
                            if let syn::Expr::Lit(expr_lit) = &nv.value
                                && let Lit::Int(lit_int) = &expr_lit.lit
                            {
                                priority = Some(lit_int.base10_parse()?);
                            } else {
                                return Err(syn::Error::new_spanned(
                                    &nv.value,
                                    "priority must be an integer literal",
                                ));
                            }
                        }
                        "name" => {
                            if let syn::Expr::Lit(expr_lit) = &nv.value
                                && let Lit::Str(lit_str) = &expr_lit.lit
                            {
                                name = Some(lit_str.value());
                            } else {
                                return Err(syn::Error::new_spanned(
                                    &nv.value,
                                    "name must be a string literal",
                                ));
                            }
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                &nv.path,
                                format!(
                                    "unknown catch option `{other}` (expected `priority` or `name`)"
                                ),
                            ));
                        }
                    }
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "expected an error type name, `priority = N` or `name = \"...\"`",
                    ));
                }
            }
        }

        Ok(Self {
            error_types,
            priority,
            name,
        })
    }
}

/// Implementation of the `#[catch]` attribute macro.
pub fn catch_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as CatchArgs);
    let input_fn = parse_macro_input!(item as ItemFn);

    let fn_name = &input_fn.sig.ident;
    let fn_vis = &input_fn.vis;
    let fn_block = &input_fn.block;
    let fn_attrs = &input_fn.attrs;
    let fn_inputs = &input_fn.sig.inputs;
    let fn_output = &input_fn.sig.output;

    // Generate the struct name from the function name
    let struct_name = syn::Ident::new(
        &format!(
            "{}ExceptionFilter",
            fn_name
                .to_string()
                .split('_')
                .map(|s| {
                    let mut c = s.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    }
                })
                .collect::<String>()
        ),
        fn_name.span(),
    );

    // Generate error type matches
    let error_types_impl = if args.error_types.is_empty() {
        quote! { None }
    } else {
        let types: Vec<_> = args.error_types.iter().map(|t| t.to_string()).collect();
        quote! { Some(vec![#(#types),*]) }
    };

    // Generate priority
    let priority_impl = match args.priority {
        Some(p) => quote! { #p },
        None => quote! { 0 },
    };

    // Generate name
    let name_impl = match args.name {
        Some(ref n) => quote! { #n },
        None => {
            let default_name = struct_name.to_string();
            quote! { #default_name }
        }
    };

    // Check if the function is async
    let is_async = input_fn.sig.asyncness.is_some();

    // The user's handler returns `HttpResponse`, but the trait method returns
    // `Option<HttpResponse>`. Re-emit the user's body as an inner function with
    // its original signature (so its parameter names are preserved regardless of
    // what the user called them), call it with the trait-provided `error`/`ctx`,
    // and wrap the result in `Some`.
    let inner_name = syn::Ident::new(&format!("__{}_catch_inner", fn_name), fn_name.span());
    let async_marker = if is_async {
        quote! { async }
    } else {
        quote! {}
    };
    let inner_call = if is_async {
        quote! { #inner_name(error, ctx).await }
    } else {
        quote! { #inner_name(error, ctx) }
    };

    let catch_impl = quote! {
        async fn catch(
            &self,
            error: &armature_core::Error,
            ctx: &armature_core::exception_filter::ExceptionContext,
        ) -> Option<armature_core::HttpResponse> {
            #async_marker fn #inner_name(#fn_inputs) #fn_output #fn_block

            Some(#inner_call)
        }
    };

    let expanded = quote! {
        #(#fn_attrs)*
        #fn_vis struct #struct_name;

        impl #struct_name {
            /// Create a new instance of this exception filter.
            pub fn new() -> Self {
                Self
            }
        }

        impl Default for #struct_name {
            fn default() -> Self {
                Self::new()
            }
        }

        #[async_trait::async_trait]
        impl armature_core::exception_filter::ExceptionFilter for #struct_name {
            #catch_impl

            fn handles(&self) -> Option<Vec<&'static str>> {
                #error_types_impl
            }

            fn priority(&self) -> i32 {
                #priority_impl
            }

            fn name(&self) -> &str {
                #name_impl
            }
        }

        /// Function to create the exception filter (for convenience).
        #fn_vis fn #fn_name() -> #struct_name {
            #struct_name::new()
        }
    };

    expanded.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_forms_parse() {
        let args: CatchArgs = syn::parse_str("NotFound, RouteNotFound").expect("must parse");
        assert_eq!(args.error_types.len(), 2);

        let args: CatchArgs =
            syn::parse_str("Validation, priority = 100, name = \"custom\"").expect("must parse");
        assert_eq!(args.priority, Some(100));
        assert_eq!(args.name.as_deref(), Some("custom"));

        let args: CatchArgs = syn::parse_str("").expect("must parse");
        assert!(args.error_types.is_empty());
    }

    #[test]
    fn unknown_or_ill_typed_options_are_rejected() {
        // Each of these used to be dropped, leaving the filter at priority 0 —
        // silently reordering which filter answers an error.
        for src in [
            "Validation, priorty = 100",
            "NotFound, priority = \"high\"",
            "NotFound, name = 5",
        ] {
            assert!(
                syn::parse_str::<CatchArgs>(src).is_err(),
                "`{src}` must be a compile error, not a silently ignored option"
            );
        }
    }
}
