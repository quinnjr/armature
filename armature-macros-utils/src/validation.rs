use proc_macro::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{Expr, Token, parse::Parser, parse_macro_input};

pub fn validate_impl(input: TokenStream) -> TokenStream {
    let args = match Punctuated::<Expr, Token![,]>::parse_terminated.parse(input) {
        Ok(a) => a,
        Err(e) => return e.to_compile_error().into(),
    };
    let items: Vec<Expr> = args.into_iter().collect();

    let expanded = match items.as_slice() {
        // `validate!(condition)` — bare condition, generic message.
        [cond] => quote! {
            {
                if !(#cond) {
                    return Err(armature_core::Error::Validation(
                        "Validation failed".to_string()
                    ));
                }
            }
        },
        // `validate!(condition, "message")` — condition with a custom message.
        [cond, message] => quote! {
            {
                if !(#cond) {
                    return Err(armature_core::Error::Validation((#message).to_string()));
                }
            }
        },
        // `validate!(value, validator_fn, "message")` — custom validator called
        // with a reference to `value`.
        [value, validator, message] => quote! {
            {
                if !#validator(&(#value)) {
                    return Err(armature_core::Error::Validation((#message).to_string()));
                }
            }
        },
        _ => syn::Error::new(
            proc_macro2::Span::call_site(),
            "validate! expects 1 to 3 arguments: \
             `validate!(cond)`, `validate!(cond, msg)`, or `validate!(value, fn, msg)`",
        )
        .to_compile_error(),
    };

    expanded.into()
}

pub fn validate_required_impl(input: TokenStream) -> TokenStream {
    let fields = match Punctuated::<Expr, Token![,]>::parse_terminated.parse(input) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error().into(),
    };

    if fields.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "validate_required! expects at least one field",
        )
        .to_compile_error()
        .into();
    }

    // Emit one `is_none()` guard per field, naming the missing field.
    let checks = fields.iter().map(|field| {
        quote! {
            if (#field).is_none() {
                return Err(armature_core::Error::Validation(
                    format!("Required field '{}' is missing", stringify!(#field))
                ));
            }
        }
    });

    let expanded = quote! {
        {
            #(#checks)*
        }
    };

    expanded.into()
}

pub fn validate_email_impl(input: TokenStream) -> TokenStream {
    let expr = parse_macro_input!(input as Expr);

    // Compile the regex exactly once via a `static LazyLock` instead of
    // rebuilding it on every call. Callers must depend on the `regex` crate.
    let expanded = quote! {
        {
            static EMAIL_REGEX: ::std::sync::LazyLock<regex::Regex> =
                ::std::sync::LazyLock::new(|| {
                    regex::Regex::new(
                        r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$"
                    ).expect("validate_email!: built-in email regex is valid")
                });
            let email = #expr;
            if !EMAIL_REGEX.is_match(&email) {
                return Err(armature_core::Error::Validation(
                    "Invalid email format".to_string()
                ));
            }
        }
    };

    expanded.into()
}
