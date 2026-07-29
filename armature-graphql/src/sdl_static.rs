//! Static SDL generation from async-graphql resolver source.
//!
//! Unlike introspection-over-HTTP, this analyzes Rust source text directly
//! (via `syn`) and renders SDL from the parsed AST — no running server, no
//! network, no compiling the target crate. It recognizes the standard
//! async-graphql macros:
//!
//! - `#[derive(SimpleObject)]` / `#[derive(InputObject)]` struct fields
//! - `#[Object]` / `#[Subscription]` / `#[ComplexObject]` impl-block methods
//! - `#[derive(Enum)]` variants
//! - `#[derive(Union)]` newtype-variant enums
//! - `#[derive(Interface)]` enums with `#[graphql(field(name = "..", type = ".."))]`
//! - `#[Scalar]` impl blocks
//!
//! This is necessarily a subset of what the real macros support (they run
//! as full proc-macros with type information this pass doesn't have) — the
//! goal is a fast, offline approximation good enough for docs/tooling, not
//! a reimplementation of async-graphql's macro expansion.

use std::collections::HashMap;
use std::fmt;
use syn::{Fields, FnArg, ImplItem, Item, ItemEnum, ItemImpl, ItemStruct, Pat, ReturnType, Type};

#[derive(Debug)]
pub enum SdlError {
    Parse(String),
}

impl fmt::Display for SdlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SdlError::Parse(e) => write!(f, "failed to parse Rust source: {e}"),
        }
    }
}

impl std::error::Error for SdlError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeKind {
    Object,
    Input,
    Enum,
    Union,
    Interface,
    Scalar,
}

#[derive(Debug, Clone, Default)]
pub struct Deprecation {
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Arg {
    pub name: String,
    pub type_str: String,
    pub default: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub description: Option<String>,
    pub args: Vec<Arg>,
    pub type_str: String,
    pub deprecated: Option<Deprecation>,
}

#[derive(Debug, Clone)]
pub struct EnumValue {
    pub name: String,
    pub description: Option<String>,
    pub deprecated: Option<Deprecation>,
}

#[derive(Debug, Clone)]
pub struct TypeDef {
    pub kind: TypeKind,
    pub name: String,
    pub description: Option<String>,
    pub fields: Vec<Field>,
    pub enum_values: Vec<EnumValue>,
    pub union_members: Vec<String>,
}

impl TypeDef {
    fn new(kind: TypeKind, name: String) -> Self {
        Self {
            kind,
            name,
            description: None,
            fields: Vec::new(),
            enum_values: Vec::new(),
            union_members: Vec::new(),
        }
    }
}

/// Accumulates type definitions discovered across one or more source files,
/// merging same-named types (e.g. a `#[derive(SimpleObject)]` struct's
/// fields plus a later `#[ComplexObject]` impl block's extra fields).
#[derive(Debug, Default)]
pub struct SchemaBuilder {
    order: Vec<String>,
    defs: HashMap<String, TypeDef>,
}

impl SchemaBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse one Rust source file's text and merge any discovered
    /// GraphQL type definitions into this builder.
    pub fn add_source(&mut self, source: &str) -> Result<(), SdlError> {
        let file = syn::parse_file(source).map_err(|e| SdlError::Parse(e.to_string()))?;
        self.visit_items(&file.items);
        Ok(())
    }

    fn visit_items(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Struct(s) => self.visit_struct(s),
                Item::Enum(e) => self.visit_enum(e),
                Item::Impl(i) => self.visit_impl(i),
                Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        self.visit_items(inner);
                    }
                }
                _ => {}
            }
        }
    }

    fn entry(&mut self, kind: TypeKind, name: &str) -> &mut TypeDef {
        if !self.defs.contains_key(name) {
            self.order.push(name.to_string());
            self.defs
                .insert(name.to_string(), TypeDef::new(kind, name.to_string()));
        }
        self.defs.get_mut(name).unwrap()
    }

    fn visit_struct(&mut self, s: &ItemStruct) {
        let derives = derive_names(&s.attrs);
        let name = s.ident.to_string();

        if derives.iter().any(|d| d == "SimpleObject") {
            let def = self.entry(TypeKind::Object, &name);
            merge_description(def, doc_string(&s.attrs));
            append_struct_fields(def, &s.fields, false);
        }

        if derives.iter().any(|d| d == "InputObject") {
            let def = self.entry(TypeKind::Input, &name);
            merge_description(def, doc_string(&s.attrs));
            append_struct_fields(def, &s.fields, true);
        }
    }

    fn visit_enum(&mut self, e: &ItemEnum) {
        let derives = derive_names(&e.attrs);
        let name = e.ident.to_string();

        if derives.iter().any(|d| d == "Enum") {
            let def = self.entry(TypeKind::Enum, &name);
            merge_description(def, doc_string(&e.attrs));
            for variant in &e.variants {
                let vattrs = graphql_attrs(&variant.attrs);
                if vattrs.skip {
                    continue;
                }
                let vname = vattrs
                    .name
                    .clone()
                    .unwrap_or_else(|| to_screaming_snake(&variant.ident.to_string()));
                def.enum_values.push(EnumValue {
                    name: vname,
                    description: doc_string(&variant.attrs),
                    deprecated: vattrs.deprecation,
                });
            }
        }

        if derives.iter().any(|d| d == "Union") {
            let def = self.entry(TypeKind::Union, &name);
            merge_description(def, doc_string(&e.attrs));
            for variant in &e.variants {
                if let Fields::Unnamed(fields) = &variant.fields
                    && let Some(f) = fields.unnamed.first()
                {
                    let member = type_name_only(&f.ty);
                    if !def.union_members.contains(&member) {
                        def.union_members.push(member);
                    }
                }
            }
        }

        if derives.iter().any(|d| d == "Interface") {
            let def = self.entry(TypeKind::Interface, &name);
            merge_description(def, doc_string(&e.attrs));
            for attr in &e.attrs {
                if attr.path().is_ident("graphql") {
                    collect_interface_fields(attr, &mut def.fields);
                }
            }
        }
    }

    fn visit_impl(&mut self, imp: &ItemImpl) {
        let is_object = has_attr_ident(&imp.attrs, "Object");
        let is_subscription = has_attr_ident(&imp.attrs, "Subscription");
        let is_complex_object = has_attr_ident(&imp.attrs, "ComplexObject");
        let is_scalar = has_attr_ident(&imp.attrs, "Scalar");

        if !(is_object || is_subscription || is_complex_object || is_scalar) {
            return;
        }

        let Some(name) = self_type_name(imp) else {
            return;
        };

        if is_scalar {
            self.entry(TypeKind::Scalar, &name);
            return;
        }

        let def = self.entry(TypeKind::Object, &name);
        for item in &imp.items {
            if let ImplItem::Fn(m) = item {
                let attrs = graphql_attrs(&m.attrs);
                if attrs.skip {
                    continue;
                }
                let Some(ret_ty) = return_type(&m.sig.output) else {
                    continue;
                };
                let field_name = attrs
                    .name
                    .clone()
                    .unwrap_or_else(|| to_camel_case(&m.sig.ident.to_string()));

                let mut args = Vec::new();
                for input in &m.sig.inputs {
                    let FnArg::Typed(pat_type) = input else {
                        continue;
                    };
                    if type_mentions_context(&pat_type.ty) {
                        continue;
                    }
                    let arg_attrs = graphql_attrs(&pat_type.attrs);
                    if arg_attrs.skip {
                        continue;
                    }
                    let Pat::Ident(pat_ident) = pat_type.pat.as_ref() else {
                        continue;
                    };
                    let arg_name = arg_attrs
                        .name
                        .clone()
                        .unwrap_or_else(|| to_camel_case(&pat_ident.ident.to_string()));
                    args.push(Arg {
                        name: arg_name,
                        type_str: graphql_type(&pat_type.ty),
                        default: arg_attrs.default,
                    });
                }

                def.fields.push(Field {
                    name: field_name,
                    description: doc_string(&m.attrs),
                    args,
                    type_str: graphql_type(ret_ty),
                    deprecated: attrs.deprecation,
                });
            }
        }
    }

    /// Render all discovered types as SDL text. If any of `query_root`,
    /// `mutation_root`, `subscription_root` names a type that was actually
    /// discovered, a `schema { ... }` block naming the ones that were found
    /// is emitted first.
    pub fn render(&self, query_root: &str, mutation_root: &str, subscription_root: &str) -> String {
        let mut out = String::new();

        let roots: Vec<(&str, &str)> = [
            ("query", query_root),
            ("mutation", mutation_root),
            ("subscription", subscription_root),
        ]
        .into_iter()
        .filter(|(_, name)| self.defs.contains_key(*name))
        .collect();

        if !roots.is_empty() {
            out.push_str("schema {\n");
            for (op, name) in &roots {
                out.push_str(&format!("  {op}: {name}\n"));
            }
            out.push_str("}\n\n");
        }

        for name in &self.order {
            let def = &self.defs[name];
            render_type(def, &mut out);
            out.push('\n');
        }

        // Trim the trailing extra blank line while keeping a single
        // newline at end of file.
        while out.ends_with("\n\n") {
            out.pop();
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

fn render_type(def: &TypeDef, out: &mut String) {
    render_description(&def.description, out, "");

    match def.kind {
        TypeKind::Object => {
            out.push_str(&format!("type {} {{\n", def.name));
            for f in &def.fields {
                render_field(f, out);
            }
            out.push_str("}\n");
        }
        TypeKind::Input => {
            out.push_str(&format!("input {} {{\n", def.name));
            for f in &def.fields {
                out.push_str("  ");
                if let Some(desc) = &f.description {
                    out.push_str(&format!("\"{}\" ", escape_desc(desc)));
                }
                out.push_str(&format!("{}: {}\n", f.name, f.type_str));
            }
            out.push_str("}\n");
        }
        TypeKind::Interface => {
            out.push_str(&format!("interface {} {{\n", def.name));
            for f in &def.fields {
                render_field(f, out);
            }
            out.push_str("}\n");
        }
        TypeKind::Enum => {
            out.push_str(&format!("enum {} {{\n", def.name));
            for v in &def.enum_values {
                out.push_str("  ");
                if let Some(desc) = &v.description {
                    out.push_str(&format!("\"{}\" ", escape_desc(desc)));
                }
                out.push_str(&v.name);
                push_deprecation(&v.deprecated, out);
                out.push('\n');
            }
            out.push_str("}\n");
        }
        TypeKind::Union => {
            out.push_str(&format!(
                "union {} = {}\n",
                def.name,
                def.union_members.join(" | ")
            ));
        }
        TypeKind::Scalar => {
            out.push_str(&format!("scalar {}\n", def.name));
        }
    }
}

fn render_field(f: &Field, out: &mut String) {
    out.push_str("  ");
    if let Some(desc) = &f.description {
        out.push_str(&format!("\"{}\" ", escape_desc(desc)));
    }
    out.push_str(&f.name);
    if !f.args.is_empty() {
        let rendered_args: Vec<String> = f
            .args
            .iter()
            .map(|a| match &a.default {
                Some(d) => format!("{}: {} = {}", a.name, a.type_str, d),
                None => format!("{}: {}", a.name, a.type_str),
            })
            .collect();
        out.push_str(&format!("({})", rendered_args.join(", ")));
    }
    out.push_str(&format!(": {}", f.type_str));
    push_deprecation(&f.deprecated, out);
    out.push('\n');
}

fn push_deprecation(dep: &Option<Deprecation>, out: &mut String) {
    if let Some(d) = dep {
        match &d.reason {
            Some(reason) => out.push_str(&format!(" @deprecated(reason: \"{reason}\")")),
            None => out.push_str(" @deprecated"),
        }
    }
}

fn render_description(desc: &Option<String>, out: &mut String, indent: &str) {
    if let Some(desc) = desc {
        if desc.contains('\n') {
            out.push_str(&format!("{indent}\"\"\"\n{indent}{desc}\n{indent}\"\"\"\n"));
        } else {
            out.push_str(&format!("{indent}\"{}\"\n", escape_desc(desc)));
        }
    }
}

fn escape_desc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn merge_description(def: &mut TypeDef, doc: Option<String>) {
    if def.description.is_none() {
        def.description = doc;
    }
}

fn append_struct_fields(def: &mut TypeDef, fields: &Fields, is_input: bool) {
    let Fields::Named(named) = fields else {
        return;
    };
    for f in &named.named {
        let attrs = graphql_attrs(&f.attrs);
        if attrs.skip {
            continue;
        }
        let Some(ident) = &f.ident else { continue };
        let name = attrs
            .name
            .clone()
            .unwrap_or_else(|| to_camel_case(&ident.to_string()));
        def.fields.push(Field {
            name,
            description: doc_string(&f.attrs),
            args: Vec::new(),
            type_str: graphql_type(&f.ty),
            deprecated: if is_input { None } else { attrs.deprecation },
        });
    }
}

fn collect_interface_fields(attr: &syn::Attribute, fields: &mut Vec<Field>) {
    let _ = attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("field") {
            let mut name = None;
            let mut type_str = None;
            meta.parse_nested_meta(|inner| {
                if inner.path.is_ident("name") {
                    let value = inner.value()?;
                    let lit: syn::LitStr = value.parse()?;
                    name = Some(lit.value());
                } else if inner.path.is_ident("type") {
                    let value = inner.value()?;
                    let lit: syn::LitStr = value.parse()?;
                    type_str = Some(lit.value());
                } else {
                    let _ = inner.value().and_then(|v| v.parse::<syn::Lit>());
                }
                Ok(())
            })?;
            if let (Some(name), Some(type_str)) = (name, type_str) {
                fields.push(Field {
                    name: to_camel_case(&name),
                    description: None,
                    args: Vec::new(),
                    type_str,
                    deprecated: None,
                });
            }
        }
        Ok(())
    });
}

#[derive(Debug, Default)]
struct GraphqlAttrs {
    name: Option<String>,
    skip: bool,
    default: Option<String>,
    deprecation: Option<Deprecation>,
}

fn graphql_attrs(attrs: &[syn::Attribute]) -> GraphqlAttrs {
    let mut result = GraphqlAttrs::default();

    for attr in attrs {
        if attr.path().is_ident("deprecated") {
            result.deprecation = Some(Deprecation { reason: None });
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("note") {
                    let value = meta.value()?;
                    let lit: syn::LitStr = value.parse()?;
                    result.deprecation = Some(Deprecation {
                        reason: Some(lit.value()),
                    });
                }
                Ok(())
            });
            continue;
        }

        if !attr.path().is_ident("graphql") {
            continue;
        }

        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                result.skip = true;
            } else if meta.path.is_ident("name") {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                result.name = Some(lit.value());
            } else if meta.path.is_ident("default") {
                if meta.input.peek(syn::Token![=]) {
                    let value = meta.value()?;
                    if let Ok(lit) = value.parse::<syn::Lit>() {
                        result.default = Some(literal_to_sdl(&lit));
                    }
                } else {
                    result.default = Some("null".to_string());
                }
            } else if meta.path.is_ident("deprecation") {
                if meta.input.peek(syn::Token![=]) {
                    let value = meta.value()?;
                    let lit: syn::LitStr = value.parse()?;
                    result.deprecation = Some(Deprecation {
                        reason: Some(lit.value()),
                    });
                } else {
                    result.deprecation = Some(Deprecation { reason: None });
                }
            } else {
                // Unrecognized nested meta (e.g. `field(...)` on interfaces,
                // handled separately) — consume its value if any so parsing
                // of subsequent items doesn't fail.
                let _ = meta.value().and_then(|v| v.parse::<syn::Lit>());
            }
            Ok(())
        });
    }

    result
}

fn literal_to_sdl(lit: &syn::Lit) -> String {
    match lit {
        syn::Lit::Str(s) => format!("\"{}\"", escape_desc(&s.value())),
        syn::Lit::Int(i) => i.base10_digits().to_string(),
        syn::Lit::Float(f) => f.base10_digits().to_string(),
        syn::Lit::Bool(b) => b.value.to_string(),
        _ => "null".to_string(),
    }
}

fn derive_names(attrs: &[syn::Attribute]) -> Vec<String> {
    let mut names = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("derive") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if let Some(ident) = meta.path.get_ident() {
                names.push(ident.to_string());
            }
            Ok(())
        });
    }
    names
}

fn has_attr_ident(attrs: &[syn::Attribute], ident: &str) -> bool {
    attrs.iter().any(|a| a.path().is_ident(ident))
}

fn doc_string(attrs: &[syn::Attribute]) -> Option<String> {
    let mut lines = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        if let syn::Meta::NameValue(nv) = &attr.meta
            && let syn::Expr::Lit(expr_lit) = &nv.value
            && let syn::Lit::Str(s) = &expr_lit.lit
        {
            lines.push(s.value().trim().to_string());
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n").trim().to_string())
    }
}

fn self_type_name(imp: &ItemImpl) -> Option<String> {
    if let Type::Path(p) = imp.self_ty.as_ref() {
        p.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}

fn return_type(output: &ReturnType) -> Option<&Type> {
    match output {
        ReturnType::Type(_, ty) => Some(ty),
        ReturnType::Default => None,
    }
}

fn type_mentions_context(ty: &Type) -> bool {
    let mut ty = ty;
    if let Type::Reference(r) = ty {
        ty = &r.elem;
    }
    if let Type::Path(p) = ty {
        return p.path.segments.last().is_some_and(|s| s.ident == "Context");
    }
    false
}

/// The bare type name of a single-generic-arg-free path type, e.g. the `T`
/// in a union variant's newtype wrapper `Variant(T)`. Strips references and
/// smart-pointer wrappers but does not attempt full `graphql_type` mapping
/// since union members must be concrete object type names, never lists or
/// scalars.
fn type_name_only(ty: &Type) -> String {
    match ty {
        Type::Reference(r) => type_name_only(&r.elem),
        Type::Path(p) => p
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_else(|| "Unknown".to_string()),
        _ => "Unknown".to_string(),
    }
}

fn first_generic_type(seg: &syn::PathSegment) -> Option<&Type> {
    if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
        args.args.iter().find_map(|a| {
            if let syn::GenericArgument::Type(t) = a {
                Some(t)
            } else {
                None
            }
        })
    } else {
        None
    }
}

/// Maps a Rust type to a GraphQL type string, including nullability (`!`)
/// and list (`[...]`) wrapping. `Option<T>` becomes nullable; everything
/// else is non-null. `Vec<T>`/`[T]` becomes a list. `Box`/`Arc`/`Rc` and
/// `Result<T, E>` (as a resolver return type — the `Ok` value is the field
/// type) are transparently unwrapped.
pub fn graphql_type(ty: &Type) -> String {
    if let Type::Path(p) = ty
        && let Some(seg) = p.path.segments.last()
        && seg.ident == "Option"
        && let Some(inner) = first_generic_type(seg)
    {
        return render_non_null(inner);
    }
    format!("{}!", render_non_null(ty))
}

fn render_non_null(ty: &Type) -> String {
    match ty {
        Type::Reference(r) => render_non_null(&r.elem),
        Type::Slice(s) => format!("[{}]", graphql_type(&s.elem)),
        Type::Path(p) => {
            let Some(seg) = p.path.segments.last() else {
                return "Unknown".to_string();
            };
            match seg.ident.to_string().as_str() {
                "Box" | "Arc" | "Rc" | "RefCell" | "Result" => first_generic_type(seg)
                    .map(graphql_type_stripped)
                    .unwrap_or_else(|| "Unknown".to_string()),
                "Vec" | "VecDeque" | "HashSet" | "BTreeSet" => first_generic_type(seg)
                    .map(|inner| format!("[{}]", graphql_type(inner)))
                    .unwrap_or_else(|| "[Unknown]".to_string()),
                "String" | "str" | "Cow" => "String".to_string(),
                "bool" => "Boolean".to_string(),
                "f32" | "f64" => "Float".to_string(),
                "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64"
                | "u128" | "usize" => "Int".to_string(),
                "ID" => "ID".to_string(),
                other => other.to_string(),
            }
        }
        _ => "Unknown".to_string(),
    }
}

/// Like `render_non_null`, but for types that are themselves already being
/// unwrapped from a transparent wrapper (`Box<T>`, `Result<T, E>`) — the
/// wrapper contributes no nullability of its own, so the inner type's
/// *own* `Option`-ness (or lack of it) is what determines the `!`.
fn graphql_type_stripped(ty: &Type) -> String {
    graphql_type(ty)
}

fn to_camel_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut capitalize_next = false;
    for (i, ch) in s.chars().enumerate() {
        if ch == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            out.extend(ch.to_uppercase());
            capitalize_next = false;
        } else if i == 0 {
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn to_screaming_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let chars: Vec<char> = s.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if ch.is_uppercase() && i > 0 {
            let prev_lower = chars[i - 1].is_lowercase() || chars[i - 1].is_ascii_digit();
            let next_lower = chars.get(i + 1).map(|c| c.is_lowercase()).unwrap_or(false);
            if prev_lower || (next_lower && chars[i - 1].is_uppercase()) {
                out.push('_');
            }
        }
        out.extend(ch.to_uppercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(source: &str) -> String {
        let mut builder = SchemaBuilder::new();
        builder.add_source(source).expect("valid source");
        builder.render("Query", "Mutation", "Subscription")
    }

    #[test]
    fn simple_object_fields() {
        let sdl = render(
            r#"
            /// A user account.
            #[derive(SimpleObject)]
            struct User {
                /// The user's id.
                id: ID,
                name: String,
                nickname: Option<String>,
                friends: Vec<User>,
            }
            "#,
        );
        assert!(sdl.contains("\"A user account.\"\ntype User {"));
        assert!(sdl.contains("\"The user's id.\" id: ID!"));
        assert!(sdl.contains("name: String!"));
        assert!(sdl.contains("nickname: String"));
        assert!(!sdl.contains("nickname: String!"));
        assert!(sdl.contains("friends: [User!]!"));
    }

    #[test]
    fn simple_object_field_rename_and_skip() {
        let sdl = render(
            r#"
            #[derive(SimpleObject)]
            struct User {
                #[graphql(name = "userId")]
                id: ID,
                #[graphql(skip)]
                internal_secret: String,
            }
            "#,
        );
        assert!(sdl.contains("userId: ID!"));
        assert!(!sdl.contains("internal_secret"));
    }

    #[test]
    fn object_impl_methods_become_fields_with_args() {
        let sdl = render(
            r#"
            struct Query;

            #[Object]
            impl Query {
                async fn user(&self, ctx: &Context<'_>, id: ID) -> Option<User> {
                    None
                }

                async fn users(&self, limit: Option<i32>) -> Vec<User> {
                    vec![]
                }

                fn helper(&self) {}
            }
            "#,
        );
        assert!(sdl.contains("user(id: ID!): User\n"));
        assert!(sdl.contains("users(limit: Int): [User!]!\n"));
        assert!(!sdl.contains("helper"));
    }

    #[test]
    fn method_name_and_field_name_camel_cased() {
        let sdl = render(
            r#"
            struct Query;

            #[Object]
            impl Query {
                async fn get_all_users(&self) -> Vec<User> {
                    vec![]
                }
            }
            "#,
        );
        assert!(sdl.contains("getAllUsers: [User!]!"));
    }

    #[test]
    fn result_return_type_unwraps_to_ok_value() {
        let sdl = render(
            r#"
            struct Query;

            #[Object]
            impl Query {
                async fn user(&self) -> Result<User> {
                    unimplemented!()
                }
            }
            "#,
        );
        assert!(sdl.contains("user: User!"));
    }

    #[test]
    fn input_object_fields() {
        let sdl = render(
            r#"
            #[derive(InputObject)]
            struct NewUser {
                name: String,
                age: Option<i32>,
            }
            "#,
        );
        assert!(sdl.contains("input NewUser {"));
        assert!(sdl.contains("name: String!"));
        assert!(sdl.contains("age: Int"));
        assert!(!sdl.contains("age: Int!"));
    }

    #[test]
    fn enum_variants_become_screaming_snake_case() {
        let sdl = render(
            r#"
            #[derive(Enum, Clone, Copy, Eq, PartialEq)]
            enum Status {
                Active,
                PendingReview,
                #[graphql(name = "ARCHIVED_LEGACY")]
                Archived,
            }
            "#,
        );
        assert!(sdl.contains("enum Status {"));
        assert!(sdl.contains("ACTIVE"));
        assert!(sdl.contains("PENDING_REVIEW"));
        assert!(sdl.contains("ARCHIVED_LEGACY"));
    }

    #[test]
    fn union_members_from_newtype_variants() {
        let sdl = render(
            r#"
            #[derive(Union)]
            enum SearchResult {
                UserResult(User),
                PostResult(Post),
            }
            "#,
        );
        assert!(sdl.contains("union SearchResult = User | Post"));
    }

    #[test]
    fn scalar_impl_block() {
        let sdl = render(
            r#"
            #[Scalar]
            impl ScalarType for JSON {
                fn parse(_value: Value) -> InputValueResult<Self> {
                    unimplemented!()
                }
            }
            "#,
        );
        assert!(sdl.contains("scalar JSON"));
    }

    #[test]
    fn deprecated_field() {
        let sdl = render(
            r#"
            #[derive(SimpleObject)]
            struct User {
                #[graphql(deprecation = "use fullName instead")]
                name: String,
            }
            "#,
        );
        assert!(sdl.contains("name: String! @deprecated(reason: \"use fullName instead\")"));
    }

    #[test]
    fn schema_block_only_includes_discovered_roots() {
        let sdl = render(
            r#"
            struct Query;

            #[Object]
            impl Query {
                async fn ping(&self) -> String {
                    "pong".to_string()
                }
            }
            "#,
        );
        assert!(sdl.starts_with("schema {\n  query: Query\n}\n\n"));
        assert!(!sdl.contains("mutation:"));
        assert!(!sdl.contains("subscription:"));
    }

    #[test]
    fn complex_object_impl_merges_with_simple_object_struct() {
        let mut builder = SchemaBuilder::new();
        builder
            .add_source(
                r#"
                #[derive(SimpleObject)]
                #[graphql(complex)]
                struct User {
                    id: ID,
                }
                "#,
            )
            .unwrap();
        builder
            .add_source(
                r#"
                #[ComplexObject]
                impl User {
                    async fn full_name(&self) -> String {
                        String::new()
                    }
                }
                "#,
            )
            .unwrap();
        let sdl = builder.render("Query", "Mutation", "Subscription");
        assert!(sdl.contains("id: ID!"));
        assert!(sdl.contains("fullName: String!"));
    }

    #[test]
    fn camel_case_conversion() {
        assert_eq!(to_camel_case("user_id"), "userId");
        assert_eq!(to_camel_case("id"), "id");
        assert_eq!(to_camel_case("get_all_users"), "getAllUsers");
    }

    #[test]
    fn screaming_snake_conversion() {
        assert_eq!(to_screaming_snake("Active"), "ACTIVE");
        assert_eq!(to_screaming_snake("PendingReview"), "PENDING_REVIEW");
        assert_eq!(to_screaming_snake("HTTPError"), "HTTP_ERROR");
    }
}
