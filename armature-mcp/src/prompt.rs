//! MCP Prompt definitions and registry
//!
//! Prompts are reusable, named message templates a client can list and
//! render. They are registered at compile time with the
//! [`register_mcp_prompt!`](crate::register_mcp_prompt) macro and collected
//! via the `inventory` crate, mirroring the tool registry in
//! [`crate::tool`].

use crate::error::{McpError, Result};
use crate::types::{PromptArgument, PromptDefinition, PromptGetResult};
use async_trait::async_trait;
use serde_json::Value;
use std::any::TypeId;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

/// Fn-pointer form of an MCP prompt handler. Const-evaluable, so it can sit
/// inside `inventory::submit!`'s static initializer.
pub type PromptHandlerFnPtr =
    fn(Value) -> Pin<Box<dyn Future<Output = Result<PromptGetResult>> + Send>>;

/// An MCP prompt entry registered at compile time
pub struct McpPromptEntry {
    /// Unique name of the prompt
    pub name: &'static str,
    /// Human-readable description
    pub description: Option<&'static str>,
    /// JSON array of argument descriptors (as a JSON string), e.g.
    /// `r#"[{"name":"topic","required":true}]"#`.
    pub arguments: &'static str,
    /// The handler function (fn pointer so the entry can be built in a
    /// `static` context — see `register_mcp_prompt!`).
    pub handler: PromptHandlerFnPtr,
    /// Type ID of the struct this prompt belongs to (for grouping)
    pub owner_type_id: TypeId,
}

inventory::collect!(McpPromptEntry);

impl McpPromptEntry {
    /// Create a new prompt entry.
    ///
    /// `const fn` so the macro expansion can sit inside
    /// `inventory::submit!`'s static initializer on Rust 2024.
    pub const fn new<T: 'static>(
        name: &'static str,
        description: Option<&'static str>,
        arguments: &'static str,
        handler: PromptHandlerFnPtr,
    ) -> Self {
        Self {
            name,
            description,
            arguments,
            handler,
            owner_type_id: TypeId::of::<T>(),
        }
    }

    /// Convert to a [`PromptDefinition`] for the protocol.
    ///
    /// Note: this re-parses the `&'static str` argument list on each call.
    /// [`McpPromptRegistry`] caches the parsed result, so serving paths go
    /// through it rather than calling this per request.
    pub fn to_definition(&self) -> PromptDefinition {
        let arguments: Vec<PromptArgument> =
            serde_json::from_str(self.arguments).unwrap_or_else(|e| {
                // Same reasoning as `McpToolEntry::to_definition`: an
                // unparseable argument list is a registration bug, and
                // silently advertising "this prompt takes no arguments"
                // hands that bug to clients as a protocol-level lie.
                tracing::error!(
                    prompt = self.name,
                    error = %e,
                    arguments = self.arguments,
                    "MCP prompt has a malformed argument list; advertising no arguments instead"
                );
                debug_assert!(
                    false,
                    "MCP prompt `{}` has a malformed argument list: {e}",
                    self.name
                );
                Vec::new()
            });

        PromptDefinition {
            name: self.name.to_string(),
            description: self.description.map(|s| s.to_string()),
            arguments,
        }
    }

    /// Render the prompt with the given arguments. The returned future is `Send`.
    pub async fn render(&self, arguments: Value) -> Result<PromptGetResult> {
        (self.handler)(arguments).await
    }
}

/// Trait for types that provide MCP prompts
#[async_trait]
pub trait McpPromptProvider: Send + Sync {
    /// Get prompt definitions provided by this type
    fn prompts(&self) -> Vec<PromptDefinition>;

    /// Render a prompt by name
    async fn get_prompt(&self, name: &str, arguments: Value) -> Result<PromptGetResult>;
}

/// Registry for MCP prompts collected at compile time
#[derive(Default)]
pub struct McpPromptRegistry {
    prompts: HashMap<String, &'static McpPromptEntry>,
    /// Lazily-computed, sorted cache of prompt names. The registry is
    /// populated once at startup from `inventory` and never mutated
    /// afterward, so the cache stays valid for its whole lifetime.
    sorted_names: OnceLock<Vec<String>>,
    /// Lazily-computed cache of parsed [`PromptDefinition`]s keyed by name,
    /// so repeated `prompts/list` calls do not re-parse every entry's
    /// argument list.
    definitions: OnceLock<HashMap<String, PromptDefinition>>,
}

impl McpPromptRegistry {
    /// Create a new registry and collect all registered prompts
    pub fn new() -> Self {
        let mut prompts = HashMap::new();

        for entry in inventory::iter::<McpPromptEntry> {
            prompts.insert(entry.name.to_string(), entry);
        }

        Self {
            prompts,
            sorted_names: OnceLock::new(),
            definitions: OnceLock::new(),
        }
    }

    /// Parsed prompt definitions keyed by name, parsed once and cached.
    fn definitions(&self) -> &HashMap<String, PromptDefinition> {
        self.definitions.get_or_init(|| {
            self.prompts
                .iter()
                .map(|(name, entry)| (name.clone(), entry.to_definition()))
                .collect()
        })
    }

    /// Get all registered prompt definitions
    pub fn list_prompts(&self) -> Vec<PromptDefinition> {
        self.definitions().values().cloned().collect()
    }

    /// Get a page of registered prompt definitions, ordered
    /// deterministically by name.
    ///
    /// `cursor` is an opaque token: the name of the last item returned by
    /// the previous page (or `None` to start from the beginning).
    pub fn list_prompts_page(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> (Vec<PromptDefinition>, Option<String>) {
        let names = self.sorted_names.get_or_init(|| {
            let mut names: Vec<String> = self.prompts.keys().cloned().collect();
            names.sort();
            names
        });

        let (page_names, next_cursor) =
            crate::service::paginate_by(names, cursor, limit, |name| name.as_str());

        let definitions = self.definitions();
        let defs = page_names
            .iter()
            .map(|name| definitions[name.as_str()].clone())
            .collect();

        (defs, next_cursor)
    }

    /// Get a prompt entry by name
    pub fn get_prompt_entry(&self, name: &str) -> Option<&'static McpPromptEntry> {
        self.prompts.get(name).copied()
    }

    /// Render a prompt by name
    pub async fn get_prompt(&self, name: &str, arguments: Value) -> Result<PromptGetResult> {
        let entry = self
            .prompts
            .get(name)
            .ok_or_else(|| McpError::PromptNotFound(name.to_string()))?;

        entry.render(arguments).await
    }

    /// Check if a prompt exists
    pub fn has_prompt(&self, name: &str) -> bool {
        self.prompts.contains_key(name)
    }

    /// Get the number of registered prompts
    pub fn len(&self) -> usize {
        self.prompts.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.prompts.is_empty()
    }
}

/// Macro to register an MCP prompt at compile time
///
/// # Usage
///
/// ```ignore
/// use armature_mcp::register_mcp_prompt;
///
/// async fn summarize(args: Value) -> Result<PromptGetResult> {
///     let topic = args.get("topic").and_then(|v| v.as_str()).unwrap_or("");
///     Ok(PromptGetResult::text(format!("Summarize {topic}")))
/// }
///
/// register_mcp_prompt!(
///     MyPrompts,
///     "summarize",
///     "Summarize a topic",
///     r#"[{"name": "topic", "required": true}]"#,
///     summarize
/// );
/// ```
#[macro_export]
macro_rules! register_mcp_prompt {
    ($owner:ty, $name:expr, $description:expr, $arguments:expr, $handler:expr) => {
        $crate::inventory::submit! {
            $crate::prompt::McpPromptEntry::new::<$owner>(
                $name,
                ::core::option::Option::Some($description),
                $arguments,
                {
                    fn __wrap(args: ::serde_json::Value)
                        -> ::std::pin::Pin<
                            ::std::boxed::Box<
                                dyn ::std::future::Future<
                                    Output = $crate::error::Result<$crate::types::PromptGetResult>,
                                > + ::std::marker::Send,
                            >,
                        >
                    {
                        ::std::boxed::Box::pin($handler(args))
                    }
                    __wrap as $crate::prompt::PromptHandlerFnPtr
                },
            )
        }
    };
    ($owner:ty, $name:expr, $arguments:expr, $handler:expr) => {
        $crate::inventory::submit! {
            $crate::prompt::McpPromptEntry::new::<$owner>(
                $name,
                ::core::option::Option::None,
                $arguments,
                {
                    fn __wrap(args: ::serde_json::Value)
                        -> ::std::pin::Pin<
                            ::std::boxed::Box<
                                dyn ::std::future::Future<
                                    Output = $crate::error::Result<$crate::types::PromptGetResult>,
                                > + ::std::marker::Send,
                            >,
                        >
                    {
                        ::std::boxed::Box::pin($handler(args))
                    }
                    __wrap as $crate::prompt::PromptHandlerFnPtr
                },
            )
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestOwner;

    fn test_handler(_args: Value) -> Pin<Box<dyn Future<Output = Result<PromptGetResult>> + Send>> {
        Box::pin(async { Ok(PromptGetResult::text("hello")) })
    }

    #[test]
    fn test_prompt_registry_is_empty_without_inventory_entries() {
        let registry = McpPromptRegistry::new();
        assert!(registry.is_empty());
    }

    #[test]
    fn test_prompt_definition_conversion() {
        let entry = McpPromptEntry::new::<TestOwner>(
            "summarize",
            Some("Summarize a topic"),
            r#"[{"name": "topic", "required": true}]"#,
            test_handler as PromptHandlerFnPtr,
        );

        let def = entry.to_definition();
        assert_eq!(def.name, "summarize");
        assert_eq!(def.description.as_deref(), Some("Summarize a topic"));
        assert_eq!(def.arguments.len(), 1);
        assert_eq!(def.arguments[0].name, "topic");
        assert!(def.arguments[0].required);
    }

    /// Build a registry with `count` prompts named `prompt_00`, ... so
    /// pagination has a deterministically ordered set to page through.
    fn registry_with_prompts(count: usize) -> McpPromptRegistry {
        let mut prompts = HashMap::new();
        for i in 0..count {
            let name: &'static str = Box::leak(format!("prompt_{i:02}").into_boxed_str());
            let entry: &'static McpPromptEntry =
                Box::leak(Box::new(McpPromptEntry::new::<TestOwner>(
                    name,
                    None,
                    "[]",
                    test_handler as PromptHandlerFnPtr,
                )));
            prompts.insert(name.to_string(), entry);
        }
        McpPromptRegistry {
            prompts,
            sorted_names: OnceLock::new(),
            definitions: OnceLock::new(),
        }
    }

    #[test]
    fn test_list_prompts_page_paginates_by_name() {
        let registry = registry_with_prompts(10);

        let (page, cursor) = registry.list_prompts_page(None, 4);
        assert_eq!(page.len(), 4);
        assert_eq!(page[0].name, "prompt_00");
        assert_eq!(cursor, Some("prompt_03".to_string()));

        let (last, cursor) = registry.list_prompts_page(Some("prompt_07"), 4);
        assert_eq!(last.len(), 2);
        assert_eq!(last[0].name, "prompt_08");
        assert_eq!(cursor, None);
    }

    #[tokio::test]
    async fn test_get_prompt_renders_registered_prompt() {
        let registry = registry_with_prompts(1);

        let result = registry
            .get_prompt("prompt_00", serde_json::json!({}))
            .await
            .expect("registered prompt should render");
        assert_eq!(result.messages.len(), 1);

        let err = registry
            .get_prompt("missing", serde_json::json!({}))
            .await
            .expect_err("unknown prompt should error");
        assert_eq!(err.to_error_code(), -32002);
    }
}
