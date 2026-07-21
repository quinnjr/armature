use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{
    Expr, Ident, Token, braced, parse::Parse, parse::ParseStream, parse::Parser, parse_macro_input,
};

/// A single `"name": "value"` header entry inside the `headers: { .. }` block.
struct HeaderPair {
    name: Expr,
    value: Expr,
}

impl Parse for HeaderPair {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: Expr = input.parse()?;
        let _: Token![:] = input.parse()?;
        let value: Expr = input.parse()?;
        Ok(HeaderPair { name, value })
    }
}

/// `test_request!( METHOD "path" [, body] [, headers: { "k": "v", .. }] )`
struct TestRequest {
    method: Ident,
    path: Expr,
    body: Option<Expr>,
    headers: Vec<HeaderPair>,
}

impl Parse for TestRequest {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // `GET "/path"` — method ident directly followed by the path (no comma).
        let method: Ident = input.parse()?;
        let path: Expr = input.parse()?;

        let mut body = None;
        let mut headers = Vec::new();

        // Optional, comma-separated trailing segments: a body expression and/or
        // a `headers: { .. }` block, in any order.
        while input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
            if input.is_empty() {
                break; // tolerate a trailing comma
            }

            // `headers:` uses a single colon; guard against a `path::segment`
            // body expression whose `::` also begins with a colon.
            if input.peek(Ident) && input.peek2(Token![:]) && !input.peek2(Token![::]) {
                let key: Ident = input.parse()?;
                if key != "headers" {
                    return Err(syn::Error::new(
                        key.span(),
                        "expected `headers` or a body expression",
                    ));
                }
                let _: Token![:] = input.parse()?;
                let content;
                braced!(content in input);
                let pairs = Punctuated::<HeaderPair, Token![,]>::parse_terminated(&content)?;
                headers.extend(pairs);
            } else {
                if body.is_some() {
                    return Err(input.error("duplicate body expression in test_request!"));
                }
                body = Some(input.parse()?);
            }
        }

        Ok(TestRequest {
            method,
            path,
            body,
            headers,
        })
    }
}

pub fn test_request_impl(input: TokenStream) -> TokenStream {
    let TestRequest {
        method,
        path,
        body,
        headers,
    } = parse_macro_input!(input as TestRequest);

    let method_str = method.to_string();

    // Serialize the body as JSON (requires the caller to depend on serde_json)
    // and default the Content-Type; explicit headers below still override it.
    //
    // The internal binding is `__req` (not `req`) so a caller's body/header
    // expression that happens to reference its own local `req` cannot
    // capture this macro's binding.
    let body_setup = match &body {
        Some(body) => quote! {
            __req.set_body(
                ::serde_json::to_vec(&(#body))
                    .expect("test_request!: body value must serialize to JSON")
            );
            __req.headers.insert(
                "Content-Type".to_string(),
                "application/json".to_string()
            );
        },
        None => quote! {},
    };

    let header_inserts = headers.iter().map(|HeaderPair { name, value }| {
        quote! {
            __req.headers.insert((#name).to_string(), (#value).to_string());
        }
    });

    let expanded = quote! {
        {
            let mut __req = armature_core::HttpRequest::new(
                #method_str.to_string(),
                (#path).to_string(),
            );
            #body_setup
            #(#header_inserts)*
            __req
        }
    };

    expanded.into()
}

/// Map an `assert_status!` / status expectation into a `u16` token stream.
///
/// Recognizes the documented `ok` alias; otherwise casts the given expression
/// (numeric literal or `u16`-valued expression) to `u16`.
fn expected_status_tokens(expr: &Expr) -> TokenStream2 {
    if let Expr::Path(path) = expr
        && let Some(ident) = path.path.get_ident()
        && ident == "ok"
    {
        return quote! { 200u16 };
    }
    quote! { (#expr) as u16 }
}

pub fn assert_status_impl(input: TokenStream) -> TokenStream {
    let args = match Punctuated::<Expr, Token![,]>::parse_terminated
        .parse2(TokenStream2::from(input))
    {
        Ok(a) => a,
        Err(e) => return e.to_compile_error().into(),
    };
    let items: Vec<Expr> = args.into_iter().collect();
    let (response, expected) = match items.as_slice() {
        [response, expected] => (response, expected),
        _ => {
            return syn::Error::new(
                proc_macro2::Span::call_site(),
                "assert_status! expects exactly two arguments: `assert_status!(response, status)`",
            )
            .to_compile_error()
            .into();
        }
    };
    let expected = expected_status_tokens(expected);

    let expanded = quote! {
        {
            let __actual_status: u16 = (#response).status;
            let __expected_status: u16 = #expected;
            assert_eq!(
                __actual_status,
                __expected_status,
                "assert_status!: expected HTTP status {} but got {}",
                __expected_status,
                __actual_status
            );
        }
    };

    expanded.into()
}

/// `assert_json!( response , <json tokens> )`
///
/// The expected JSON is captured as raw tokens and wrapped in
/// `serde_json::json!`, so the object-literal form (`{ "id": 1 }`) and any
/// `Serialize` expression are both accepted.
struct AssertJson {
    response: Expr,
    expected: TokenStream2,
}

impl Parse for AssertJson {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let response: Expr = input.parse()?;
        let _: Token![,] = input.parse()?;
        let expected: TokenStream2 = input.parse()?;
        if expected.is_empty() {
            return Err(input.error("assert_json! expects an expected JSON value after the comma"));
        }
        Ok(AssertJson { response, expected })
    }
}

pub fn assert_json_impl(input: TokenStream) -> TokenStream {
    let AssertJson { response, expected } = parse_macro_input!(input as AssertJson);

    let expanded = quote! {
        {
            let __actual: serde_json::Value =
                serde_json::from_slice((#response).body_ref())
                    .expect("assert_json!: response body was not valid JSON");
            let __expected: serde_json::Value = serde_json::json!(#expected);
            assert_eq!(
                __actual,
                __expected,
                "assert_json!: response body did not match expected JSON"
            );
        }
    };

    expanded.into()
}
