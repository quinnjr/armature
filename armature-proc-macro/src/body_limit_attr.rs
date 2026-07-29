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

impl Parse for BodyLimitArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // Try to parse named parameter first (kb = X, mb = X, gb = X, bytes = X)
        if input.peek(syn::Ident) && input.peek2(Token![=]) {
            let ident: syn::Ident = input.parse()?;
            let _eq: Token![=] = input.parse()?;

            let value: Expr = input.parse()?;
            let num = match &value {
                Expr::Lit(expr_lit) => match &expr_lit.lit {
                    Lit::Int(lit_int) => lit_int.base10_parse::<usize>().unwrap_or(1024 * 1024),
                    Lit::Float(lit_float) => {
                        lit_float.base10_parse::<f64>().unwrap_or(1.0) as usize
                    }
                    _ => 1024 * 1024,
                },
                _ => 1024 * 1024,
            };

            let limit_bytes = match ident.to_string().as_str() {
                "bytes" | "b" => num,
                "kilobytes" | "kb" | "k" => num * 1024,
                "megabytes" | "mb" | "m" => num * 1024 * 1024,
                "gigabytes" | "gb" | "g" => num * 1024 * 1024 * 1024,
                _ => num, // Default to bytes
            };

            Ok(Self { limit_bytes })
        } else if input.peek(Lit) {
            // Parse as a literal (number or string like "10mb")
            let lit: Lit = input.parse()?;
            let limit_bytes = match lit {
                Lit::Int(lit_int) => {
                    // Just a number, assume bytes
                    lit_int.base10_parse::<usize>().unwrap_or(1024 * 1024)
                }
                Lit::Str(lit_str) => {
                    // String like "10mb", "512kb", etc.
                    parse_size_string(&lit_str.value()).unwrap_or(1024 * 1024)
                }
                _ => 1024 * 1024,
            };
            Ok(Self { limit_bytes })
        } else if input.peek(syn::Ident) {
            // Parse as identifier with optional suffix (e.g., 10mb, 512kb)
            let ident: syn::Ident = input.parse()?;
            let ident_str = ident.to_string();

            let limit_bytes = parse_size_string(&ident_str).unwrap_or(1024 * 1024);
            Ok(Self { limit_bytes })
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

    // Try parsing with unit suffix
    let (num_str, multiplier) = if s.ends_with("gb") {
        (&s[..s.len() - 2], 1024usize * 1024 * 1024)
    } else if s.ends_with("mb") {
        (&s[..s.len() - 2], 1024usize * 1024)
    } else if s.ends_with("kb") {
        (&s[..s.len() - 2], 1024usize)
    } else if s.ends_with('g') {
        (&s[..s.len() - 1], 1024usize * 1024 * 1024)
    } else if s.ends_with('m') {
        (&s[..s.len() - 1], 1024usize * 1024)
    } else if s.ends_with('k') {
        (&s[..s.len() - 1], 1024usize)
    } else if s.ends_with('b') {
        (&s[..s.len() - 1], 1usize)
    } else {
        return None;
    };

    let num: f64 = num_str.trim().parse().ok()?;
    Some((num * multiplier as f64) as usize)
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
/// #[body_limit("1.5mb")]    // 1.5 megabytes
/// #[body_limit(1gb)]        // 1 gigabyte
/// ```
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
