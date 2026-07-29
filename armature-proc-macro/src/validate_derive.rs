// Derive macro for `#[derive(Validate)]` used by `armature-validation`.
//
// Generates an `armature_validation::Validate` implementation that runs the
// built-in validators against annotated fields and recurses into nested
// `#[validate]`-annotated struct fields.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Expr, Fields, LitInt, LitStr, parse_macro_input, spanned::Spanned};

/// A single validation check to emit for a field.
enum Check {
    Length {
        min: Option<Expr>,
        max: Option<Expr>,
    },
    Range {
        min: Option<Expr>,
        max: Option<Expr>,
    },
    Email,
    Url,
    Regex(String),
    Required,
    Custom(Expr),
    Nested,
}

pub fn validate_derive_impl(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => &named.named,
            _ => {
                return syn::Error::new(
                    input.span(),
                    "#[derive(Validate)] only supports structs with named fields",
                )
                .to_compile_error()
                .into();
            }
        },
        _ => {
            return syn::Error::new(
                input.span(),
                "#[derive(Validate)] can only be applied to structs",
            )
            .to_compile_error()
            .into();
        }
    };

    let mut field_checks: Vec<TokenStream2> = Vec::new();

    for field in fields {
        let field_ident = field.ident.as_ref().expect("named field");
        let field_name = field_ident.to_string();

        for attr in &field.attrs {
            if !attr.path().is_ident("validate") {
                continue;
            }

            let mut checks: Vec<Check> = Vec::new();

            // Bare `#[validate]` means "recurse into this nested struct".
            if matches!(attr.meta, syn::Meta::Path(_)) {
                checks.push(Check::Nested);
            } else {
                let parse_result = attr.parse_nested_meta(|meta| {
                    let ident = meta
                        .path
                        .get_ident()
                        .map(|i| i.to_string())
                        .unwrap_or_default();

                    match ident.as_str() {
                        "length" => {
                            let (min, max) = parse_min_max(&meta)?;
                            checks.push(Check::Length { min, max });
                            Ok(())
                        }
                        "range" => {
                            let (min, max) = parse_min_max(&meta)?;
                            checks.push(Check::Range { min, max });
                            Ok(())
                        }
                        "email" => {
                            checks.push(Check::Email);
                            Ok(())
                        }
                        "url" => {
                            checks.push(Check::Url);
                            Ok(())
                        }
                        "required" => {
                            checks.push(Check::Required);
                            Ok(())
                        }
                        "nested" => {
                            checks.push(Check::Nested);
                            Ok(())
                        }
                        "regex" => {
                            let pattern = parse_regex_pattern(&meta)?;
                            checks.push(Check::Regex(pattern));
                            Ok(())
                        }
                        "custom" => {
                            // `custom = "path"` or `custom(path)`
                            let expr: Expr = if meta.input.peek(syn::Token![=]) {
                                let lit: LitStr = meta.value()?.parse()?;
                                lit.parse()?
                            } else {
                                let content;
                                syn::parenthesized!(content in meta.input);
                                content.parse()?
                            };
                            checks.push(Check::Custom(expr));
                            Ok(())
                        }
                        other => Err(meta.error(format!("unknown validate rule `{}`", other))),
                    }
                });

                if let Err(e) = parse_result {
                    return e.to_compile_error().into();
                }
            }

            for check in checks {
                field_checks.push(emit_check(field_ident, &field_name, check));
            }
        }
    }

    let expanded = quote! {
        impl #impl_generics ::armature_validation::Validate for #name #ty_generics #where_clause {
            fn validate(&self) -> ::std::result::Result<(), ::std::vec::Vec<::armature_validation::ValidationError>> {
                let mut errors: ::std::vec::Vec<::armature_validation::ValidationError> = ::std::vec::Vec::new();
                #(#field_checks)*
                if errors.is_empty() {
                    ::std::result::Result::Ok(())
                } else {
                    ::std::result::Result::Err(errors)
                }
            }
        }
    };

    TokenStream::from(expanded)
}

/// Parse `min = <expr>, max = <expr>` (either optional) from a nested meta list.
fn parse_min_max(meta: &syn::meta::ParseNestedMeta) -> syn::Result<(Option<Expr>, Option<Expr>)> {
    let mut min = None;
    let mut max = None;
    meta.parse_nested_meta(|inner| {
        if inner.path.is_ident("min") {
            min = Some(inner.value()?.parse()?);
            Ok(())
        } else if inner.path.is_ident("max") {
            max = Some(inner.value()?.parse()?);
            Ok(())
        } else {
            Err(inner.error("expected `min` or `max`"))
        }
    })?;
    Ok((min, max))
}

/// Parse `regex(pattern = "...")`, `regex("...")`, or `regex = "..."`.
fn parse_regex_pattern(meta: &syn::meta::ParseNestedMeta) -> syn::Result<String> {
    if meta.input.peek(syn::Token![=]) {
        let lit: LitStr = meta.value()?.parse()?;
        return Ok(lit.value());
    }
    let content;
    syn::parenthesized!(content in meta.input);
    if content.peek(syn::Ident) {
        // pattern = "..."
        let _ident: syn::Ident = content.parse()?;
        let _eq: syn::Token![=] = content.parse()?;
    }
    let lit: LitStr = content.parse()?;
    Ok(lit.value())
}

fn emit_check(field_ident: &syn::Ident, field_name: &str, check: Check) -> TokenStream2 {
    match check {
        Check::Length { min, max } => {
            let min_check = min.map(|m| {
                let m = coerce_usize(&m);
                quote! {
                    if let ::std::result::Result::Err(e) =
                        ::armature_validation::MinLength(#m).validate(&self.#field_ident, #field_name)
                    {
                        errors.push(e);
                    }
                }
            });
            let max_check = max.map(|m| {
                let m = coerce_usize(&m);
                quote! {
                    if let ::std::result::Result::Err(e) =
                        ::armature_validation::MaxLength(#m).validate(&self.#field_ident, #field_name)
                    {
                        errors.push(e);
                    }
                }
            });
            quote! { #min_check #max_check }
        }
        Check::Range { min, max } => {
            let cond = match (&min, &max) {
                (Some(lo), Some(hi)) => quote! { !((#lo)..=(#hi)).contains(&__v) },
                (Some(lo), None) => quote! { __v < (#lo) },
                (None, Some(hi)) => quote! { __v > (#hi) },
                (None, None) => quote! { false },
            };
            let message = match (&min, &max) {
                (Some(_), Some(_)) => {
                    let min = min.as_ref().unwrap();
                    let max = max.as_ref().unwrap();
                    quote! { format!("{} must be between {} and {}", #field_name, #min, #max) }
                }
                (Some(_), None) => {
                    let min = min.as_ref().unwrap();
                    quote! { format!("{} must be at least {}", #field_name, #min) }
                }
                (None, Some(_)) => {
                    let max = max.as_ref().unwrap();
                    quote! { format!("{} must be at most {}", #field_name, #max) }
                }
                (None, None) => quote! { format!("{} is out of range", #field_name) },
            };
            quote! {
                {
                    let __v = self.#field_ident;
                    if #cond {
                        errors.push(
                            ::armature_validation::ValidationError::new(#field_name, #message)
                                .with_constraint("range")
                                .with_value(__v.to_string()),
                        );
                    }
                }
            }
        }
        Check::Email => quote! {
            if let ::std::result::Result::Err(e) =
                ::armature_validation::IsEmail::validate(&self.#field_ident, #field_name)
            {
                errors.push(e);
            }
        },
        Check::Url => quote! {
            if let ::std::result::Result::Err(e) =
                ::armature_validation::IsUrl::validate(&self.#field_ident, #field_name)
            {
                errors.push(e);
            }
        },
        Check::Regex(pattern) => quote! {
            match ::armature_validation::Matches::new(#pattern) {
                ::std::result::Result::Ok(__m) => {
                    if let ::std::result::Result::Err(e) = __m.validate(&self.#field_ident, #field_name) {
                        errors.push(e);
                    }
                }
                ::std::result::Result::Err(__err) => {
                    errors.push(
                        ::armature_validation::ValidationError::new(
                            #field_name,
                            format!("invalid regex pattern: {}", __err),
                        )
                        .with_constraint("regex"),
                    );
                }
            }
        },
        Check::Required => quote! {
            if let ::std::result::Result::Err(e) =
                ::armature_validation::NotEmpty::validate(&self.#field_ident, #field_name)
            {
                errors.push(e);
            }
        },
        Check::Custom(func) => quote! {
            if let ::std::result::Result::Err(e) = (#func)(&self.#field_ident) {
                errors.push(e);
            }
        },
        Check::Nested => quote! {
            if let ::std::result::Result::Err(mut __nested) =
                ::armature_validation::Validate::validate(&self.#field_ident)
            {
                errors.append(&mut __nested);
            }
        },
    }
}

/// Coerce a `min`/`max` length literal to `usize` for `MinLength`/`MaxLength`.
fn coerce_usize(expr: &Expr) -> TokenStream2 {
    if let Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Int(int),
        ..
    }) = expr
        && int.suffix().is_empty()
        && let Ok(lit) = syn::parse_str::<LitInt>(&format!("{}usize", int.base10_digits()))
    {
        return quote! { #lit };
    }
    quote! { (#expr) as usize }
}
