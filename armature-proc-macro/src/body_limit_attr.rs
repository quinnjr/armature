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

/// Arguments for the body_limit attribute
/// Parses: #[body_limit(1mb)] or #[body_limit(1024)] or #[body_limit(kb = 512)]
pub struct BodyLimitArgs {
    pub limit_bytes: usize,
}

/// Byte multiplier for a size unit name.
///
/// Returns `None` for anything unrecognized so every caller can report the
/// typo instead of quietly falling back to bytes — `#[body_limit(512kb)]`
/// silently meaning 512 *bytes* is exactly the failure this guards against.
fn unit_multiplier(unit: &str) -> Option<usize> {
    match unit.to_ascii_lowercase().as_str() {
        "" | "b" | "byte" | "bytes" => Some(1),
        "k" | "kb" | "kilobytes" => Some(1024),
        "m" | "mb" | "megabytes" => Some(1024 * 1024),
        "g" | "gb" | "gigabytes" => Some(1024 * 1024 * 1024),
        _ => None,
    }
}

/// The set of accepted unit names, for use in diagnostics.
const KNOWN_UNITS: &str = "b/bytes, k/kb/kilobytes, m/mb/megabytes, g/gb/gigabytes";

/// Scale a size by a unit multiplier, rejecting an overflowing product rather
/// than wrapping into a nonsensically small limit.
fn scale(value: f64, multiplier: usize, span: proc_macro2::Span) -> syn::Result<usize> {
    let bytes = value * multiplier as f64;
    if !bytes.is_finite() || bytes < 0.0 || bytes > usize::MAX as f64 {
        return Err(syn::Error::new(
            span,
            "body limit does not fit in a byte count",
        ));
    }
    Ok(bytes as usize)
}

impl Parse for BodyLimitArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // Named parameter form: `kb = 512`, `mb = 5`, `bytes = 2048`.
        if input.peek(syn::Ident) && input.peek2(Token![=]) {
            let ident: syn::Ident = input.parse()?;
            let _eq: Token![=] = input.parse()?;

            let value: Expr = input.parse()?;
            let num = match &value {
                Expr::Lit(expr_lit) => match &expr_lit.lit {
                    Lit::Int(lit_int) => lit_int.base10_parse::<usize>()? as f64,
                    Lit::Float(lit_float) => lit_float.base10_parse::<f64>()?,
                    other => {
                        return Err(syn::Error::new_spanned(
                            other,
                            "body limit must be a numeric literal",
                        ));
                    }
                },
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "body limit must be a numeric literal",
                    ));
                }
            };

            let unit = ident.to_string();
            let Some(multiplier) = unit_multiplier(&unit) else {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("unknown body_limit unit `{unit}` (expected {KNOWN_UNITS})"),
                ));
            };

            Ok(Self {
                limit_bytes: scale(num, multiplier, ident.span())?,
            })
        } else if input.peek(Lit) {
            // Literal form. Note that `512kb` and `1.5mb` are NOT identifiers:
            // Rust lexes them as an integer/float literal carrying the *suffix*
            // `kb`/`mb`, so the suffix has to be read off the literal here. It
            // used to be dropped, turning `512kb` into a 512-byte limit.
            let lit: Lit = input.parse()?;
            let limit_bytes = match lit {
                Lit::Int(lit_int) => {
                    let suffix = lit_int.suffix().to_string();
                    let Some(multiplier) = unit_multiplier(&suffix) else {
                        return Err(syn::Error::new(
                            lit_int.span(),
                            format!(
                                "unknown body_limit size suffix `{suffix}` (expected {KNOWN_UNITS})"
                            ),
                        ));
                    };
                    scale(
                        lit_int.base10_parse::<usize>()? as f64,
                        multiplier,
                        lit_int.span(),
                    )?
                }
                Lit::Float(lit_float) => {
                    let suffix = lit_float.suffix().to_string();
                    let Some(multiplier) = unit_multiplier(&suffix) else {
                        return Err(syn::Error::new(
                            lit_float.span(),
                            format!(
                                "unknown body_limit size suffix `{suffix}` (expected {KNOWN_UNITS})"
                            ),
                        ));
                    };
                    scale(
                        lit_float.base10_parse::<f64>()?,
                        multiplier,
                        lit_float.span(),
                    )?
                }
                Lit::Str(lit_str) => {
                    // String like "10mb", "1.5mb", "512kb".
                    let raw = lit_str.value();
                    parse_size_string(&raw).ok_or_else(|| {
                        syn::Error::new(
                            lit_str.span(),
                            format!("invalid body_limit size `{raw}` (expected a number optionally followed by {KNOWN_UNITS})"),
                        )
                    })?
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "body limit must be a number, a size literal like `512kb`, or a string like \"1.5mb\"",
                    ));
                }
            };
            Ok(Self { limit_bytes })
        } else if input.peek(syn::Ident) {
            // A bare identifier (no `=`, no digits) can never carry a size.
            let ident: syn::Ident = input.parse()?;
            Err(syn::Error::new(
                ident.span(),
                format!(
                    "invalid body_limit argument `{ident}` (expected a size like `512kb`, `mb = 5` or \"1.5mb\")"
                ),
            ))
        } else {
            // Default to 1MB
            Ok(Self {
                limit_bytes: 1024 * 1024,
            })
        }
    }
}

/// Parses a size string like "10mb", "512kb", "1gb" into bytes.
fn parse_size_string(s: &str) -> Option<usize> {
    let s = s.trim().to_lowercase();

    // Try parsing as just a number (bytes)
    if let Ok(bytes) = s.parse::<usize>() {
        return Some(bytes);
    }

    // Split the trailing alphabetic unit off the leading number, then reuse the
    // same unit table the literal/named forms use so all three agree.
    let split = s
        .char_indices()
        .find(|(_, c)| c.is_ascii_alphabetic())
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    let (num_str, unit) = s.split_at(split);

    let multiplier = unit_multiplier(unit)?;
    let num: f64 = num_str.trim().parse().ok()?;
    if !num.is_finite() || num < 0.0 {
        return None;
    }
    let bytes = num * multiplier as f64;
    if bytes > usize::MAX as f64 {
        return None;
    }
    Some(bytes as usize)
}

/// Implementation of the `#[body_limit(...)]` attribute macro.
///
/// This macro wraps a route handler function to apply a body size limit.
///
/// # Usage
///
/// ```ignore
/// use armature::{post, body_limit};
/// use armature_core::{HttpRequest, HttpResponse, Error};
///
/// // Limit in bytes
/// #[body_limit(1024)]
/// #[post("/small")]
/// async fn small_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
///
/// // Limit with unit suffix
/// #[body_limit("10mb")]
/// #[post("/upload")]
/// async fn upload_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
///
/// // Limit with named parameter
/// #[body_limit(mb = 5)]
/// #[post("/medium")]
/// async fn medium_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
///
/// // Various formats supported:
/// #[body_limit(512kb)]      // 512 kilobytes
/// #[body_limit(kb = 512)]   // 512 kilobytes
/// #[body_limit(1.5mb)]      // 1.5 megabytes
/// #[body_limit("1.5mb")]    // 1.5 megabytes
/// #[body_limit(1gb)]        // 1 gigabyte
/// ```
///
/// Units are `b`/`bytes`, `k`/`kb`/`kilobytes`, `m`/`mb`/`megabytes` and
/// `g`/`gb`/`gigabytes`. An unrecognized unit is a compile error.
///
/// # How It Works
///
/// The macro generates a wrapper function that:
/// 1. Checks the request body size against the specified limit
/// 2. Returns a 413 Payload Too Large error if the limit is exceeded
/// 3. Calls the original handler if the body is within limits
pub fn body_limit_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as BodyLimitArgs);
    let input = parse_macro_input!(item as ItemFn);

    let func_name = &input.sig.ident;
    let func_vis = &input.vis;
    let func_attrs = &input.attrs;
    let func_output = &input.sig.output;
    let func_body = &input.block;
    let func_inputs = &input.sig.inputs;
    let is_async = input.sig.asyncness.is_some();

    let limit_bytes = args.limit_bytes;

    // Format the limit for error messages
    let limit_display = if limit_bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", limit_bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if limit_bytes >= 1024 * 1024 {
        format!("{:.2} MB", limit_bytes as f64 / (1024.0 * 1024.0))
    } else if limit_bytes >= 1024 {
        format!("{:.2} KB", limit_bytes as f64 / 1024.0)
    } else {
        format!("{} bytes", limit_bytes)
    };

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
            const __BODY_LIMIT: usize = #limit_bytes;

            // Check body size
            if #req_ident.body.len() > __BODY_LIMIT {
                return Err(armature_core::Error::PayloadTooLarge(format!(
                    "Request body size ({} bytes) exceeds maximum allowed size ({})",
                    #req_ident.body.len(),
                    #limit_display
                )));
            }

            // Define the inner function with original body
            #async_marker fn #inner_fn_name(#func_inputs) #func_output
                #func_body

            #inner_fn_name(#req_ident).await
        }
    };

    TokenStream::from(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limit(src: &str) -> usize {
        syn::parse_str::<BodyLimitArgs>(src)
            .unwrap_or_else(|e| panic!("`{src}` must parse: {e}"))
            .limit_bytes
    }

    #[test]
    fn documented_forms_scale_by_their_unit() {
        // `512kb` / `1gb` / `1.5mb` are suffixed *literals*, not identifiers —
        // dropping the suffix used to turn these into 512, 1 and 1 MiB bytes.
        assert_eq!(limit("512kb"), 512 * 1024);
        assert_eq!(limit("1gb"), 1024 * 1024 * 1024);
        assert_eq!(limit("1.5mb"), 1024 * 1024 + 512 * 1024);
        assert_eq!(limit("\"10mb\""), 10 * 1024 * 1024);
        assert_eq!(limit("\"1.5mb\""), 1024 * 1024 + 512 * 1024);
        assert_eq!(limit("kb = 512"), 512 * 1024);
        assert_eq!(limit("mb = 5"), 5 * 1024 * 1024);
        assert_eq!(limit("bytes = 2048"), 2048);
        assert_eq!(limit("1024"), 1024);
        assert_eq!(limit(""), 1024 * 1024);
    }

    #[test]
    fn unknown_units_are_rejected() {
        for src in ["512tb", "tb = 5", "\"10zb\"", "\"nonsense\"", "big"] {
            assert!(
                syn::parse_str::<BodyLimitArgs>(src).is_err(),
                "`{src}` must be a compile error, not a silently wrong limit"
            );
        }
    }
}
