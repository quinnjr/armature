use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{Expr, Ident, Token, parse::Parse, parse::ParseStream, parse_macro_input};

/// Build a `String`-producing token stream from a message + optional format
/// args.
///
/// * zero args  -> `fallback.to_string()`
/// * one  arg   -> `(arg).to_string()` (works for string literals *and*
///   `String`/`&str` expressions, unlike `format!`).
/// * many args  -> `format!(fmt, args...)`.
fn build_message(args: &[Expr], fallback: &str) -> TokenStream2 {
    match args {
        [] => quote! { #fallback.to_string() },
        [only] => quote! { (#only).to_string() },
        _ => quote! { format!(#(#args),*) },
    }
}

/// `bail!( [Kind ,] message [, format-args...] )`
struct BailArgs {
    kind: Ident,
    args: Vec<Expr>,
}

impl Parse for BailArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // A leading `Ident` followed by a comma is treated as the error kind.
        // A bare message (string literal, or a single expression) keeps the
        // default `BadRequest` kind.
        let kind = if input.peek(Ident) && input.peek2(Token![,]) {
            let k: Ident = input.parse()?;
            let _: Token![,] = input.parse()?;
            k
        } else {
            Ident::new("BadRequest", Span::call_site())
        };
        let args = Punctuated::<Expr, Token![,]>::parse_terminated(input)?
            .into_iter()
            .collect();
        Ok(BailArgs { kind, args })
    }
}

pub fn bail_impl(input: TokenStream) -> TokenStream {
    let BailArgs { kind, args } = parse_macro_input!(input as BailArgs);
    let message = build_message(&args, "");

    let expanded = quote! {
        return Err(armature_core::Error::#kind(#message))
    };

    expanded.into()
}

/// `ensure!( condition [, Kind] [, message [, format-args...]] )`
struct EnsureArgs {
    cond: Expr,
    kind: Ident,
    args: Vec<Expr>,
}

impl Parse for EnsureArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let cond: Expr = input.parse()?;

        // `ensure!(cond)` — condition only, generic message.
        if input.is_empty() {
            return Ok(EnsureArgs {
                cond,
                kind: Ident::new("BadRequest", Span::call_site()),
                args: Vec::new(),
            });
        }

        let _: Token![,] = input.parse()?;

        // Optional error kind before the message.
        let kind = if input.peek(Ident) && input.peek2(Token![,]) {
            let k: Ident = input.parse()?;
            let _: Token![,] = input.parse()?;
            k
        } else {
            Ident::new("BadRequest", Span::call_site())
        };

        let args = Punctuated::<Expr, Token![,]>::parse_terminated(input)?
            .into_iter()
            .collect();

        Ok(EnsureArgs { cond, kind, args })
    }
}

pub fn ensure_impl(input: TokenStream) -> TokenStream {
    let EnsureArgs { cond, kind, args } = parse_macro_input!(input as EnsureArgs);
    let message = build_message(&args, "Condition failed");

    let expanded = quote! {
        if !(#cond) {
            return Err(armature_core::Error::#kind(#message));
        }
    };

    expanded.into()
}
