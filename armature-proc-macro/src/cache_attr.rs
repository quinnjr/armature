use proc_macro::TokenStream;
use quote::quote;
use syn::{
    FnArg, ItemFn, Lit, Meta, Pat, Token, parse::Parse, parse::ParseStream, parse_macro_input,
    punctuated::Punctuated,
};

/// Parsed `#[cache(...)]` options.
///
/// Supports `ttl = <seconds>`, `key = "<template>"` and one or more
/// `tag = "<tag>"` entries.
#[derive(Default)]
struct CacheArgs {
    ttl_secs: Option<u64>,
    key_template: Option<String>,
    tags: Vec<String>,
}

impl Parse for CacheArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = CacheArgs::default();

        if input.is_empty() {
            return Ok(args);
        }

        let items = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
        for item in items {
            let Meta::NameValue(nv) = item else {
                return Err(syn::Error::new_spanned(
                    item,
                    "expected `ttl = N`, `key = \"...\"` or `tag = \"...\"`",
                ));
            };
            let Some(ident) = nv.path.get_ident() else {
                continue;
            };
            match ident.to_string().as_str() {
                "ttl" => {
                    if let syn::Expr::Lit(expr_lit) = &nv.value
                        && let Lit::Int(lit_int) = &expr_lit.lit
                    {
                        args.ttl_secs = Some(lit_int.base10_parse()?);
                    } else {
                        return Err(syn::Error::new_spanned(
                            &nv.value,
                            "ttl must be an integer number of seconds",
                        ));
                    }
                }
                "key" => {
                    if let syn::Expr::Lit(expr_lit) = &nv.value
                        && let Lit::Str(lit_str) = &expr_lit.lit
                    {
                        args.key_template = Some(lit_str.value());
                    } else {
                        return Err(syn::Error::new_spanned(
                            &nv.value,
                            "key must be a string literal template",
                        ));
                    }
                }
                "tag" => {
                    if let syn::Expr::Lit(expr_lit) = &nv.value
                        && let Lit::Str(lit_str) = &expr_lit.lit
                    {
                        args.tags.push(lit_str.value());
                    } else {
                        return Err(syn::Error::new_spanned(
                            &nv.value,
                            "tag must be a string literal",
                        ));
                    }
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        &nv.path,
                        format!("unknown cache option `{other}` (expected ttl, key or tag)"),
                    ));
                }
            }
        }

        Ok(args)
    }
}

/// Collect the non-receiver parameter identifiers of a function.
fn arg_idents(input_fn: &ItemFn) -> Vec<syn::Ident> {
    input_fn
        .sig
        .inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Typed(pat_type) => match pat_type.pat.as_ref() {
                Pat::Ident(pat_ident) => Some(pat_ident.ident.clone()),
                _ => None,
            },
            FnArg::Receiver(_) => None,
        })
        .collect()
}

pub fn cache_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as CacheArgs);
    let input_fn = parse_macro_input!(item as ItemFn);

    let fn_name = &input_fn.sig.ident;
    let fn_vis = &input_fn.vis;
    let fn_sig = &input_fn.sig;
    let fn_block = &input_fn.block;
    let fn_attrs = &input_fn.attrs;

    let idents = arg_idents(&input_fn);
    let ttl_secs = args.ttl_secs.unwrap_or(3600);
    let tags = args.tags;

    // Build the expression that computes the cache key at runtime. When a custom
    // key template is provided, the argument values are formatted positionally
    // into it (`{}` placeholders). Otherwise the function name and a Debug tuple
    // of the argument values form the key, so distinct arguments never collide.
    let cache_key_expr = if let Some(template) = args.key_template {
        quote! { format!(#template, #(#idents),*) }
    } else {
        let default_template = format!("{fn_name}:{{:?}}");
        quote! { format!(#default_template, ( #( &#idents, )* )) }
    };

    let cache_code = if tags.is_empty() {
        // Simple cache without tags
        quote! {
            #(#fn_attrs)*
            #fn_vis #fn_sig {
                use std::time::Duration;

                // Generate cache key from the function arguments
                let cache_key = #cache_key_expr;

                // Try to get from cache. The stored value is the serialized
                // success payload, so deserialize into it and wrap in `Ok`.
                if let Ok(Some(cached)) = __cache.get_json(&cache_key).await {
                    if let Ok(value) = serde_json::from_str(&cached) {
                        return Ok(value);
                    }
                }

                // Execute function
                let result = (|| async #fn_block)().await;

                // Cache successful results
                if let Ok(ref success_result) = result {
                    if let Ok(json) = serde_json::to_string(success_result) {
                        let _ = __cache.set_json(
                            &cache_key,
                            json,
                            Some(Duration::from_secs(#ttl_secs))
                        ).await;
                    }
                }

                result
            }
        }
    } else {
        // Tagged cache
        let tag_literals = tags.iter().map(|t| t.as_str());
        quote! {
            #(#fn_attrs)*
            #fn_vis #fn_sig {
                use std::time::Duration;

                // Generate cache key from the function arguments
                let cache_key = #cache_key_expr;

                // Try to get from cache. The stored value is the serialized
                // success payload, so deserialize into it and wrap in `Ok`.
                if let Ok(Some(cached)) = __tagged_cache.get(&cache_key).await {
                    if let Ok(value) = serde_json::from_str(&cached) {
                        return Ok(value);
                    }
                }

                // Execute function
                let result = (|| async #fn_block)().await;

                // Cache successful results with tags
                if let Ok(ref success_result) = result {
                    if let Ok(json) = serde_json::to_string(success_result) {
                        let tags: &[&str] = &[ #( #tag_literals ),* ];
                        let _ = __tagged_cache.set_with_tags(
                            &cache_key,
                            json,
                            tags,
                            Some(Duration::from_secs(#ttl_secs))
                        ).await;
                    }
                }

                result
            }
        }
    };

    TokenStream::from(cache_code)
}
