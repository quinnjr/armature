//! MCP Tool definitions and registry
//!
//! Tools are functions that can be invoked by language models through the MCP protocol.
//! They are registered at compile time using the `#[mcp]` attribute macro and collected
//! via the `inventory` crate.

use crate::error::{McpError, Result};
use crate::types::{ToolCallResult, ToolDefinition};
use async_trait::async_trait;
use serde_json::Value;
use std::any::TypeId;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

/// Fn-pointer form of an MCP tool handler. Const-evaluable, so it can
/// sit inside `inventory::submit!`'s static initializer.
pub type ToolHandlerFnPtr =
    fn(Value) -> Pin<Box<dyn Future<Output = Result<ToolCallResult>> + Send>>;

/// Runtime-callable form. Kept as a public alias for backwards
/// compatibility, but the inventory entry no longer carries this
/// directly. The registry wraps the fn pointer at call time.
pub type ToolHandlerFn = Arc<
    dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<ToolCallResult>> + Send>> + Send + Sync,
>;

/// An MCP tool entry registered at compile time
pub struct McpToolEntry {
    /// Unique name of the tool
    pub name: &'static str,
    /// Human-readable description
    pub description: Option<&'static str>,
    /// JSON Schema for input parameters (as JSON string)
    pub input_schema: &'static str,
    /// The handler function (fn pointer so the entry can be built in a
    /// `static` context — see `register_mcp_tool!`).
    pub handler: ToolHandlerFnPtr,
    /// Type ID of the struct this tool belongs to (for grouping)
    pub owner_type_id: TypeId,
}

inventory::collect!(McpToolEntry);

impl McpToolEntry {
    /// Create a new tool entry.
    ///
    /// `const fn` so the macro expansion can sit inside
    /// `inventory::submit!`'s static initializer on Rust 2024.
    pub const fn new<T: 'static>(
        name: &'static str,
        description: Option<&'static str>,
        input_schema: &'static str,
        handler: ToolHandlerFnPtr,
    ) -> Self {
        Self {
            name,
            description,
            input_schema,
            handler,
            owner_type_id: TypeId::of::<T>(),
        }
    }

    /// Convert to a ToolDefinition for the protocol.
    ///
    /// Note: this re-parses the `&'static str` schema on each call. Callers
    /// that serve definitions repeatedly (e.g. `tools/list` per request)
    /// should go through [`McpToolRegistry::list_tools`] /
    /// [`McpToolRegistry::list_tools_page`], which parse each entry at most
    /// once and cache the result (see `McpToolRegistry::definitions`).
    pub fn to_definition(&self) -> ToolDefinition {
        let schema: Value = serde_json::from_str(self.input_schema).unwrap_or_else(|e| {
            // A schema that does not parse is a bug in the `#[mcp]` macro
            // expansion or in the literal passed to `register_mcp_tool!`.
            // Falling back to a permissive `{"type":"object"}` keeps the
            // server serving, but it silently tells every client "this tool
            // accepts any object" — so make the substitution loud rather
            // than shipping the bug downstream unnoticed.
            tracing::error!(
                tool = self.name,
                error = %e,
                schema = self.input_schema,
                "MCP tool has a malformed input_schema; advertising a permissive object schema instead"
            );
            debug_assert!(
                false,
                "MCP tool `{}` has a malformed input_schema: {e}",
                self.name
            );
            serde_json::json!({"type": "object"})
        });

        ToolDefinition {
            name: self.name.to_string(),
            description: self.description.map(|s| s.to_string()),
            input_schema: schema,
        }
    }

    /// Call the tool with the given arguments. The returned future is `Send`.
    pub async fn call(&self, arguments: Value) -> Result<ToolCallResult> {
        (self.handler)(arguments).await
    }
}

/// Trait for types that provide MCP tools
#[async_trait]
pub trait McpToolProvider: Send + Sync {
    /// Get tool definitions provided by this type
    fn tools(&self) -> Vec<ToolDefinition>;

    /// Call a tool by name
    async fn call_tool(&self, name: &str, arguments: Value) -> Result<ToolCallResult>;
}

/// Registry for MCP tools collected at compile time
#[derive(Default)]
pub struct McpToolRegistry {
    tools: HashMap<String, &'static McpToolEntry>,
    /// Lazily-computed, sorted cache of tool names. The registry is
    /// populated once at startup from `inventory` and never mutated
    /// afterward, so this cache — built on the first paginated list
    /// call — stays valid for the registry's whole lifetime and avoids
    /// re-sorting the full key set on every `list_tools_page` call.
    sorted_names: OnceLock<Vec<String>>,
    /// Lazily-computed cache of parsed `ToolDefinition`s keyed by tool name.
    /// Each entry's schema is an immutable `&'static str`, so it only needs
    /// to be parsed once; without this every `list_tools`/`list_tools_page`
    /// call (one per HTTP request on the list endpoints) would re-run
    /// `serde_json::from_str` over every tool's schema. Built once and reused
    /// for the registry's whole (append-only-at-startup) lifetime.
    definitions: OnceLock<HashMap<String, ToolDefinition>>,
}

impl McpToolRegistry {
    /// Create a new registry and collect all registered tools
    pub fn new() -> Self {
        let mut tools = HashMap::new();

        for entry in inventory::iter::<McpToolEntry> {
            tools.insert(entry.name.to_string(), entry);
        }

        Self {
            tools,
            sorted_names: OnceLock::new(),
            definitions: OnceLock::new(),
        }
    }

    /// Parsed tool definitions keyed by name, parsed once and cached (see the
    /// `definitions` field). Shared by `list_tools` and `list_tools_page` so
    /// neither re-parses schemas on repeated calls.
    fn definitions(&self) -> &HashMap<String, ToolDefinition> {
        self.definitions.get_or_init(|| {
            self.tools
                .iter()
                .map(|(name, entry)| (name.clone(), entry.to_definition()))
                .collect()
        })
    }

    /// Get all registered tool definitions
    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        self.definitions().values().cloned().collect()
    }

    /// Get a page of registered tool definitions, ordered deterministically
    /// by name.
    ///
    /// `cursor` is an opaque token: the name of the last item returned by
    /// the previous page (or `None` to start from the beginning). Returns
    /// the page of tools together with the `next_cursor` to pass in to
    /// fetch the following page, or `None` if this was the last page.
    pub fn list_tools_page(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> (Vec<ToolDefinition>, Option<String>) {
        let names = self.sorted_names.get_or_init(|| {
            let mut names: Vec<String> = self.tools.keys().cloned().collect();
            names.sort();
            names
        });

        // Delegate the actual cursor-pagination algorithm to the single
        // shared implementation (see `crate::service::paginate_by`), then
        // map the paged names back to their tool definitions.
        let (page_names, next_cursor) =
            crate::service::paginate_by(names, cursor, limit, |name| name.as_str());

        let definitions = self.definitions();
        let defs = page_names
            .iter()
            .map(|name| definitions[name.as_str()].clone())
            .collect();

        (defs, next_cursor)
    }

    /// Get a tool by name
    pub fn get_tool(&self, name: &str) -> Option<&'static McpToolEntry> {
        self.tools.get(name).copied()
    }

    /// Call a tool by name
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<ToolCallResult> {
        let entry = self
            .tools
            .get(name)
            .ok_or_else(|| McpError::ToolNotFound(name.to_string()))?;

        entry.call(arguments).await
    }

    /// Check if a tool exists
    pub fn has_tool(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Get the number of registered tools
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

/// Macro to register an MCP tool at compile time
///
/// # Usage
///
/// ```ignore
/// use armature_mcp::register_mcp_tool;
///
/// async fn my_tool_handler(args: Value) -> Result<ToolCallResult> {
///     let input: MyInput = serde_json::from_value(args)?;
///     Ok(ToolCallResult::text(format!("Result: {}", input.value)))
/// }
///
/// register_mcp_tool!(
///     MyToolProvider,
///     "my_tool",
///     "Does something useful",
///     r#"{"type": "object", "properties": {"value": {"type": "string"}}}"#,
///     my_tool_handler
/// );
/// ```
#[macro_export]
macro_rules! register_mcp_tool {
    ($owner:ty, $name:expr, $description:expr, $schema:expr, $handler:expr) => {
        $crate::inventory::submit! {
            $crate::tool::McpToolEntry::new::<$owner>(
                $name,
                ::core::option::Option::Some($description),
                $schema,
                {
                    fn __wrap(args: ::serde_json::Value)
                        -> ::std::pin::Pin<
                            ::std::boxed::Box<
                                dyn ::std::future::Future<
                                    Output = $crate::error::Result<$crate::types::ToolCallResult>,
                                > + ::std::marker::Send,
                            >,
                        >
                    {
                        ::std::boxed::Box::pin($handler(args))
                    }
                    __wrap as $crate::tool::ToolHandlerFnPtr
                },
            )
        }
    };
    ($owner:ty, $name:expr, $schema:expr, $handler:expr) => {
        $crate::inventory::submit! {
            $crate::tool::McpToolEntry::new::<$owner>(
                $name,
                ::core::option::Option::None,
                $schema,
                {
                    fn __wrap(args: ::serde_json::Value)
                        -> ::std::pin::Pin<
                            ::std::boxed::Box<
                                dyn ::std::future::Future<
                                    Output = $crate::error::Result<$crate::types::ToolCallResult>,
                                > + ::std::marker::Send,
                            >,
                        >
                    {
                        ::std::boxed::Box::pin($handler(args))
                    }
                    __wrap as $crate::tool::ToolHandlerFnPtr
                },
            )
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_registry_creation() {
        let registry = McpToolRegistry::new();
        // No tools registered in test context without inventory.
        // The constructor must succeed and the registry must be empty
        // (`registry.len() >= 0` was a tautology — len() returns usize).
        assert!(registry.is_empty());
    }

    #[test]
    fn test_tool_definition_conversion() {
        struct TestOwner;

        fn test_handler(
            _args: Value,
        ) -> Pin<Box<dyn Future<Output = Result<ToolCallResult>> + Send>> {
            Box::pin(async { Ok(ToolCallResult::text("test")) })
        }

        let entry = McpToolEntry::new::<TestOwner>(
            "test_tool",
            Some("A test tool"),
            r#"{"type": "object", "properties": {"input": {"type": "string"}}}"#,
            test_handler as ToolHandlerFnPtr,
        );

        let def = entry.to_definition();
        assert_eq!(def.name, "test_tool");
        assert_eq!(def.description, Some("A test tool".to_string()));
    }

    struct PageTestOwner;

    fn page_test_handler(
        _args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolCallResult>> + Send>> {
        Box::pin(async { Ok(ToolCallResult::text("test")) })
    }

    /// Build a registry with `count` tools named `tool_00`, `tool_01`, ...
    /// so pagination has a large, deterministically ordered set to page
    /// through.
    fn registry_with_tools(count: usize) -> McpToolRegistry {
        let mut tools = HashMap::new();
        for i in 0..count {
            let name: &'static str = Box::leak(format!("tool_{i:02}").into_boxed_str());
            let entry: &'static McpToolEntry =
                Box::leak(Box::new(McpToolEntry::new::<PageTestOwner>(
                    name,
                    None,
                    r#"{"type": "object"}"#,
                    page_test_handler as ToolHandlerFnPtr,
                )));
            tools.insert(name.to_string(), entry);
        }
        McpToolRegistry {
            tools,
            sorted_names: OnceLock::new(),
            definitions: OnceLock::new(),
        }
    }

    #[test]
    fn test_list_tools_page_first_page_has_cursor() {
        let registry = registry_with_tools(10);

        let (page, next_cursor) = registry.list_tools_page(None, 4);

        assert_eq!(page.len(), 4);
        assert_eq!(page[0].name, "tool_00");
        assert_eq!(page[3].name, "tool_03");
        assert_eq!(next_cursor, Some("tool_03".to_string()));
    }

    #[test]
    fn test_list_tools_page_cursor_returns_next_page() {
        let registry = registry_with_tools(10);

        let (first, cursor) = registry.list_tools_page(None, 4);
        let cursor = cursor.expect("first page should have a cursor");
        let (second, _) = registry.list_tools_page(Some(&cursor), 4);

        assert_eq!(second.len(), 4);
        assert_eq!(second[0].name, "tool_04");
        assert_eq!(second[3].name, "tool_07");

        // no overlap between pages
        let first_names: std::collections::HashSet<_> =
            first.iter().map(|t| t.name.clone()).collect();
        let second_names: std::collections::HashSet<_> =
            second.iter().map(|t| t.name.clone()).collect();
        assert!(first_names.is_disjoint(&second_names));
    }

    #[test]
    fn test_list_tools_page_final_page_has_no_cursor() {
        let registry = registry_with_tools(10);

        let (page, cursor) = registry.list_tools_page(Some("tool_07"), 4);

        assert_eq!(page.len(), 2);
        assert_eq!(page[0].name, "tool_08");
        assert_eq!(page[1].name, "tool_09");
        assert_eq!(cursor, None);
    }

    #[test]
    fn test_list_tools_page_union_equals_full_list_no_dupes() {
        let registry = registry_with_tools(10);

        let mut collected = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let (page, next_cursor) = registry.list_tools_page(cursor.as_deref(), 3);
            collected.extend(page.into_iter().map(|t| t.name));
            match next_cursor {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }

        let mut expected: Vec<String> = (0..10).map(|i| format!("tool_{i:02}")).collect();
        expected.sort();

        assert_eq!(collected, expected);

        let unique: std::collections::HashSet<_> = collected.iter().collect();
        assert_eq!(unique.len(), collected.len());
    }
}
