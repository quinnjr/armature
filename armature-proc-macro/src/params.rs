use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

/// Derive macro for extracting request body
///
/// Implements `FromRequest` for the type, allowing it to be extracted from
/// the request body as JSON.
///
/// # Example
///
/// ```rust,ignore
/// use armature::prelude::*;
///
/// #[derive(Body, Deserialize)]
/// struct CreateUser {
///     name: String,
///     email: String,
/// }
///
/// // In a handler
/// let user: CreateUser = body!(request, CreateUser)?;
/// // Or using the extractor
/// let body: Body<CreateUser> = Body::from_request(&request)?;
/// ```
pub fn body_derive_impl(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let expanded = quote! {
        impl #impl_generics #name #ty_generics #where_clause {
            /// Extract this type from the request body as JSON
            pub fn from_request(request: &armature_core::HttpRequest) -> Result<Self, armature_core::Error> {
                request.json::<Self>()
            }

            /// Extract this type from the request body as JSON, returning Option
            pub fn from_request_opt(request: &armature_core::HttpRequest) -> Option<Self> {
                request.json::<Self>().ok()
            }
        }

        impl #impl_generics armature_core::extractors::FromRequest for #name #ty_generics #where_clause {
            fn from_request(request: &armature_core::HttpRequest) -> Result<Self, armature_core::Error> {
                request.json::<Self>()
            }
        }
    };

    TokenStream::from(expanded)
}

/// Derive macro for extracting path parameters
///
/// Implements parsing from a string for single-value types, or from multiple
/// path parameters for structs.
///
/// # Example
///
/// ```rust,ignore
/// // For single values (implementing FromStr)
/// #[derive(Param)]
/// struct UserId(u32);
///
/// // Usage: let id: UserId = path!(request, "id", UserId)?;
///
/// // For structs with multiple path params
/// #[derive(Param, Deserialize)]
/// struct UserPostParams {
///     user_id: u32,
///     post_id: u32,
/// }
///
/// // Usage for /users/:user_id/posts/:post_id
/// let params: UserPostParams = PathParams::from_request(&request)?;
/// ```
pub fn param_derive_impl(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    // Choose the mode by shape: structs with named fields use multi-field
    // extraction; everything else uses the single-value `FromStr` mode. The two
    // are mutually exclusive because the single-value methods carry a
    // `where Self: FromStr` bound, which is a rejected trivial bound on a
    // concrete multi-field struct that does not implement `FromStr`.
    let is_named_struct = matches!(
        &input.data,
        Data::Struct(data) if matches!(data.fields, Fields::Named(_))
    );

    // Single-value mode: the `FromStr` bound on the methods means it is only
    // usable for types that implement `FromStr`.
    let single_value = quote! {
        impl #impl_generics #name #ty_generics #where_clause {
            /// Extract a path parameter by name
            pub fn from_param(request: &armature_core::HttpRequest, param_name: &str) -> Result<Self, armature_core::Error>
            where
                Self: std::str::FromStr,
                <Self as std::str::FromStr>::Err: std::fmt::Display,
            {
                let value = request.param(param_name)
                    .ok_or_else(|| armature_core::Error::Validation(format!("Missing parameter: {}", param_name)))?;

                value.parse::<Self>()
                    .map_err(|e| armature_core::Error::Validation(format!("Invalid parameter '{}': {}", param_name, e)))
            }

            /// Extract a path parameter by name, returning Option
            pub fn from_param_opt(request: &armature_core::HttpRequest, param_name: &str) -> Option<Self>
            where
                Self: std::str::FromStr,
            {
                request.param(param_name)
                    .and_then(|v| v.parse::<Self>().ok())
            }
        }
    };

    // Multi-field mode: for structs with named fields, extract each field from
    // the same-named path parameter via `FromStr`.
    let multi_field = match &input.data {
        Data::Struct(data) if is_named_struct => match &data.fields {
            Fields::Named(fields) => {
                let extractions = fields.named.iter().map(|f| {
                    let field_ident = f.ident.as_ref().expect("named field has an ident");
                    let field_name = field_ident.to_string();
                    let field_ty = &f.ty;
                    quote! {
                        #field_ident: {
                            let __raw = request.param(#field_name)
                                .ok_or_else(|| armature_core::Error::Validation(
                                    format!("Missing parameter: {}", #field_name)
                                ))?;
                            __raw.parse::<#field_ty>()
                                .map_err(|e| armature_core::Error::Validation(
                                    format!("Invalid parameter '{}': {}", #field_name, e)
                                ))?
                        }
                    }
                });

                quote! {
                    impl #impl_generics #name #ty_generics #where_clause {
                        /// Extract this struct from path parameters. Each named field is
                        /// parsed from the same-named path parameter via `FromStr`.
                        pub fn from_request(request: &armature_core::HttpRequest) -> Result<Self, armature_core::Error> {
                            Ok(Self {
                                #(#extractions),*
                            })
                        }

                        /// Like [`from_request`](Self::from_request) but returns `None` on any failure.
                        pub fn from_request_opt(request: &armature_core::HttpRequest) -> Option<Self> {
                            Self::from_request(request).ok()
                        }
                    }
                }
            }
            _ => quote! {},
        },
        _ => quote! {},
    };

    let expanded = if is_named_struct {
        multi_field
    } else {
        single_value
    };

    TokenStream::from(expanded)
}

/// Derive macro for extracting query parameters
///
/// Implements `FromRequest` for the type, allowing it to be extracted from
/// URL query parameters.
///
/// # Example
///
/// ```rust,ignore
/// use armature::prelude::*;
///
/// #[derive(Query, Deserialize)]
/// struct UserFilters {
///     page: Option<u32>,
///     limit: Option<u32>,
///     sort: Option<String>,
///     order: Option<String>,
/// }
///
/// // In a handler
/// let filters: UserFilters = query!(request, UserFilters)?;
/// // Or using the extractor
/// let query: Query<UserFilters> = Query::from_request(&request)?;
/// ```
pub fn query_derive_impl(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let expanded = quote! {
        impl #impl_generics #name #ty_generics #where_clause {
            /// Extract this type from query parameters
            pub fn from_query(request: &armature_core::HttpRequest) -> Result<Self, armature_core::Error>
            where
                Self: serde::de::DeserializeOwned,
            {
                // The raw query string, not the decoded pairs: re-encoding a
                // decoded pair cannot always reproduce what the client sent.
                serde_urlencoded::from_str(request.query_string().unwrap_or(""))
                    .map_err(|e| armature_core::Error::Validation(format!("Invalid query parameters: {}", e)))
            }

            /// Extract this type from query parameters, returning Option
            pub fn from_query_opt(request: &armature_core::HttpRequest) -> Option<Self>
            where
                Self: serde::de::DeserializeOwned,
            {
                Self::from_query(request).ok()
            }
        }

        impl #impl_generics armature_core::extractors::FromRequest for #name #ty_generics #where_clause
        where
            Self: serde::de::DeserializeOwned,
        {
            fn from_request(request: &armature_core::HttpRequest) -> Result<Self, armature_core::Error> {
                Self::from_query(request)
            }
        }
    };

    TokenStream::from(expanded)
}
