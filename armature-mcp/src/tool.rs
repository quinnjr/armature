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
use std::sync::Arc;

/// Handler function type for MCP tools
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
    /// The handler function
    pub handler: ToolHandlerFn,
    /// Type ID of the struct this tool belongs to (for grouping)
    pub owner_type_id: TypeId,
}

inventory::collect!(McpToolEntry);

impl McpToolEntry {
    /// Create a new tool entry
    pub fn new<T: 'static>(
        name: &'static str,
        description: Option<&'static str>,
        input_schema: &'static str,
        handler: ToolHandlerFn,
    ) -> Self {
        Self {
            name,
            description,
            input_schema,
            handler,
            owner_type_id: TypeId::of::<T>(),
        }
    }

    /// Convert to a ToolDefinition for the protocol
    pub fn to_definition(&self) -> ToolDefinition {
        let schema: Value = serde_json::from_str(self.input_schema)
            .unwrap_or_else(|_| serde_json::json!({"type": "object"}));

        ToolDefinition {
            name: self.name.to_string(),
            description: self.description.map(|s| s.to_string()),
            input_schema: schema,
        }
    }

    /// Call the tool with the given arguments
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
}

impl McpToolRegistry {
    /// Create a new registry and collect all registered tools
    pub fn new() -> Self {
        let mut tools = HashMap::new();

        for entry in inventory::iter::<McpToolEntry> {
            tools.insert(entry.name.to_string(), entry);
        }

        Self { tools }
    }

    /// Get all registered tool definitions
    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .map(|entry| entry.to_definition())
            .collect()
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
                Some($description),
                $schema,
                std::sync::Arc::new(move |args| {
                    Box::pin($handler(args))
                }),
            )
        }
    };
    ($owner:ty, $name:expr, $schema:expr, $handler:expr) => {
        $crate::inventory::submit! {
            $crate::tool::McpToolEntry::new::<$owner>(
                $name,
                None,
                $schema,
                std::sync::Arc::new(move |args| {
                    Box::pin($handler(args))
                }),
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

        let entry = McpToolEntry::new::<TestOwner>(
            "test_tool",
            Some("A test tool"),
            r#"{"type": "object", "properties": {"input": {"type": "string"}}}"#,
            Arc::new(|_| Box::pin(async { Ok(ToolCallResult::text("test")) })),
        );

        let def = entry.to_definition();
        assert_eq!(def.name, "test_tool");
        assert_eq!(def.description, Some("A test tool".to_string()));
    }
}
