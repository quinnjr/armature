//! MCP tool proc macro implementation
//!
//! Provides the `#[mcp]` attribute macro for registering MCP tools.

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input, Ident, ItemFn, LitStr, Token, Type,
};

/// Arguments for the #[mcp] attribute
struct McpArgs {
    name: Option<String>,
    description: Option<String>,
    owner: Option<Type>,
}

impl Parse for McpArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut description = None;
        let mut owner = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match key.to_string().as_str() {
                "name" => {
                    let value: LitStr = input.parse()?;
                    name = Some(value.value());
                }
                "description" => {
                    let value: LitStr = input.parse()?;
                    description = Some(value.value());
                }
                "owner" => {
                    let value: Type = input.parse()?;
                    owner = Some(value);
                }
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("Unknown attribute: {}", key),
                    ));
                }
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(McpArgs {
            name,
            description,
            owner,
        })
    }
}

/// Extract the input type from a function's first parameter
fn extract_input_type(func: &ItemFn) -> Option<Type> {
    func.sig.inputs.iter().find_map(|arg| {
        if let syn::FnArg::Typed(pat_type) = arg {
            Some((*pat_type.ty).clone())
        } else {
            None
        }
    })
}

/// Generate JSON schema for a type (simplified)
fn generate_schema_for_type(ty: &Type) -> String {
    let type_str = quote!(#ty).to_string().replace(' ', "");

    match type_str.as_str() {
        "String" | "&str" => r#"{"type": "string"}"#.to_string(),
        "i32" | "i64" | "u32" | "u64" | "isize" | "usize" => {
            r#"{"type": "integer"}"#.to_string()
        }
        "f32" | "f64" => r#"{"type": "number"}"#.to_string(),
        "bool" => r#"{"type": "boolean"}"#.to_string(),
        "()" => r#"{"type": "object"}"#.to_string(),
        "Value" | "serde_json::Value" => r#"{"type": "object"}"#.to_string(),
        _ => {
            // For custom types, assume they implement JsonSchema or use object
            r#"{"type": "object", "additionalProperties": true}"#.to_string()
        }
    }
}

pub fn mcp_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as McpArgs);
    let input_fn = parse_macro_input!(item as ItemFn);

    let fn_name = &input_fn.sig.ident;
    let fn_name_str = fn_name.to_string();

    // Use provided name or function name
    let tool_name = args.name.unwrap_or_else(|| fn_name_str.clone());

    // Use provided description or generate from function name
    let description = args.description.unwrap_or_else(|| {
        format!(
            "{}",
            fn_name_str.replace('_', " ").trim()
        )
    });

    // Check if owner was provided
    let has_owner = args.owner.is_some();

    // Get owner type or use a generated one
    let owner_type = args.owner.map(|t| quote!(#t)).unwrap_or_else(|| {
        let owner_name = syn::Ident::new(
            &format!("__McpTool_{}", fn_name_str),
            proc_macro2::Span::call_site(),
        );
        quote!(#owner_name)
    });

    // Generate owner struct if not provided
    let owner_struct = if !has_owner {
        let owner_name = syn::Ident::new(
            &format!("__McpTool_{}", fn_name_str),
            proc_macro2::Span::call_site(),
        );
        quote! {
            #[doc(hidden)]
            struct #owner_name;
        }
    } else {
        quote!()
    };

    // Extract input type for schema generation
    let input_type = extract_input_type(&input_fn);
    let schema = input_type
        .as_ref()
        .map(generate_schema_for_type)
        .unwrap_or_else(|| r#"{"type": "object"}"#.to_string());

    // Generate the wrapper handler
    let wrapper_name = syn::Ident::new(
        &format!("__mcp_handler_{}", fn_name_str),
        proc_macro2::Span::call_site(),
    );

    // Check if the function takes an argument
    let handler_impl = if input_type.is_some() {
        let input_ty = input_type.unwrap();
        quote! {
            async fn #wrapper_name(args: ::serde_json::Value) -> ::armature_mcp::Result<::armature_mcp::ToolCallResult> {
                let input: #input_ty = ::serde_json::from_value(args)
                    .map_err(|e| ::armature_mcp::McpError::InvalidParams(e.to_string()))?;
                let result = #fn_name(input).await;
                Ok(result)
            }
        }
    } else {
        quote! {
            async fn #wrapper_name(args: ::serde_json::Value) -> ::armature_mcp::Result<::armature_mcp::ToolCallResult> {
                let _ = args; // Unused
                let result = #fn_name().await;
                Ok(result)
            }
        }
    };

    // Generate the tool registration
    let registration = quote! {
        ::armature_mcp::inventory::submit! {
            ::armature_mcp::McpToolEntry::new::<#owner_type>(
                #tool_name,
                Some(#description),
                #schema,
                ::std::sync::Arc::new(move |args| {
                    Box::pin(#wrapper_name(args))
                }),
            )
        }
    };

    let expanded = quote! {
        #owner_struct

        #input_fn

        #handler_impl

        #registration
    };

    TokenStream::from(expanded)
}

/// MCP resource attribute implementation
pub fn mcp_resource_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as McpResourceArgs);
    let input_fn = parse_macro_input!(item as ItemFn);

    let fn_name = &input_fn.sig.ident;
    let fn_name_str = fn_name.to_string();

    let uri = args.uri;
    let name = args.name.unwrap_or_else(|| fn_name_str.clone());
    let description = args.description;
    let mime_type = args.mime_type;

    // Check if owner was provided
    let has_owner = args.owner.is_some();

    let owner_type = args.owner.map(|t| quote!(#t)).unwrap_or_else(|| {
        let owner_name = syn::Ident::new(
            &format!("__McpResource_{}", fn_name_str),
            proc_macro2::Span::call_site(),
        );
        quote!(#owner_name)
    });

    let owner_struct = if !has_owner {
        let owner_name = syn::Ident::new(
            &format!("__McpResource_{}", fn_name_str),
            proc_macro2::Span::call_site(),
        );
        quote! {
            #[doc(hidden)]
            struct #owner_name;
        }
    } else {
        quote!()
    };

    let desc_tokens = description
        .map(|d| quote!(Some(#d)))
        .unwrap_or_else(|| quote!(None));

    let mime_tokens = mime_type
        .map(|m| quote!(Some(#m)))
        .unwrap_or_else(|| quote!(None));

    let wrapper_name = syn::Ident::new(
        &format!("__mcp_resource_handler_{}", fn_name_str),
        proc_macro2::Span::call_site(),
    );

    let expanded = quote! {
        #owner_struct

        #input_fn

        async fn #wrapper_name() -> ::armature_mcp::Result<::armature_mcp::ResourceContent> {
            let result = #fn_name().await;
            Ok(result)
        }

        ::armature_mcp::inventory::submit! {
            ::armature_mcp::McpResourceEntry::new::<#owner_type>(
                #uri,
                #name,
                #desc_tokens,
                #mime_tokens,
                ::std::sync::Arc::new(move || {
                    Box::pin(#wrapper_name())
                }),
            )
        }
    };

    TokenStream::from(expanded)
}

/// Arguments for the #[mcp_resource] attribute
struct McpResourceArgs {
    uri: String,
    name: Option<String>,
    description: Option<String>,
    mime_type: Option<String>,
    owner: Option<Type>,
}

impl Parse for McpResourceArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut uri = None;
        let mut name = None;
        let mut description = None;
        let mut mime_type = None;
        let mut owner = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match key.to_string().as_str() {
                "uri" => {
                    let value: LitStr = input.parse()?;
                    uri = Some(value.value());
                }
                "name" => {
                    let value: LitStr = input.parse()?;
                    name = Some(value.value());
                }
                "description" => {
                    let value: LitStr = input.parse()?;
                    description = Some(value.value());
                }
                "mime_type" => {
                    let value: LitStr = input.parse()?;
                    mime_type = Some(value.value());
                }
                "owner" => {
                    let value: Type = input.parse()?;
                    owner = Some(value);
                }
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("Unknown attribute: {}", key),
                    ));
                }
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        let uri = uri.ok_or_else(|| {
            syn::Error::new(proc_macro2::Span::call_site(), "uri is required")
        })?;

        Ok(McpResourceArgs {
            uri,
            name,
            description,
            mime_type,
            owner,
        })
    }
}
