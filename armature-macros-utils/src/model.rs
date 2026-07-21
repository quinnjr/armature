use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Field, Fields, Ident, LitStr, parse_macro_input};

/// `#[derive(Model)]` — emit field-wise `Debug` + `Clone` impls plus a
/// `Default`-bounded `new()`.
///
/// Note: this cannot attach `Serialize`/`Deserialize` (a derive macro cannot
/// make the type derive *other* traits); callers add those derives themselves.
pub fn derive_model_impl(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let data = match &input.data {
        Data::Struct(s) => s,
        _ => {
            return syn::Error::new_spanned(
                &input.ident,
                "#[derive(Model)] can only be applied to structs",
            )
            .to_compile_error()
            .into();
        }
    };

    let (debug_body, clone_body) = match &data.fields {
        Fields::Named(named) => {
            let idents: Vec<&Ident> = named
                .named
                .iter()
                .map(|f| f.ident.as_ref().expect("named field has an ident"))
                .collect();
            let debug = quote! {
                f.debug_struct(stringify!(#name))
                    #(.field(stringify!(#idents), &self.#idents))*
                    .finish()
            };
            let clone = quote! {
                Self {
                    #(#idents: ::core::clone::Clone::clone(&self.#idents),)*
                }
            };
            (debug, clone)
        }
        Fields::Unnamed(unnamed) => {
            let indices: Vec<syn::Index> =
                (0..unnamed.unnamed.len()).map(syn::Index::from).collect();
            let debug = quote! {
                f.debug_tuple(stringify!(#name))
                    #(.field(&self.#indices))*
                    .finish()
            };
            let clone = quote! {
                Self(
                    #(::core::clone::Clone::clone(&self.#indices),)*
                )
            };
            (debug, clone)
        }
        Fields::Unit => (
            quote! { f.write_str(stringify!(#name)) },
            quote! { Self },
        ),
    };

    let expanded = quote! {
        impl #impl_generics ::core::fmt::Debug for #name #ty_generics #where_clause {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                #debug_body
            }
        }

        impl #impl_generics ::core::clone::Clone for #name #ty_generics #where_clause {
            fn clone(&self) -> Self {
                #clone_body
            }
        }

        impl #impl_generics #name #ty_generics #where_clause {
            /// Create a new instance using the type's `Default` implementation.
            ///
            /// Bounded on `Self: Default` — add `#[derive(Default)]` (or an
            /// explicit `Default` impl) to use it.
            #[allow(clippy::new_without_default)]
            pub fn new() -> Self
            where
                Self: ::core::default::Default,
            {
                <Self as ::core::default::Default>::default()
            }
        }
    };

    expanded.into()
}

/// Does this field carry `#[api(skip)]`?
fn has_api_skip(field: &Field) -> bool {
    let mut skip = false;
    for attr in &field.attrs {
        if attr.path().is_ident("api") {
            // Ignore parse errors from unrelated `api(..)` shapes.
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("skip") {
                    skip = true;
                }
                Ok(())
            });
        }
    }
    skip
}

/// Extract a field-level `#[serde(rename = "...")]` value, if present.
///
/// Handles both the plain `rename = "x"` shape and the split
/// `rename(serialize = "..", deserialize = "..")` form — for the latter, the
/// `serialize = ".."` value is used, since that's the key `serde_json::to_value`
/// (via `Serialize`) actually emits (`to_json` is serialize-side; `#[api(skip)]`
/// must counteract whatever key serialization produces, and `deserialize` never
/// affects that).
fn serde_field_rename(attrs: &[Attribute]) -> Option<String> {
    let mut renamed = None;
    for attr in attrs {
        if attr.path().is_ident("serde") {
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("rename") {
                    if meta.input.peek(syn::token::Paren) {
                        meta.parse_nested_meta(|nested| {
                            let value = nested.value()?;
                            let lit: LitStr = value.parse()?;
                            if nested.path.is_ident("serialize") {
                                renamed = Some(lit.value());
                            }
                            Ok(())
                        })?;
                    } else {
                        let value = meta.value()?;
                        let lit: LitStr = value.parse()?;
                        renamed = Some(lit.value());
                    }
                }
                Ok(())
            });
        }
    }
    renamed
}

/// Extract a struct-level `#[serde(rename_all = "...")]` value, if present.
///
/// Handles both the plain `rename_all = "x"` shape and the split
/// `rename_all(serialize = "..", deserialize = "..")` form (serde supports
/// this at the container level too), using the `serialize = ".."` value for
/// the same reason as `serde_field_rename`.
fn serde_container_rename_all(attrs: &[Attribute]) -> Option<String> {
    let mut style = None;
    for attr in attrs {
        if attr.path().is_ident("serde") {
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("rename_all") {
                    if meta.input.peek(syn::token::Paren) {
                        meta.parse_nested_meta(|nested| {
                            let value = nested.value()?;
                            let lit: LitStr = value.parse()?;
                            if nested.path.is_ident("serialize") {
                                style = Some(lit.value());
                            }
                            Ok(())
                        })?;
                    } else {
                        let value = meta.value()?;
                        let lit: LitStr = value.parse()?;
                        style = Some(lit.value());
                    }
                }
                Ok(())
            });
        }
    }
    style
}

/// Apply a serde `rename_all` casing style to a (conventionally snake_case)
/// Rust field ident, mirroring the naming conventions serde itself supports:
/// `lowercase`, `UPPERCASE`, `PascalCase`, `camelCase`, `snake_case`,
/// `SCREAMING_SNAKE_CASE`, `kebab-case`, `SCREAMING-KEBAB-CASE`.
///
/// An unrecognized style falls back to the plain ident rather than guessing,
/// so an unsupported/misspelled style never produces a bogus removal key.
fn apply_rename_all(ident: &str, style: &str) -> String {
    let words: Vec<&str> = ident.split('_').filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        return ident.to_string();
    }

    fn capitalize(word: &str) -> String {
        let mut chars = word.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    }

    match style {
        // serde's real `RenameRule::apply_to_field` operates on the whole
        // ident string for these two styles, PRESERVING underscores
        // (`lowercase` -> `ident.to_lowercase()`, a no-op for an
        // already-snake_case ident; `UPPERCASE` ->
        // `ident.to_ascii_uppercase()`). Concatenating the split words (the
        // old behavior) strips underscores and produces a removal key that
        // never matches the real wire key for multi-word fields.
        "lowercase" => ident.to_lowercase(),
        "UPPERCASE" => ident.to_ascii_uppercase(),
        "PascalCase" => words.iter().map(|w| capitalize(w)).collect(),
        "camelCase" => words
            .iter()
            .enumerate()
            .map(|(i, w)| {
                if i == 0 {
                    w.to_lowercase()
                } else {
                    capitalize(w)
                }
            })
            .collect(),
        "snake_case" => words.join("_").to_lowercase(),
        "SCREAMING_SNAKE_CASE" => words.join("_").to_uppercase(),
        "kebab-case" => words.join("-").to_lowercase(),
        "SCREAMING-KEBAB-CASE" => words.join("-").to_uppercase(),
        _ => ident.to_string(),
    }
}

/// Compute the effective serialized JSON key for a field: a field-level
/// `#[serde(rename = "..")]` wins; otherwise a struct-level
/// `#[serde(rename_all = "..")]` is applied to the ident; otherwise the Rust
/// ident itself is used. This must match what `Serialize` actually emits, or
/// `#[api(skip)]` silently fails to remove the field (see `to_json` below).
fn effective_json_key(field: &Field, container_rename_all: Option<&str>) -> String {
    let ident = field
        .ident
        .as_ref()
        .expect("effective_json_key called on a named field");
    if let Some(renamed) = serde_field_rename(&field.attrs) {
        return renamed;
    }
    if let Some(style) = container_rename_all {
        return apply_rename_all(&ident.to_string(), style);
    }
    ident.to_string()
}

/// `#[derive(ApiModel)]` — `to_json`/`from_json`, honoring `#[api(skip)]`.
pub fn derive_api_model_impl(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let container_rename_all = serde_container_rename_all(&input.attrs);

    let mut skip_fields: Vec<&Field> = Vec::new();
    if let Data::Struct(s) = &input.data
        && let Fields::Named(named) = &s.fields
    {
        for field in &named.named {
            if has_api_skip(field) && field.ident.is_some() {
                skip_fields.push(field);
            }
        }
    }

    // Remove by the *effective serialized key*, not the Rust ident: a field
    // that renames on the wire (via `#[serde(rename)]` or a container-level
    // `#[serde(rename_all)]`) would otherwise leak, because
    // `stringify!(ident)` never matches the key that actually lands in the
    // serialized map.
    let removals = skip_fields.iter().map(|field| {
        let key = effective_json_key(field, container_rename_all.as_deref());
        let key_lit = LitStr::new(&key, field.ident.as_ref().unwrap().span());
        quote! { __map.remove(#key_lit); }
    });

    let expanded = quote! {
        impl #impl_generics #name #ty_generics #where_clause {
            /// Convert to a JSON value, excluding any `#[api(skip)]` fields.
            pub fn to_json(&self) -> Result<::serde_json::Value, ::serde_json::Error> {
                let mut __value = ::serde_json::to_value(self)?;
                if let ::serde_json::Value::Object(ref mut __map) = __value {
                    #(#removals)*
                }
                Ok(__value)
            }

            /// Convert from a JSON value.
            pub fn from_json(value: &::serde_json::Value) -> Result<Self, ::serde_json::Error> {
                ::serde_json::from_value(value.clone())
            }
        }
    };

    expanded.into()
}

/// Convert a PascalCase struct name to snake_case for the default table name.
fn to_snake_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 4);
    for (i, ch) in input.char_indices() {
        if ch.is_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// `#[derive(Resource)]` — expose the configured table metadata.
///
/// Narrowed from the old "CRUD operations" claim: this parses
/// `#[resource(table = "..")]` and `#[resource(primary_key)]` and exposes
/// `table_name()` / `primary_key()`. No queries are generated.
pub fn derive_resource_impl(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    // Container attribute: #[resource(table = "..")]
    let mut table_name: Option<String> = None;
    for attr in &input.attrs {
        if attr.path().is_ident("resource") {
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("table") {
                    let value = meta.value()?;
                    let lit: LitStr = value.parse()?;
                    table_name = Some(lit.value());
                }
                Ok(())
            });
        }
    }

    // Field attribute: #[resource(primary_key)]
    let mut primary_key: Option<String> = None;
    if let Data::Struct(s) = &input.data
        && let Fields::Named(named) = &s.fields
    {
        for field in &named.named {
            for attr in &field.attrs {
                if attr.path().is_ident("resource") {
                    let _ = attr.parse_nested_meta(|meta| {
                        if meta.path.is_ident("primary_key")
                            && let Some(ident) = field.ident.as_ref()
                        {
                            primary_key = Some(ident.to_string());
                        }
                        Ok(())
                    });
                }
            }
        }
    }

    let table = table_name.unwrap_or_else(|| to_snake_case(&name.to_string()));
    let primary = primary_key.unwrap_or_else(|| "id".to_string());

    let table_lit = LitStr::new(&table, Span::call_site());
    let primary_lit = LitStr::new(&primary, Span::call_site());

    let expanded = quote! {
        impl #name {
            /// The configured table name, or the snake_case struct name if
            /// `#[resource(table = "..")]` was not provided.
            pub fn table_name() -> &'static str {
                #table_lit
            }

            /// The primary-key column name from `#[resource(primary_key)]`,
            /// defaulting to `"id"`.
            pub fn primary_key() -> &'static str {
                #primary_lit
            }
        }
    };

    expanded.into()
}
