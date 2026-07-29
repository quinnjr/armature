use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Expr, FnArg, ItemFn, Lit, Pat, Token, parse::Parse, parse::ParseStream, parse_macro_input,
};

/// Find the identifier of the handler's first non-receiver parameter — the
/// request binding — so generated code refers to it by its real name instead
/// of assuming `req`.
fn request_ident(input: &ItemFn) -> syn::Ident {
    input
        .sig
        .inputs
        .iter()
        .find_map(|arg| match arg {
            FnArg::Typed(pat_type) => match pat_type.pat.as_ref() {
                Pat::Ident(pat_ident) => Some(pat_ident.ident.clone()),
                _ => None,
            },
            FnArg::Receiver(_) => None,
        })
        .unwrap_or_else(|| syn::Ident::new("req", proc_macro2::Span::call_site()))
}

/// Arguments for the timeout attribute
/// Parses: #[timeout(5)] or #[timeout(seconds = 5)] or #[timeout(ms = 5000)]
pub struct TimeoutArgs {
    pub duration_ms: u64,
}

impl Parse for TimeoutArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // Try to parse named parameter first (seconds = X or ms = X)
        if input.peek(syn::Ident) {
            let ident: syn::Ident = input.parse()?;
            let _eq: Token![=] = input.parse()?;

            let value: Expr = input.parse()?;
            let duration_ms = match &value {
                Expr::Lit(expr_lit) => match &expr_lit.lit {
                    Lit::Int(lit_int) => lit_int.base10_parse::<u64>().unwrap_or(30000),
                    Lit::Float(lit_float) => {
                        (lit_float.base10_parse::<f64>().unwrap_or(30.0) * 1000.0) as u64
                    }
                    _ => 30000,
                },
                _ => 30000,
            };

            let duration_ms = match ident.to_string().as_str() {
                "seconds" | "secs" | "s" => duration_ms * 1000,
                "milliseconds" | "millis" | "ms" => duration_ms,
                "minutes" | "mins" | "m" => duration_ms * 60 * 1000,
                _ => duration_ms * 1000, // Default to seconds
            };

            Ok(Self { duration_ms })
        } else if input.peek(Lit) {
            // Parse as just a number (defaults to seconds)
            let lit: Lit = input.parse()?;
            let seconds = match lit {
                Lit::Int(lit_int) => lit_int.base10_parse::<u64>().unwrap_or(30),
                Lit::Float(lit_float) => lit_float.base10_parse::<f64>().unwrap_or(30.0) as u64,
                _ => 30,
            };
            Ok(Self {
                duration_ms: seconds * 1000,
            })
        } else {
            // Default timeout of 30 seconds
            Ok(Self { duration_ms: 30000 })
        }
    }
}

/// Implementation of the `#[timeout(...)]` attribute macro.
///
/// This macro wraps a route handler function to apply a timeout.
///
/// # Usage
///
/// ```ignore
/// use armature::{get, timeout};
/// use armature_core::{HttpRequest, HttpResponse, Error};
///
/// // Timeout in seconds (default unit)
/// #[timeout(5)]
/// #[get("/quick")]
/// async fn quick_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
///
/// // Timeout with explicit unit
/// #[timeout(seconds = 30)]
/// #[get("/slow")]
/// async fn slow_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
///
/// // Timeout in milliseconds
/// #[timeout(ms = 500)]
/// #[get("/fast")]
/// async fn fast_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
/// ```
///
/// # How It Works
///
/// The macro generates a wrapper function that:
/// 1. Defines an inner function containing the original handler body
/// 2. Awaits the inner function's call wrapped in `tokio::time::timeout(...)` with the specified duration
/// 3. Returns a timeout error if the handler doesn't complete in time
pub fn timeout_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as TimeoutArgs);
    let input = parse_macro_input!(item as ItemFn);

    let func_name = &input.sig.ident;
    let func_vis = &input.vis;
    let func_attrs = &input.attrs;
    let func_output = &input.sig.output;
    let func_body = &input.block;
    let func_inputs = &input.sig.inputs;
    let is_async = input.sig.asyncness.is_some();

    let duration_ms = args.duration_ms;

    // Generate the appropriate function signature
    let async_marker = if is_async {
        quote! { async }
    } else {
        quote! {}
    };

    // Create the inner handler name
    let inner_fn_name = syn::Ident::new(&format!("__{}_inner", func_name), func_name.span());
    let req_ident = request_ident(&input);

    let expanded = quote! {
        #(#func_attrs)*
        #func_vis #async_marker fn #func_name(#func_inputs) #func_output {
            use std::time::Duration;

            // Define the inner function with original body
            #async_marker fn #inner_fn_name(#func_inputs) #func_output
                #func_body

            // Apply timeout
            let __timeout_duration = Duration::from_millis(#duration_ms);

            match tokio::time::timeout(__timeout_duration, #inner_fn_name(#req_ident)).await {
                Ok(result) => result,
                Err(_) => Err(armature_core::Error::RequestTimeout(format!(
                    "Request exceeded timeout of {} ms",
                    #duration_ms
                ))),
            }
        }
    };

    TokenStream::from(expanded)
}
