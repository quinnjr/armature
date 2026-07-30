use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Error, Ident, ImplItem, ImplItemFn, ItemImpl, Type};

/// Which generated GraphQL root a tagged method belongs to.
///
/// The marker names below (`"query"`/`"mutation"`/`"subscription"`, see
/// [`Kind::marker`] and [`find_marker`]) are also independently recognized,
/// by source-text parsing rather than macro expansion, by
/// `armature-graphql`'s static SDL analyzer (`resolver_marker()` in
/// `armature-graphql/src/sdl_static.rs`). There is no compiler-enforced link
/// between the two lists — if a marker name here is ever renamed, or a new
/// one is added, that function needs the matching update too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Query,
    Mutation,
    Subscription,
}

impl Kind {
    fn marker(self) -> &'static str {
        match self {
            Kind::Query => "query",
            Kind::Mutation => "mutation",
            Kind::Subscription => "subscription",
        }
    }

    fn type_suffix(self) -> &'static str {
        match self {
            Kind::Query => "Query",
            Kind::Mutation => "Mutation",
            Kind::Subscription => "Subscription",
        }
    }

    fn async_graphql_macro(self) -> proc_macro2::TokenStream {
        match self {
            Kind::Query | Kind::Mutation => quote! { ::async_graphql::Object },
            Kind::Subscription => quote! { ::async_graphql::Subscription },
        }
    }
}

const ALL_KINDS: [Kind; 3] = [Kind::Query, Kind::Mutation, Kind::Subscription];

/// Find the `#[query]`/`#[mutation]`/`#[subscription]` marker on a method,
/// erroring if more than one is present. Returns `None` for methods without
/// any of the three markers — those are left untouched on the original
/// type.
///
/// See the note on [`Kind`] about `armature-graphql`'s `sdl_static.rs`
/// independently recognizing these same marker strings.
fn find_marker(method: &ImplItemFn) -> syn::Result<Option<Kind>> {
    let mut found = None;

    for attr in &method.attrs {
        let Some(ident) = attr.path().get_ident() else {
            continue;
        };
        let kind = match ident.to_string().as_str() {
            "query" => Kind::Query,
            "mutation" => Kind::Mutation,
            "subscription" => Kind::Subscription,
            _ => continue,
        };

        if !matches!(attr.meta, syn::Meta::Path(_)) {
            return Err(Error::new_spanned(
                attr,
                format!(
                    "#[{}] takes no arguments; use `async-graphql`'s own \
                     #[graphql(...)] attribute on the method or its \
                     parameters for field-level options",
                    kind.marker()
                ),
            ));
        }

        if let Some(existing) = found {
            let existing: Kind = existing;
            return Err(Error::new_spanned(
                attr,
                format!(
                    "method is already tagged #[{}]; a resolver method can \
                     only be one of #[query], #[mutation], or #[subscription]",
                    existing.marker()
                ),
            ));
        }

        found = Some(kind);
    }

    Ok(found)
}

fn strip_markers(method: &mut ImplItemFn) {
    method.attrs.retain(|attr| match attr.path().get_ident() {
        Some(ident) => !matches!(
            ident.to_string().as_str(),
            "query" | "mutation" | "subscription"
        ),
        None => true,
    });
}

/// Extract the plain, non-generic type name an `impl` block is for (e.g.
/// `Foo` out of `impl Foo { ... }`), rejecting the generic/qualified-path
/// shapes this macro doesn't support yet.
fn self_type_ident(self_ty: &Type) -> syn::Result<&Ident> {
    let Type::Path(type_path) = self_ty else {
        return Err(Error::new_spanned(
            self_ty,
            "#[resolver] only supports a plain `impl TypeName { ... }` block",
        ));
    };
    if type_path.qself.is_some() {
        return Err(Error::new_spanned(
            self_ty,
            "#[resolver] does not support qualified-path `impl` blocks",
        ));
    }
    let segment = type_path.path.segments.last().ok_or_else(|| {
        Error::new_spanned(self_ty, "#[resolver] requires a named type to implement")
    })?;
    if !segment.arguments.is_empty() {
        return Err(Error::new_spanned(
            self_ty,
            "#[resolver] does not support generic types yet; implement each \
             concrete instantiation separately",
        ));
    }
    Ok(&segment.ident)
}

pub fn resolver_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    match resolver_impl2(attr.into(), item.into()) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn resolver_impl2(
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> syn::Result<proc_macro2::TokenStream> {
    if !attr.is_empty() {
        return Err(Error::new_spanned(attr, "#[resolver] takes no arguments"));
    }

    let input: ItemImpl = syn::parse2(item)?;

    if let Some((_, path, _)) = &input.trait_ {
        return Err(Error::new_spanned(
            path,
            "#[resolver] must be applied to an inherent `impl TypeName { ... }` \
             block, not a trait impl",
        ));
    }

    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            "#[resolver] does not support generic `impl` blocks yet",
        ));
    }

    let self_ty = &input.self_ty;
    let struct_name = self_type_ident(self_ty)?.clone();

    let mut buckets: [Vec<ImplItemFn>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut remaining_items = Vec::new();

    for item in input.items {
        let ImplItem::Fn(mut method) = item else {
            remaining_items.push(item);
            continue;
        };

        let Some(kind) = find_marker(&method)? else {
            remaining_items.push(ImplItem::Fn(method));
            continue;
        };

        strip_markers(&mut method);
        buckets[kind as usize].push(method);
    }

    let attrs = &input.attrs;
    let unsafety = &input.unsafety;

    let mut generated = Vec::new();

    for kind in ALL_KINDS {
        let methods = &buckets[kind as usize];
        if methods.is_empty() {
            continue;
        }

        let wrapper_name = format_ident!("{}{}", struct_name, kind.type_suffix());
        let macro_path = kind.async_graphql_macro();

        generated.push(quote! {
            #[derive(Clone)]
            #[doc(hidden)]
            pub struct #wrapper_name(pub #struct_name);

            impl ::std::ops::Deref for #wrapper_name {
                type Target = #struct_name;

                fn deref(&self) -> &Self::Target {
                    &self.0
                }
            }

            impl ::std::convert::From<#struct_name> for #wrapper_name {
                fn from(value: #struct_name) -> Self {
                    Self(value)
                }
            }

            #[#macro_path]
            impl #wrapper_name {
                #(#methods)*
            }
        });
    }

    let expanded = quote! {
        #(#attrs)*
        #unsafety impl #self_ty {
            #(#remaining_items)*
        }

        #(#generated)*
    };

    Ok(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn find_marker_recognizes_each_kind() {
        let query: ImplItemFn = parse_quote! {
            #[query]
            async fn f(&self) -> i32 { 1 }
        };
        assert!(matches!(find_marker(&query).unwrap(), Some(Kind::Query)));

        let mutation: ImplItemFn = parse_quote! {
            #[mutation]
            async fn f(&self) -> i32 { 1 }
        };
        assert!(matches!(
            find_marker(&mutation).unwrap(),
            Some(Kind::Mutation)
        ));

        let subscription: ImplItemFn = parse_quote! {
            #[subscription]
            async fn f(&self) -> i32 { 1 }
        };
        assert!(matches!(
            find_marker(&subscription).unwrap(),
            Some(Kind::Subscription)
        ));
    }

    #[test]
    fn find_marker_returns_none_for_untagged_methods() {
        let plain: ImplItemFn = parse_quote! {
            fn helper(&self) -> i32 { 1 }
        };
        assert!(find_marker(&plain).unwrap().is_none());

        // Unrelated attributes (e.g. async-graphql's own field options) are
        // ignored, not mistaken for a marker.
        let with_graphql_attr: ImplItemFn = parse_quote! {
            #[graphql(name = "foo")]
            async fn f(&self) -> i32 { 1 }
        };
        assert!(find_marker(&with_graphql_attr).unwrap().is_none());
    }

    #[test]
    fn find_marker_rejects_more_than_one_tag() {
        let method: ImplItemFn = parse_quote! {
            #[query]
            #[mutation]
            async fn f(&self) -> i32 { 1 }
        };
        let err = find_marker(&method).unwrap_err();
        assert!(err.to_string().contains("already tagged"));
    }

    #[test]
    fn find_marker_rejects_arguments_on_the_marker() {
        let method: ImplItemFn = parse_quote! {
            #[query(name = "foo")]
            async fn f(&self) -> i32 { 1 }
        };
        let err = find_marker(&method).unwrap_err();
        assert!(err.to_string().contains("takes no arguments"));
    }

    #[test]
    fn strip_markers_removes_only_the_marker_attribute() {
        let mut method: ImplItemFn = parse_quote! {
            #[query]
            #[graphql(name = "foo")]
            async fn f(&self) -> i32 { 1 }
        };
        strip_markers(&mut method);
        assert_eq!(method.attrs.len(), 1);
        assert_eq!(
            method.attrs[0].path().get_ident().unwrap().to_string(),
            "graphql"
        );
    }

    #[test]
    fn self_type_ident_accepts_a_plain_named_type() {
        let ty: Type = parse_quote! { Foo };
        assert_eq!(self_type_ident(&ty).unwrap().to_string(), "Foo");
    }

    #[test]
    fn self_type_ident_rejects_generics() {
        let ty: Type = parse_quote! { Foo<T> };
        assert!(self_type_ident(&ty).is_err());
    }

    #[test]
    fn self_type_ident_rejects_non_path_types() {
        let ty: Type = parse_quote! { (Foo, Bar) };
        assert!(self_type_ident(&ty).is_err());
    }

    #[test]
    fn resolver_impl_rejects_trait_impls() {
        let attr = proc_macro2::TokenStream::new();
        let item = quote! {
            impl SomeTrait for Foo {}
        };
        let err = resolver_impl2(attr, item).unwrap_err();
        assert!(err.to_string().contains("inherent"));
    }

    #[test]
    fn resolver_impl_rejects_generic_impls() {
        let attr = proc_macro2::TokenStream::new();
        let item = quote! {
            impl<T> Foo<T> {}
        };
        let err = resolver_impl2(attr, item).unwrap_err();
        assert!(err.to_string().contains("generic"));
    }

    #[test]
    fn resolver_impl_rejects_attribute_arguments() {
        let attr = quote! { some_arg };
        let item = quote! {
            impl Foo {}
        };
        let err = resolver_impl2(attr, item).unwrap_err();
        assert!(err.to_string().contains("takes no arguments"));
    }

    #[test]
    fn resolver_impl_leaves_untagged_methods_on_the_original_type_only() {
        let attr = proc_macro2::TokenStream::new();
        let item = quote! {
            impl Foo {
                #[query]
                async fn a(&self) -> i32 { 1 }

                fn helper(&self) -> i32 { 2 }
            }
        };
        let out = resolver_impl2(attr, item).unwrap().to_string();
        assert!(out.contains("FooQuery"));
        assert!(!out.contains("FooMutation"));
        assert!(!out.contains("FooSubscription"));
        // The helper stays in the original `impl Foo` block only.
        assert_eq!(out.matches("helper").count(), 1);
    }
}
