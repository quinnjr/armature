use proc_macro::TokenStream;
use quote::quote;
use syn::{FnArg, Ident, ItemFn, LitStr, Pat, PatType, Type, parse_macro_input};

use crate::route_validation::validate_route_path;

/// Parameter extraction kind.
///
/// Note the split between *marker* kinds and *named* kinds. `Body`, `Query`,
/// `Headers` and `RawBody` take no argument and are markers only: the request
/// source is decided by the parameter's type through its
/// `armature_core::extractors::FromRequest` impl (`Body<T>` reads the JSON
/// body, `Query<T>` the query string, and so on). `verify_kind_matches_type`
/// rejects a marker paired with a wrapper type that reads something else, so
/// the annotation cannot lie about where the value comes from. The named kinds
/// (`BodyField`, `QueryField`, `Param`, `Header`) do carry their own extraction
/// semantics.
#[derive(Debug)]
pub(crate) enum ExtractorKind {
    /// #[body] - marker for a `Body<T>` parameter (entire JSON body)
    Body,
    /// #[body("field")] - Extract specific field from body
    BodyField(String),
    /// #[query] - marker for a `Query<T>` parameter (all query parameters)
    Query,
    /// #[query("field")] - Extract specific query parameter
    QueryField(String),
    /// #[param("name")] or #[path("name")] - Extract path parameter
    Param(String),
    /// #[header("name")] - Extract header value
    Header(String),
    /// #[headers] - marker for a `Headers` parameter (all headers)
    Headers,
    /// #[raw_body] - marker for a `RawBody` parameter (raw body bytes)
    RawBody,
    /// No extractor attribute - pass request directly
    Request,
}

/// The extractor attribute names recognized on a handler parameter.
const EXTRACTOR_ATTRS: [&str; 7] = [
    "body", "query", "param", "path", "header", "headers", "raw_body",
];

/// Does this parameter carry an extractor attribute?
///
/// Distinct from [`parse_extractor_attr`], which also reports `Request` for an
/// unannotated `HttpRequest` parameter.
pub(crate) fn has_extractor_attr(param: &PatType) -> bool {
    param.attrs.iter().any(|attr| {
        attr.path()
            .get_ident()
            .is_some_and(|ident| EXTRACTOR_ATTRS.contains(&ident.to_string().as_str()))
    })
}

/// Does any parameter of this signature carry an extractor attribute?
pub(crate) fn signature_has_extractors<'a>(inputs: impl IntoIterator<Item = &'a FnArg>) -> bool {
    inputs.into_iter().any(|arg| match arg {
        FnArg::Typed(pat_type) => has_extractor_attr(pat_type),
        FnArg::Receiver(_) => false,
    })
}

/// The `armature_core::extractors` wrapper types whose `FromRequest` impl is
/// what actually decides which part of the request a marker extractor reads.
fn known_wrapper(ty: &Type) -> Option<String> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let name = type_path.path.segments.last()?.ident.to_string();
    matches!(
        name.as_str(),
        "State"
            | "Body"
            | "Query"
            | "Path"
            | "PathParams"
            | "Header"
            | "Headers"
            | "RawBody"
            | "Form"
            | "ContentType"
    )
    .then_some(name)
}

/// Reject a marker extractor attached to a wrapper type that reads a different
/// part of the request (`#[body] filters: Query<Filters>` would silently read
/// the query string, because only the type is consulted at runtime).
///
/// Only *known* wrapper types are checked, so a type alias or a bespoke
/// `FromRequest` implementation is still accepted.
fn verify_kind_matches_type(kind: &ExtractorKind, ty: &Type) -> syn::Result<()> {
    let (attr, expected) = match kind {
        ExtractorKind::Body => ("#[body]", "Body"),
        ExtractorKind::Query => ("#[query]", "Query"),
        ExtractorKind::Headers => ("#[headers]", "Headers"),
        ExtractorKind::RawBody => ("#[raw_body]", "RawBody"),
        _ => return Ok(()),
    };

    match known_wrapper(ty) {
        Some(found) if found != expected => Err(syn::Error::new_spanned(
            ty,
            format!(
                "{attr} does not itself choose the request source — the parameter's type does, \
                 and `{found}` reads a different part of the request. Use `{expected}<..>` here, \
                 or annotate this parameter to match its type."
            ),
        )),
        _ => Ok(()),
    }
}

/// Parse extractor attributes from a function parameter
pub(crate) fn parse_extractor_attr(param: &PatType) -> Option<(ExtractorKind, Ident, Type)> {
    let param_name = match param.pat.as_ref() {
        Pat::Ident(pat_ident) => pat_ident.ident.clone(),
        _ => return None,
    };
    let param_type = (*param.ty).clone();

    for attr in &param.attrs {
        let path = attr.path();
        let ident = path.get_ident()?;
        let ident_str = ident.to_string();

        match ident_str.as_str() {
            "body" => {
                // Check if there's an argument: #[body("field")]
                if let Ok(field_name) = attr.parse_args::<LitStr>() {
                    return Some((
                        ExtractorKind::BodyField(field_name.value()),
                        param_name,
                        param_type,
                    ));
                }
                // No argument: #[body]
                return Some((ExtractorKind::Body, param_name, param_type));
            }
            "query" => {
                // Check if there's an argument: #[query("field")]
                if let Ok(field_name) = attr.parse_args::<LitStr>() {
                    return Some((
                        ExtractorKind::QueryField(field_name.value()),
                        param_name,
                        param_type,
                    ));
                }
                // No argument: #[query]
                return Some((ExtractorKind::Query, param_name, param_type));
            }
            "raw_body" => return Some((ExtractorKind::RawBody, param_name, param_type)),
            "headers" => return Some((ExtractorKind::Headers, param_name, param_type)),
            "param" | "path" => {
                // Parse #[param("name")] or #[path("name")]
                let name: LitStr = attr.parse_args().ok()?;
                return Some((ExtractorKind::Param(name.value()), param_name, param_type));
            }
            "header" => {
                // Parse #[header("name")]
                let name: LitStr = attr.parse_args().ok()?;
                return Some((ExtractorKind::Header(name.value()), param_name, param_type));
            }
            _ => continue,
        }
    }

    // Check if it's HttpRequest type (pass through)
    if let Type::Path(type_path) = &param_type
        && let Some(segment) = type_path.path.segments.last()
        && segment.ident == "HttpRequest"
    {
        return Some((ExtractorKind::Request, param_name, param_type));
    }

    None
}

/// Generate extraction code for a parameter.
///
/// Errors when a marker extractor contradicts the parameter's type — see
/// [`verify_kind_matches_type`].
pub(crate) fn generate_extraction(
    kind: &ExtractorKind,
    param_name: &Ident,
    param_type: &Type,
) -> syn::Result<proc_macro2::TokenStream> {
    verify_kind_matches_type(kind, param_type)?;

    Ok(match kind {
        ExtractorKind::Body => {
            quote! {
                let #param_name: #param_type = armature_core::extractors::FromRequest::from_request(&__request)?;
            }
        }
        ExtractorKind::BodyField(field) => {
            // Extract specific field from JSON body
            quote! {
                let #param_name: #param_type = {
                    let __body_json: serde_json::Value = __request.json()?;
                    let __field_value = __body_json.get(#field)
                        .ok_or_else(|| armature_core::Error::BadRequest(
                            format!("Missing field '{}' in request body", #field)
                        ))?;
                    serde_json::from_value(__field_value.clone())
                        .map_err(|e| armature_core::Error::BadRequest(
                            format!("Invalid type for field '{}': {}", #field, e)
                        ))?
                };
            }
        }
        ExtractorKind::Query => {
            quote! {
                let #param_name: #param_type = armature_core::extractors::FromRequest::from_request(&__request)?;
            }
        }
        ExtractorKind::QueryField(field) => {
            // Extract specific query parameter
            quote! {
                let #param_name: #param_type = {
                    let __query_value = __request.query_param(#field)
                        .ok_or_else(|| armature_core::Error::BadRequest(
                            format!("Missing query parameter '{}'", #field)
                        ))?;
                    __query_value.parse()
                        .map_err(|e| armature_core::Error::BadRequest(
                            format!("Invalid type for query parameter '{}': {}", #field, e)
                        ))?
                };
            }
        }
        ExtractorKind::RawBody => {
            quote! {
                let #param_name: #param_type = armature_core::extractors::FromRequest::from_request(&__request)?;
            }
        }
        ExtractorKind::Headers => {
            quote! {
                let #param_name: #param_type = armature_core::extractors::FromRequest::from_request(&__request)?;
            }
        }
        ExtractorKind::Param(name) => {
            quote! {
                let #param_name: #param_type = armature_core::extractors::FromRequestNamed::from_request(&__request, #name)?;
            }
        }
        ExtractorKind::Header(name) => {
            quote! {
                let #param_name: #param_type = armature_core::extractors::FromRequestNamed::from_request(&__request, #name)?;
            }
        }
        ExtractorKind::Request => {
            quote! {
                let #param_name: #param_type = __request.clone();
            }
        }
    })
}

pub fn route_impl(attr: TokenStream, item: TokenStream, method: &str) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let func_name = &input.sig.ident;
    let func_output = &input.sig.output;
    let func_body = &input.block;
    let func_vis = &input.vis;
    let func_attrs = &input.attrs;
    let is_async = input.sig.asyncness.is_some();

    let path = if attr.is_empty() {
        LitStr::new("", proc_macro2::Span::call_site())
    } else {
        parse_macro_input!(attr as LitStr)
    };
    let path_value = path.value();

    // Validate route path at compile time
    let validated = match validate_route_path(&path_value, path.span()) {
        Ok(v) => v,
        Err(e) => return e.to_compile_error().into(),
    };

    // Use the validated path verbatim (no rewriting: the router matches the
    // `:param` form directly).
    let path_value = validated.path;

    let route_const_name = syn::Ident::new(
        &format!(
            "__ROUTE_{}_{}",
            method,
            func_name.to_string().to_uppercase()
        ),
        func_name.span(),
    );

    // Check if there's a self receiver (&self, &mut self, self)
    let has_self_receiver = input
        .sig
        .inputs
        .first()
        .is_some_and(|arg| matches!(arg, FnArg::Receiver(_)));

    // Parse extractors from function parameters (skip receiver if present)
    let mut extractions = Vec::new();
    let mut has_extractors = false;

    for arg in &input.sig.inputs {
        if let FnArg::Typed(pat_type) = arg
            && let Some((kind, param_name, param_type)) = parse_extractor_attr(pat_type)
        {
            match generate_extraction(&kind, &param_name, &param_type) {
                Ok(extraction) => extractions.push(extraction),
                Err(e) => return e.to_compile_error().into(),
            }
            has_extractors = true;
        }
    }

    let async_marker = if is_async {
        quote! { async }
    } else {
        quote! {}
    };

    let expanded = if has_extractors {
        // Generate a wrapper function that extracts parameters. The original
        // parameter list is discarded entirely (the extractions re-introduce
        // every binding by name), so the extractor attributes go with it.
        let extraction_code = quote! { #(#extractions)* };

        // Preserve self receiver if present
        let self_param = if has_self_receiver {
            quote! { &self, }
        } else {
            quote! {}
        };

        quote! {
            #(#func_attrs)*
            #func_vis #async_marker fn #func_name(#self_param __request: armature_core::HttpRequest) #func_output {
                // Extract all decorated parameters
                #extraction_code

                // Execute the original function body
                #func_body
            }

            // Store route metadata as a constant
            pub const #route_const_name: (&'static str, &'static str) = (#method, #path_value);
        }
    } else {
        // No extractors - keep original signature
        let func_inputs = &input.sig.inputs;

        quote! {
            #(#func_attrs)*
            #func_vis #async_marker fn #func_name(#func_inputs) #func_output {
                #func_body
            }

            // Store route metadata as a constant
            pub const #route_const_name: (&'static str, &'static str) = (#method, #path_value);
        }
    };

    TokenStream::from(expanded)
}
