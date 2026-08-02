use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{Expr, Token, parse::Parser};

/// A parsed `[status,] value` argument list shared by the response macros.
struct StatusAndValue {
    /// The optional leading status argument (numeric literal, numeric
    /// expression, or one of the `ok`/`permanent`/`temporary` aliases).
    status: Option<Expr>,
    /// The payload expression (body / location).
    value: Expr,
}

/// Parse the `[status,] value` argument list.
///
/// * one argument  -> `value`, status defaulted by the caller.
/// * two arguments -> `status, value`.
fn parse_status_and_value(input: TokenStream) -> syn::Result<StatusAndValue> {
    let args = Punctuated::<Expr, Token![,]>::parse_terminated.parse(input)?;
    let mut it = args.into_iter();
    match (it.next(), it.next(), it.next()) {
        (Some(value), None, _) => Ok(StatusAndValue {
            status: None,
            value,
        }),
        (Some(status), Some(value), None) => Ok(StatusAndValue {
            status: Some(status),
            value,
        }),
        (None, _, _) => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "expected at least one argument",
        )),
        _ => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "expected at most two arguments: `status, value`",
        )),
    }
}

/// Turn a status argument into a `u16` token stream.
///
/// Recognizes the documented keyword aliases and otherwise casts the given
/// expression (a numeric literal or a `u16`-valued expression) to `u16`.
fn status_tokens(status: Option<&Expr>, default: u16) -> TokenStream2 {
    let Some(expr) = status else {
        return quote! { #default };
    };
    if let Expr::Path(path) = expr
        && let Some(ident) = path.path.get_ident()
    {
        match ident.to_string().as_str() {
            "ok" => return quote! { 200u16 },
            "permanent" => return quote! { 301u16 },
            "temporary" => return quote! { 302u16 },
            _ => {}
        }
    }
    quote! { (#expr) as u16 }
}

pub fn json_impl(input: TokenStream) -> TokenStream {
    let parsed = match parse_status_and_value(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    let status = status_tokens(parsed.status.as_ref(), 200);
    let value = &parsed.value;

    let expanded = quote! {
        {
            use armature_core::HttpResponse;
            HttpResponse::new(#status).with_json(&(#value))
        }
    };

    expanded.into()
}

pub fn html_impl(input: TokenStream) -> TokenStream {
    let parsed = match parse_status_and_value(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    let status = status_tokens(parsed.status.as_ref(), 200);
    let value = &parsed.value;

    let expanded = quote! {
        {
            use armature_core::HttpResponse;
            let mut response = HttpResponse::new(#status);
            response.body = armature_core::Bytes::from((#value).to_string().into_bytes());
            response.headers.insert(
                "Content-Type".to_string(),
                "text/html; charset=utf-8".to_string()
            );
            Ok::<_, armature_core::Error>(response)
        }
    };

    expanded.into()
}

pub fn text_impl(input: TokenStream) -> TokenStream {
    let parsed = match parse_status_and_value(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    let status = status_tokens(parsed.status.as_ref(), 200);
    let value = &parsed.value;

    let expanded = quote! {
        {
            use armature_core::HttpResponse;
            let mut response = HttpResponse::new(#status);
            response.body = armature_core::Bytes::from((#value).to_string().into_bytes());
            response.headers.insert(
                "Content-Type".to_string(),
                "text/plain; charset=utf-8".to_string()
            );
            Ok::<_, armature_core::Error>(response)
        }
    };

    expanded.into()
}

pub fn redirect_impl(input: TokenStream) -> TokenStream {
    let parsed = match parse_status_and_value(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    // Redirects default to 302 Found.
    let status = status_tokens(parsed.status.as_ref(), 302);
    let location = &parsed.value;

    let expanded = quote! {
        {
            use armature_core::HttpResponse;
            let mut response = HttpResponse::new(#status);
            response.headers.insert(
                "Location".to_string(),
                (#location).to_string()
            );
            Ok::<_, armature_core::Error>(response)
        }
    };

    expanded.into()
}
