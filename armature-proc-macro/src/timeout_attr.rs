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

/// Millisecond multiplier for a timeout unit name.
///
/// `None` for anything unrecognized so a typo (`hours = 2`, `millis_ = 5`) is a
/// compile error rather than a silently reinterpreted duration — the old
/// catch-all treated every unknown unit as *seconds*.
fn unit_millis(unit: &str) -> Option<u64> {
    match unit.to_ascii_lowercase().as_str() {
        "s" | "secs" | "seconds" => Some(1000),
        "ms" | "millis" | "milliseconds" => Some(1),
        "m" | "mins" | "minutes" => Some(60 * 1000),
        _ => None,
    }
}

/// The set of accepted unit names, for use in diagnostics.
const KNOWN_UNITS: &str = "s/secs/seconds, ms/millis/milliseconds, m/mins/minutes";

/// Scale a duration by a unit multiplier, rejecting an overflowing product.
fn scale(value: f64, multiplier: u64, span: proc_macro2::Span) -> syn::Result<u64> {
    let millis = value * multiplier as f64;
    if !millis.is_finite() || millis < 0.0 || millis > u64::MAX as f64 {
        return Err(syn::Error::new(
            span,
            "timeout does not fit in a millisecond count",
        ));
    }
    Ok(millis as u64)
}

impl Parse for TimeoutArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // Named parameter form: `seconds = 30`, `ms = 500`, `minutes = 2`.
        if input.peek(syn::Ident) {
            let ident: syn::Ident = input.parse()?;
            let _eq: Token![=] = input.parse()?;

            let value: Expr = input.parse()?;
            let amount = match &value {
                Expr::Lit(expr_lit) => match &expr_lit.lit {
                    Lit::Int(lit_int) => lit_int.base10_parse::<u64>()? as f64,
                    Lit::Float(lit_float) => lit_float.base10_parse::<f64>()?,
                    other => {
                        return Err(syn::Error::new_spanned(
                            other,
                            "timeout must be a numeric literal",
                        ));
                    }
                },
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "timeout must be a numeric literal",
                    ));
                }
            };

            let unit = ident.to_string();
            let Some(multiplier) = unit_millis(&unit) else {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("unknown timeout unit `{unit}` (expected {KNOWN_UNITS})"),
                ));
            };

            Ok(Self {
                duration_ms: scale(amount, multiplier, ident.span())?,
            })
        } else if input.peek(Lit) {
            // Bare literal, defaulting to seconds. A literal may still carry a
            // unit *suffix* (`500ms` lexes as an integer literal suffixed `ms`),
            // which must be honored rather than dropped.
            let lit: Lit = input.parse()?;
            let (amount, suffix, span) = match &lit {
                Lit::Int(lit_int) => (
                    lit_int.base10_parse::<u64>()? as f64,
                    lit_int.suffix().to_string(),
                    lit_int.span(),
                ),
                Lit::Float(lit_float) => (
                    lit_float.base10_parse::<f64>()?,
                    lit_float.suffix().to_string(),
                    lit_float.span(),
                ),
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "timeout must be a number of seconds, or a `unit = value` pair",
                    ));
                }
            };

            let multiplier = if suffix.is_empty() {
                1000 // No suffix: the documented default unit is seconds.
            } else {
                match unit_millis(&suffix) {
                    Some(m) => m,
                    None => {
                        return Err(syn::Error::new(
                            span,
                            format!("unknown timeout suffix `{suffix}` (expected {KNOWN_UNITS})"),
                        ));
                    }
                }
            };

            Ok(Self {
                duration_ms: scale(amount, multiplier, span)?,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn millis(src: &str) -> u64 {
        syn::parse_str::<TimeoutArgs>(src)
            .unwrap_or_else(|e| panic!("`{src}` must parse: {e}"))
            .duration_ms
    }

    #[test]
    fn documented_forms_scale_by_their_unit() {
        assert_eq!(millis("5"), 5_000);
        assert_eq!(millis("seconds = 30"), 30_000);
        assert_eq!(millis("ms = 500"), 500);
        assert_eq!(millis("minutes = 2"), 120_000);
        assert_eq!(millis("1.5"), 1_500);
        assert_eq!(millis(""), 30_000);
    }

    #[test]
    fn unknown_units_are_rejected() {
        // `hours = 2` used to compile to a *2 second* timeout, and
        // `millis_ = 5` to 5 seconds — both silently, both wrong by 1000x.
        for src in ["hours = 2", "millis_ = 5", "seconds = \"30\"", "5hours"] {
            assert!(
                syn::parse_str::<TimeoutArgs>(src).is_err(),
                "`{src}` must be a compile error, not a silently reinterpreted duration"
            );
        }
    }
}
