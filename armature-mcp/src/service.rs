//! MCP Service implementation
//!
//! Handles JSON-RPC 2.0 requests for the Model Context Protocol.

use crate::auth::{McpAuthConfig, McpAuthContext, authenticate};
use crate::error::{McpError, Result};
use crate::prompt::{McpPromptProvider, McpPromptRegistry};
use crate::resource::{McpResourceProvider, McpResourceRegistry};
use crate::tool::{McpToolProvider, McpToolRegistry};
use crate::types::*;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// MCP protocol version
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Maximum number of items returned per page by the `tools/list`,
/// `resources/list` and `prompts/list` handlers.
const LIST_PAGE_SIZE: usize = 100;

/// Configuration for the MCP service
#[derive(Clone)]
pub struct McpConfig {
    /// Server name
    pub server_name: String,
    /// Server version
    pub server_version: String,
    /// Enable tools capability
    pub enable_tools: bool,
    /// Enable resources capability
    pub enable_resources: bool,
    /// Enable prompts capability
    pub enable_prompts: bool,
    /// Authentication configuration
    pub auth: McpAuthConfig,
}

impl std::fmt::Debug for McpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpConfig")
            .field("server_name", &self.server_name)
            .field("server_version", &self.server_version)
            .field("enable_tools", &self.enable_tools)
            .field("enable_resources", &self.enable_resources)
            .field("enable_prompts", &self.enable_prompts)
            .field("auth", &"<auth_config>")
            .finish()
    }
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            server_name: "armature-mcp".to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            enable_tools: true,
            enable_resources: true,
            enable_prompts: false,
            auth: McpAuthConfig::default(),
        }
    }
}

impl McpConfig {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            server_name: name.into(),
            server_version: version.into(),
            ..Default::default()
        }
    }

    pub fn with_tools(mut self, enabled: bool) -> Self {
        self.enable_tools = enabled;
        self
    }

    pub fn with_resources(mut self, enabled: bool) -> Self {
        self.enable_resources = enabled;
        self
    }

    pub fn with_prompts(mut self, enabled: bool) -> Self {
        self.enable_prompts = enabled;
        self
    }

    /// Configure authentication for MCP access
    pub fn with_auth(mut self, auth: McpAuthConfig) -> Self {
        self.auth = auth;
        self
    }
}

/// MCP service that handles protocol requests
pub struct McpService {
    config: McpConfig,
    tool_registry: McpToolRegistry,
    resource_registry: McpResourceRegistry,
    prompt_registry: McpPromptRegistry,
    /// Dynamically-registered prompt providers, plugged in alongside the
    /// compile-time `register_mcp_prompt!` inventory via
    /// [`McpService::with_prompt_provider`].
    prompt_providers: Vec<Arc<dyn McpPromptProvider>>,
    /// Lazily-computed, merged & sorted cache of prompt definitions
    /// (registry and providers together, registry-precedence deduped by name).
    merged_prompts_cache: OnceLock<Vec<PromptDefinition>>,
    /// Dynamically-registered tool providers, plugged in alongside the
    /// compile-time `#[mcp]` inventory via [`McpService::with_tool_provider`].
    tool_providers: Vec<Arc<dyn McpToolProvider>>,
    /// Dynamically-registered resource providers, plugged in alongside the
    /// compile-time `#[mcp]` inventory via [`McpService::with_resource_provider`].
    resource_providers: Vec<Arc<dyn McpResourceProvider>>,
    /// Lazily-computed, merged & sorted cache of tool definitions (registry
    /// and providers together, registry-precedence deduped by name). Built
    /// once on first `tools/list` call since providers are registered
    /// before serving and never mutated afterward.
    merged_tools_cache: OnceLock<Vec<ToolDefinition>>,
    /// Lazily-computed, merged & sorted cache of resource definitions
    /// (registry and providers together, registry-precedence deduped by uri).
    merged_resources_cache: OnceLock<Vec<ResourceDefinition>>,
}

/// Try a sequence of provider calls (each an expression awaiting a provider,
/// e.g. `provider.call_tool(...)` / `provider.read_resource(...)`) after the
/// primary lookup (inventory registry) reported "not found". Evaluates to
/// the first `Ok`, or the first non-"not found" `Err` (short-circuiting), or
/// — if every provider also reports "not found" — the initial "not found"
/// error (updated to the most recent provider's own "not found" error as
/// the loop progresses).
///
/// Shared by `McpService::handle_tools_call` and
/// `McpService::handle_resources_read`, which previously duplicated this
/// fallback-dispatch loop verbatim. A macro (rather than a generic async
/// fn) avoids the two call sites' provider-future types acquiring an
/// over-narrow inferred lifetime that trips higher-ranked-trait-bound
/// checks elsewhere in the crate (observed as spurious "implementation of
/// `FnOnce`/`Send` is not general enough" errors at unrelated call sites).
macro_rules! dispatch_with_fallback {
    ($initial_err:expr, $providers:expr, |$p:ident| $call:expr, $is_not_found:pat) => {{
        let mut last_err = $initial_err;
        let mut found = None;

        for $p in $providers {
            match $call.await {
                Ok(v) => {
                    found = Some(Ok(v));
                    break;
                }
                Err(e @ $is_not_found) => {
                    last_err = e;
                }
                Err(e) => {
                    found = Some(Err(e));
                    break;
                }
            }
        }

        match found {
            Some(result) => result,
            None => Err(last_err),
        }
    }};
}

impl McpService {
    /// Create a new MCP service with default configuration
    pub fn new() -> Self {
        Self {
            config: McpConfig::default(),
            tool_registry: McpToolRegistry::new(),
            resource_registry: McpResourceRegistry::new(),
            prompt_registry: McpPromptRegistry::new(),
            tool_providers: Vec::new(),
            resource_providers: Vec::new(),
            prompt_providers: Vec::new(),
            merged_tools_cache: OnceLock::new(),
            merged_resources_cache: OnceLock::new(),
            merged_prompts_cache: OnceLock::new(),
        }
    }

    /// Create a new MCP service with custom configuration
    pub fn with_config(config: McpConfig) -> Self {
        Self {
            config,
            tool_registry: McpToolRegistry::new(),
            resource_registry: McpResourceRegistry::new(),
            prompt_registry: McpPromptRegistry::new(),
            tool_providers: Vec::new(),
            resource_providers: Vec::new(),
            prompt_providers: Vec::new(),
            merged_tools_cache: OnceLock::new(),
            merged_resources_cache: OnceLock::new(),
            merged_prompts_cache: OnceLock::new(),
        }
    }

    /// Register a dynamic tool provider. Providers are consulted by
    /// `tools/list` (merged with the compile-time inventory, registry names
    /// taking precedence) and by `tools/call` (tried in registration order
    /// after the inventory registry reports no match).
    pub fn with_tool_provider(mut self, provider: Arc<dyn McpToolProvider>) -> Self {
        self.tool_providers.push(provider);
        self
    }

    /// Register a dynamic resource provider. Providers are consulted by
    /// `resources/list` (merged with the compile-time inventory, registry
    /// URIs taking precedence) and by `resources/read` (tried in
    /// registration order after the inventory registry reports no match).
    pub fn with_resource_provider(mut self, provider: Arc<dyn McpResourceProvider>) -> Self {
        self.resource_providers.push(provider);
        self
    }

    /// Register a dynamic prompt provider. Providers are consulted by
    /// `prompts/list` (merged with the compile-time inventory, registry
    /// names taking precedence) and by `prompts/get` (tried in registration
    /// order after the inventory registry reports no match).
    pub fn with_prompt_provider(mut self, provider: Arc<dyn McpPromptProvider>) -> Self {
        self.prompt_providers.push(provider);
        self
    }

    /// Handle a JSON-RPC request.
    ///
    /// Returns `None` when the request is a *notification* — a request
    /// carrying no `id`. JSON-RPC 2.0 §4.1 forbids the server from replying
    /// to one, so the method is still dispatched for its side effects but
    /// neither its result nor any error it produced is sent back. Callers
    /// must emit an empty response body in that case (the HTTP controller
    /// answers `204 No Content`).
    pub async fn handle_request(
        &self,
        request: JsonRpcRequest,
        headers: &HashMap<String, String>,
    ) -> Option<JsonRpcResponse> {
        let id = request.id.clone();

        let outcome = self
            .dispatch_method(&request.method, request.params, headers)
            .await;

        // Dispatch happens first (side effects still apply), the reply is
        // then suppressed for notifications.
        id.as_ref()?;

        Some(match outcome {
            Ok(result) => JsonRpcResponse::success(id, result),
            Err(e) => {
                JsonRpcResponse::error(id, JsonRpcError::new(e.to_error_code(), e.to_string()))
            }
        })
    }

    /// Handle a raw JSON request body with headers, returning the serialized
    /// JSON-RPC response, or `None` if the request was a notification and
    /// therefore must not be answered.
    ///
    /// Parses directly out of the caller's byte buffer, so an HTTP handler
    /// need not copy the request body first.
    pub async fn handle_bytes(
        &self,
        body: &[u8],
        headers: &HashMap<String, String>,
    ) -> Option<String> {
        // A top-level JSON array is a JSON-RPC batch. Batching is not
        // supported (documented at the crate root). The payload is
        // well-formed JSON, so `-32600 Invalid Request` is the honest code
        // here — `-32700 Parse error` would misreport it.
        if body.iter().copied().find(|b| !b.is_ascii_whitespace()) == Some(b'[') {
            return Some(encode_response(JsonRpcResponse::error(
                None,
                JsonRpcError::invalid_request("Batch requests are not supported"),
            )));
        }

        let response = match serde_json::from_slice::<JsonRpcRequest>(body) {
            Ok(req) => self.handle_request(req, headers).await?,
            Err(e) => JsonRpcResponse::error(None, JsonRpcError::parse_error(e.to_string())),
        };

        Some(encode_response(response))
    }

    /// Handle a raw JSON request string with headers. See [`Self::handle_bytes`]
    /// for the `None` (notification) case.
    pub async fn handle_json(
        &self,
        json: &str,
        headers: &HashMap<String, String>,
    ) -> Option<String> {
        self.handle_bytes(json.as_bytes(), headers).await
    }

    /// Handle a JSON request without authentication (for backwards compatibility)
    pub async fn handle_json_unauthenticated(&self, json: &str) -> Option<String> {
        self.handle_json(json, &HashMap::new()).await
    }

    /// Dispatch a method call to the appropriate handler
    async fn dispatch_method(
        &self,
        method: &str,
        params: Option<Value>,
        headers: &HashMap<String, String>,
    ) -> Result<Value> {
        // Authenticate the request
        let _auth_context = authenticate(&self.config.auth, headers, method).await?;

        match method {
            "initialize" => self.handle_initialize(params),
            "tools/list" => self.handle_tools_list(params),
            "tools/call" => self.handle_tools_call(params).await,
            "resources/list" => self.handle_resources_list(params),
            "resources/read" => self.handle_resources_read(params).await,
            "prompts/list" => self.handle_prompts_list(params),
            "prompts/get" => self.handle_prompts_get(params).await,
            "ping" => Ok(serde_json::json!({})),
            // Handshake/lifecycle notifications every MCP client sends. They
            // carry no `id`, so `handle_request` discards this result; the
            // arm exists so they are not mistaken for unknown methods.
            "notifications/initialized" | "notifications/cancelled" => Ok(serde_json::json!({})),
            _ => Err(McpError::MethodNotFound(method.to_string())),
        }
    }

    /// Get the authentication context for a request (for custom handling)
    pub async fn authenticate(
        &self,
        headers: &HashMap<String, String>,
        method: &str,
    ) -> Result<McpAuthContext> {
        authenticate(&self.config.auth, headers, method).await
    }

    /// Handle initialize request
    fn handle_initialize(&self, _params: Option<Value>) -> Result<Value> {
        let mut capabilities = ServerCapabilities::default();

        if self.config.enable_tools {
            capabilities.tools = Some(ToolsCapability {
                list_changed: false,
            });
        }

        if self.config.enable_resources {
            capabilities.resources = Some(ResourcesCapability {
                subscribe: false,
                list_changed: false,
            });
        }

        if self.config.enable_prompts {
            capabilities.prompts = Some(PromptsCapability {
                list_changed: false,
            });
        }

        let result = InitializeResult {
            protocol_version: MCP_PROTOCOL_VERSION.to_string(),
            capabilities,
            server_info: ServerInfo {
                name: self.config.server_name.clone(),
                version: self.config.server_version.clone(),
            },
        };

        serde_json::to_value(result).map_err(McpError::from)
    }

    /// Handle tools/list request
    fn handle_tools_list(&self, params: Option<Value>) -> Result<Value> {
        let cursor = parse_list_cursor(params)?;

        let (tools, next_cursor) = if self.tool_providers.is_empty() {
            self.tool_registry
                .list_tools_page(cursor.as_deref(), LIST_PAGE_SIZE)
        } else {
            paginate_by(
                self.merged_tools(),
                cursor.as_deref(),
                LIST_PAGE_SIZE,
                |t| t.name.as_str(),
            )
        };

        let result = ToolsListResult { tools, next_cursor };

        serde_json::to_value(result).map_err(McpError::from)
    }

    /// Build (or fetch the cached) merged, name-sorted, deduped list of tool
    /// definitions from the inventory registry plus all registered
    /// providers. Registry entries take precedence over provider entries
    /// with the same name.
    fn merged_tools(&self) -> &[ToolDefinition] {
        self.merged_tools_cache.get_or_init(|| {
            merge_dedup_sorted(
                self.tool_registry.list_tools(),
                self.tool_providers.iter().flat_map(|p| p.tools()),
                |t| t.name.as_str(),
            )
        })
    }

    /// Handle tools/call request
    async fn handle_tools_call(&self, params: Option<Value>) -> Result<Value> {
        let params: ToolCallParams = params
            .ok_or_else(|| McpError::InvalidParams("Missing params".to_string()))
            .and_then(|v| {
                serde_json::from_value(v).map_err(|e| McpError::InvalidParams(e.to_string()))
            })?;

        let result = match self
            .tool_registry
            .call_tool(&params.name, params.arguments.clone())
            .await
        {
            Err(McpError::ToolNotFound(_)) => {
                dispatch_with_fallback!(
                    McpError::ToolNotFound(params.name.clone()),
                    &self.tool_providers,
                    |p| p.call_tool(&params.name, params.arguments.clone()),
                    McpError::ToolNotFound(_)
                )
            }
            other => other,
        }?;

        serde_json::to_value(result).map_err(McpError::from)
    }

    /// Handle resources/list request
    fn handle_resources_list(&self, params: Option<Value>) -> Result<Value> {
        let cursor = parse_list_cursor(params)?;

        let (resources, next_cursor) = if self.resource_providers.is_empty() {
            self.resource_registry
                .list_resources_page(cursor.as_deref(), LIST_PAGE_SIZE)
        } else {
            paginate_by(
                self.merged_resources(),
                cursor.as_deref(),
                LIST_PAGE_SIZE,
                |r| r.uri.as_str(),
            )
        };

        let result = ResourcesListResult {
            resources,
            next_cursor,
        };

        serde_json::to_value(result).map_err(McpError::from)
    }

    /// Build (or fetch the cached) merged, uri-sorted, deduped list of
    /// resource definitions from the inventory registry plus all registered
    /// providers. Registry entries take precedence over provider entries
    /// with the same uri.
    fn merged_resources(&self) -> &[ResourceDefinition] {
        self.merged_resources_cache.get_or_init(|| {
            merge_dedup_sorted(
                self.resource_registry.list_resources(),
                self.resource_providers.iter().flat_map(|p| p.resources()),
                |r| r.uri.as_str(),
            )
        })
    }

    /// Handle resources/read request
    async fn handle_resources_read(&self, params: Option<Value>) -> Result<Value> {
        let params: ResourceReadParams = params
            .ok_or_else(|| McpError::InvalidParams("Missing params".to_string()))
            .and_then(|v| {
                serde_json::from_value(v).map_err(|e| McpError::InvalidParams(e.to_string()))
            })?;

        let content = match self.resource_registry.read_resource(&params.uri).await {
            Err(McpError::ResourceNotFound(_)) => {
                dispatch_with_fallback!(
                    McpError::ResourceNotFound(params.uri.clone()),
                    &self.resource_providers,
                    |p| p.read_resource(&params.uri),
                    McpError::ResourceNotFound(_)
                )
            }
            other => other,
        }?;

        let result = ResourceReadResult {
            contents: vec![content],
        };

        serde_json::to_value(result).map_err(McpError::from)
    }

    /// Handle prompts/list request
    fn handle_prompts_list(&self, params: Option<Value>) -> Result<Value> {
        let cursor = parse_list_cursor(params)?;

        let (prompts, next_cursor) = if self.prompt_providers.is_empty() {
            self.prompt_registry
                .list_prompts_page(cursor.as_deref(), LIST_PAGE_SIZE)
        } else {
            paginate_by(
                self.merged_prompts(),
                cursor.as_deref(),
                LIST_PAGE_SIZE,
                |p| p.name.as_str(),
            )
        };

        let result = PromptsListResult {
            prompts,
            next_cursor,
        };

        serde_json::to_value(result).map_err(McpError::from)
    }

    /// Build (or fetch the cached) merged, name-sorted, deduped list of
    /// prompt definitions from the inventory registry plus all registered
    /// providers. Registry entries take precedence over provider entries
    /// with the same name.
    fn merged_prompts(&self) -> &[PromptDefinition] {
        self.merged_prompts_cache.get_or_init(|| {
            merge_dedup_sorted(
                self.prompt_registry.list_prompts(),
                self.prompt_providers.iter().flat_map(|p| p.prompts()),
                |p| p.name.as_str(),
            )
        })
    }

    /// Handle prompts/get request
    async fn handle_prompts_get(&self, params: Option<Value>) -> Result<Value> {
        let params: PromptGetParams = params
            .ok_or_else(|| McpError::InvalidParams("Missing params".to_string()))
            .and_then(|v| {
                serde_json::from_value(v).map_err(|e| McpError::InvalidParams(e.to_string()))
            })?;

        let result = match self
            .prompt_registry
            .get_prompt(&params.name, params.arguments.clone())
            .await
        {
            Err(McpError::PromptNotFound(_)) => {
                dispatch_with_fallback!(
                    McpError::PromptNotFound(params.name.clone()),
                    &self.prompt_providers,
                    |p| p.get_prompt(&params.name, params.arguments.clone()),
                    McpError::PromptNotFound(_)
                )
            }
            other => other,
        }?;

        serde_json::to_value(result).map_err(McpError::from)
    }

    /// Full, merged list of prompt definitions: the compile-time inventory
    /// registry plus every dynamically-registered provider, deduped by name
    /// (registry precedence) and sorted. Mirrors the `prompts/list` JSON-RPC
    /// view.
    pub fn list_prompts(&self) -> Vec<PromptDefinition> {
        self.merged_prompts().to_vec()
    }

    /// Get the prompt registry
    pub fn prompts(&self) -> &McpPromptRegistry {
        &self.prompt_registry
    }

    /// Full, merged list of tool definitions: the compile-time inventory
    /// registry plus every dynamically-registered provider, deduped by name
    /// (registry precedence) and sorted. This is the SAME view served by the
    /// `tools/list` JSON-RPC method, so the HTTP `GET /mcp/tools` handler and
    /// `GET /mcp` info counts agree with it. Prefer this over
    /// `self.tools().list_tools()`, which sees only the registry.
    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        self.merged_tools().to_vec()
    }

    /// Full, merged list of resource definitions: the compile-time inventory
    /// registry plus every dynamically-registered provider, deduped by uri
    /// (registry precedence) and sorted. Mirrors the `resources/list`
    /// JSON-RPC view; used by the HTTP GET handlers.
    pub fn list_resources(&self) -> Vec<ResourceDefinition> {
        self.merged_resources().to_vec()
    }

    /// Get the tool registry
    pub fn tools(&self) -> &McpToolRegistry {
        &self.tool_registry
    }

    /// Get the resource registry
    pub fn resources(&self) -> &McpResourceRegistry {
        &self.resource_registry
    }

    /// Get the configuration
    pub fn config(&self) -> &McpConfig {
        &self.config
    }
}

impl Default for McpService {
    fn default() -> Self {
        Self::new()
    }
}

/// Serialize a JSON-RPC response, falling back to a hand-written internal
/// error object if serialization itself somehow fails (the response types
/// are plain data, so this is unreachable in practice — but it keeps the
/// serving path infallible).
fn encode_response(response: JsonRpcResponse) -> String {
    serde_json::to_string(&response).unwrap_or_else(|_| {
        r#"{"jsonrpc":"2.0","error":{"code":-32603,"message":"Internal error"}}"#.to_string()
    })
}

/// Parse the optional `cursor` field out of a `tools/list`/`resources/list`
/// request's params. Missing params (or a missing/null `cursor`) means
/// "start from the beginning".
fn parse_list_cursor(params: Option<Value>) -> Result<Option<String>> {
    match params {
        None => Ok(None),
        Some(value) => {
            let params: ListParams = serde_json::from_value(value)
                .map_err(|e| McpError::InvalidParams(e.to_string()))?;
            Ok(params.cursor)
        }
    }
}

/// Paginate a pre-sorted slice of items using opaque-cursor semantics:
/// `cursor` is the key of the last item on the previous page (or `None`
/// to start from the beginning), and the returned `next_cursor` is the
/// key of the last item on this page if more items remain, or `None` on
/// the final page.
///
/// This is the single, shared cursor-pagination implementation for the
/// crate: `McpToolRegistry::list_tools_page` (tool.rs) and
/// `McpResourceRegistry::list_resources_page` (resource.rs) both delegate
/// to it (paging their pre-sorted key vectors, then mapping keys to
/// definitions), and [`McpService`] uses it directly here to page
/// already-merged tool/resource definitions.
pub(crate) fn paginate_by<T, K>(
    items: &[T],
    cursor: Option<&str>,
    limit: usize,
    key: K,
) -> (Vec<T>, Option<String>)
where
    T: Clone,
    K: Fn(&T) -> &str,
{
    let start = match cursor {
        Some(c) => items.partition_point(|item| key(item) <= c),
        None => 0,
    };

    let remaining = &items[start..];
    let take = remaining.len().min(limit.max(1));
    let page = &remaining[..take];

    let next_cursor = if take < remaining.len() {
        Some(key(&page[take - 1]).to_string())
    } else {
        None
    };

    (page.to_vec(), next_cursor)
}

/// Merge a "primary" collection (e.g. the compile-time inventory registry)
/// with a "secondary" collection (e.g. dynamically-registered providers),
/// deduplicating by `key` with primary entries taking precedence over
/// secondary entries that share a key, then sort the result by `key`.
///
/// Shared by `McpService::merged_tools` and `McpService::merged_resources`,
/// which previously duplicated this merge/dedup/sort logic verbatim for
/// `ToolDefinition`/`name` and `ResourceDefinition`/`uri` respectively.
fn merge_dedup_sorted<T>(
    primary: Vec<T>,
    secondary: impl IntoIterator<Item = T>,
    key: impl Fn(&T) -> &str,
) -> Vec<T> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut merged: Vec<T> = Vec::new();

    for item in primary {
        seen.insert(key(&item).to_string());
        merged.push(item);
    }

    for item in secondary {
        if seen.insert(key(&item).to_string()) {
            merged.push(item);
        }
    }

    merged.sort_by(|a, b| key(a).cmp(key(b)));
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_headers() -> HashMap<String, String> {
        HashMap::new()
    }

    /// Unwrap the response to a request that carries an `id`. `None` is only
    /// legal for notifications, so it is a test failure everywhere else.
    fn expect_response(response: Option<JsonRpcResponse>) -> JsonRpcResponse {
        response.expect("a request carrying an id must produce a response")
    }

    /// Build a JSON-RPC notification: a request with no `id`.
    fn notification(method: &str) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: method.to_string(),
            params: None,
        }
    }

    #[tokio::test]
    async fn test_handle_initialize() {
        let service = McpService::new();
        let request = JsonRpcRequest::new("initialize");

        let response = expect_response(service.handle_request(request, &empty_headers()).await);

        assert!(response.error.is_none());
        assert!(response.result.is_some());

        let result: InitializeResult = serde_json::from_value(response.result.unwrap()).unwrap();
        assert_eq!(result.protocol_version, MCP_PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn test_handle_ping() {
        let service = McpService::new();
        let request = JsonRpcRequest::new("ping");

        let response = expect_response(service.handle_request(request, &empty_headers()).await);

        assert!(response.error.is_none());
    }

    #[tokio::test]
    async fn test_handle_tools_list() {
        let service = McpService::new();
        let request = JsonRpcRequest::new("tools/list");

        let response = expect_response(service.handle_request(request, &empty_headers()).await);

        assert!(response.error.is_none());
        assert!(response.result.is_some());

        let result: ToolsListResult = serde_json::from_value(response.result.unwrap()).unwrap();
        // No inventory-registered tools in the unit test binary, so the
        // single (empty) page has no continuation.
        assert_eq!(result.next_cursor, None);
    }

    #[tokio::test]
    async fn test_handle_tools_list_with_cursor_param() {
        let service = McpService::new();
        let request = JsonRpcRequest::new("tools/list")
            .with_params(serde_json::json!({ "cursor": "some_tool_name" }));

        let response = expect_response(service.handle_request(request, &empty_headers()).await);

        assert!(response.error.is_none());
        let result: ToolsListResult = serde_json::from_value(response.result.unwrap()).unwrap();
        assert_eq!(result.next_cursor, None);
    }

    #[tokio::test]
    async fn test_handle_resources_list_with_cursor_param() {
        let service = McpService::new();
        let request = JsonRpcRequest::new("resources/list")
            .with_params(serde_json::json!({ "cursor": "some_resource_uri" }));

        let response = expect_response(service.handle_request(request, &empty_headers()).await);

        assert!(response.error.is_none());
        let result: ResourcesListResult = serde_json::from_value(response.result.unwrap()).unwrap();
        assert_eq!(result.next_cursor, None);
    }

    #[test]
    fn test_parse_list_cursor_none_when_no_params() {
        assert_eq!(parse_list_cursor(None).unwrap(), None);
    }

    #[test]
    fn test_parse_list_cursor_extracts_cursor() {
        let params = serde_json::json!({ "cursor": "abc" });
        assert_eq!(
            parse_list_cursor(Some(params)).unwrap(),
            Some("abc".to_string())
        );
    }

    #[test]
    fn test_parse_list_cursor_invalid_params_errors() {
        let params = serde_json::json!({ "cursor": 123 });
        assert!(parse_list_cursor(Some(params)).is_err());
    }

    #[tokio::test]
    async fn test_handle_unknown_method() {
        let service = McpService::new();
        let request = JsonRpcRequest::new("unknown/method");

        let response = expect_response(service.handle_request(request, &empty_headers()).await);

        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }

    #[tokio::test]
    async fn test_handle_json() {
        let service = McpService::new();
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;

        let response = service
            .handle_json(json, &empty_headers())
            .await
            .expect("a request carrying an id must produce a response");

        assert!(response.contains("result"));
    }

    #[tokio::test]
    async fn test_handle_invalid_json() {
        let service = McpService::new();
        let json = "not valid json";

        let response = service
            .handle_json(json, &empty_headers())
            .await
            .expect("a malformed payload still gets an error response");

        assert!(response.contains("error"));
        assert!(response.contains("-32700")); // Parse error
    }

    /// JSON-RPC 2.0 §4.1: the server MUST NOT reply to a notification (a
    /// request with no `id`). Every MCP client sends `notifications/initialized`
    /// right after `initialize`; before this fix it was answered with
    /// `-32601 Method not found`.
    #[tokio::test]
    async fn test_notification_produces_no_response_body() {
        let service = McpService::new();

        assert!(
            service
                .handle_request(notification("notifications/initialized"), &empty_headers())
                .await
                .is_none(),
            "a notification must not be answered"
        );

        let json = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert_eq!(
            service.handle_json(json, &empty_headers()).await,
            None,
            "a notification must not produce a JSON-RPC body"
        );

        // Even a notification naming an unknown method stays silent — the
        // error must not be sent back either.
        let json = r#"{"jsonrpc":"2.0","method":"totally/unknown"}"#;
        assert_eq!(service.handle_json(json, &empty_headers()).await, None);
    }

    /// The handshake notifications are recognised methods, so a client that
    /// (incorrectly) sends them with an `id` gets a result rather than
    /// `-32601`.
    #[tokio::test]
    async fn test_handshake_notifications_are_recognised_methods() {
        let service = McpService::new();

        for method in ["notifications/initialized", "notifications/cancelled"] {
            let response = expect_response(
                service
                    .handle_request(JsonRpcRequest::new(method), &empty_headers())
                    .await,
            );
            assert!(response.error.is_none(), "{method} should be recognised");
        }
    }

    /// Batches are documented as unsupported. The payload is valid JSON, so
    /// it must be reported as an invalid request, not a parse error.
    #[tokio::test]
    async fn test_batch_request_rejected_as_invalid_request() {
        let service = McpService::new();
        let json = r#"  [{"jsonrpc":"2.0","id":1,"method":"ping"}]"#;

        let response = service
            .handle_json(json, &empty_headers())
            .await
            .expect("a batch must still get an error response");

        assert!(response.contains("-32600"), "got: {response}");
        assert!(!response.contains("-32700"), "got: {response}");
    }

    struct EchoToolProvider;

    #[async_trait::async_trait]
    impl McpToolProvider for EchoToolProvider {
        fn tools(&self) -> Vec<ToolDefinition> {
            vec![ToolDefinition {
                name: "echo".to_string(),
                description: Some("Echoes back the input".to_string()),
                input_schema: serde_json::json!({"type": "object"}),
            }]
        }

        async fn call_tool(&self, name: &str, arguments: Value) -> Result<ToolCallResult> {
            if name == "echo" {
                Ok(ToolCallResult::text(arguments.to_string()))
            } else {
                Err(McpError::ToolNotFound(name.to_string()))
            }
        }
    }

    struct EchoResourceProvider;

    #[async_trait::async_trait]
    impl McpResourceProvider for EchoResourceProvider {
        fn resources(&self) -> Vec<ResourceDefinition> {
            vec![ResourceDefinition {
                uri: "echo://resource".to_string(),
                name: "echo".to_string(),
                description: None,
                mime_type: None,
            }]
        }

        async fn read_resource(&self, uri: &str) -> Result<ResourceContent> {
            if uri == "echo://resource" {
                Ok(ResourceContent::text(uri, "echoed content"))
            } else {
                Err(McpError::ResourceNotFound(uri.to_string()))
            }
        }
    }

    #[tokio::test]
    async fn test_tools_list_includes_registered_provider_tool() {
        let service = McpService::new().with_tool_provider(Arc::new(EchoToolProvider));
        let request = JsonRpcRequest::new("tools/list");

        let response = expect_response(service.handle_request(request, &empty_headers()).await);

        assert!(response.error.is_none());
        let result: ToolsListResult = serde_json::from_value(response.result.unwrap()).unwrap();
        assert!(result.tools.iter().any(|t| t.name == "echo"));
    }

    #[tokio::test]
    async fn test_tools_call_dispatches_to_provider() {
        let service = McpService::new().with_tool_provider(Arc::new(EchoToolProvider));
        let request = JsonRpcRequest::new("tools/call")
            .with_params(serde_json::json!({ "name": "echo", "arguments": {"a": 1} }));

        let response = expect_response(service.handle_request(request, &empty_headers()).await);

        assert!(response.error.is_none());
        let result: ToolCallResult = serde_json::from_value(response.result.unwrap()).unwrap();
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_tools_call_unknown_name_returns_tool_not_found_error() {
        let service = McpService::new().with_tool_provider(Arc::new(EchoToolProvider));
        let request = JsonRpcRequest::new("tools/call")
            .with_params(serde_json::json!({ "name": "does_not_exist", "arguments": {} }));

        let response = expect_response(service.handle_request(request, &empty_headers()).await);

        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32002);
    }

    #[tokio::test]
    async fn test_resources_list_includes_registered_provider_resource() {
        let service = McpService::new().with_resource_provider(Arc::new(EchoResourceProvider));
        let request = JsonRpcRequest::new("resources/list");

        let response = expect_response(service.handle_request(request, &empty_headers()).await);

        assert!(response.error.is_none());
        let result: ResourcesListResult = serde_json::from_value(response.result.unwrap()).unwrap();
        assert!(result.resources.iter().any(|r| r.uri == "echo://resource"));
    }

    #[tokio::test]
    async fn test_resources_read_dispatches_to_provider() {
        let service = McpService::new().with_resource_provider(Arc::new(EchoResourceProvider));
        let request = JsonRpcRequest::new("resources/read")
            .with_params(serde_json::json!({ "uri": "echo://resource" }));

        let response = expect_response(service.handle_request(request, &empty_headers()).await);

        assert!(response.error.is_none());
        let result: ResourceReadResult = serde_json::from_value(response.result.unwrap()).unwrap();
        assert_eq!(result.contents[0].text.as_deref(), Some("echoed content"));
    }

    struct NamedEchoToolProvider {
        name: &'static str,
        reply: &'static str,
    }

    #[async_trait::async_trait]
    impl McpToolProvider for NamedEchoToolProvider {
        fn tools(&self) -> Vec<ToolDefinition> {
            vec![ToolDefinition {
                name: self.name.to_string(),
                description: Some(self.reply.to_string()),
                input_schema: serde_json::json!({"type": "object"}),
            }]
        }

        async fn call_tool(&self, name: &str, _arguments: Value) -> Result<ToolCallResult> {
            if name == self.name {
                Ok(ToolCallResult::text(self.reply))
            } else {
                Err(McpError::ToolNotFound(name.to_string()))
            }
        }
    }

    struct NamedEchoResourceProvider {
        uri: &'static str,
        reply: &'static str,
    }

    #[async_trait::async_trait]
    impl McpResourceProvider for NamedEchoResourceProvider {
        fn resources(&self) -> Vec<ResourceDefinition> {
            vec![ResourceDefinition {
                uri: self.uri.to_string(),
                name: self.uri.to_string(),
                description: Some(self.reply.to_string()),
                mime_type: None,
            }]
        }

        async fn read_resource(&self, uri: &str) -> Result<ResourceContent> {
            if uri == self.uri {
                Ok(ResourceContent::text(uri, self.reply))
            } else {
                Err(McpError::ResourceNotFound(uri.to_string()))
            }
        }
    }

    #[tokio::test]
    async fn test_tools_list_dedups_colliding_provider_names_first_registered_wins() {
        // Two providers both expose a tool named "shared". The merge/dedup
        // logic (`merge_dedup_sorted`) must keep exactly one entry for the
        // name, and — matching registry-vs-provider precedence — the
        // first-registered provider's definition wins over later ones.
        let service = McpService::new()
            .with_tool_provider(Arc::new(NamedEchoToolProvider {
                name: "shared",
                reply: "first",
            }))
            .with_tool_provider(Arc::new(NamedEchoToolProvider {
                name: "shared",
                reply: "second",
            }));

        let request = JsonRpcRequest::new("tools/list");
        let response = expect_response(service.handle_request(request, &empty_headers()).await);

        assert!(response.error.is_none());
        let result: ToolsListResult = serde_json::from_value(response.result.unwrap()).unwrap();

        let matches: Vec<_> = result.tools.iter().filter(|t| t.name == "shared").collect();
        assert_eq!(matches.len(), 1, "expected exactly one deduped entry");
        assert_eq!(matches[0].description.as_deref(), Some("first"));
    }

    #[tokio::test]
    async fn test_resources_list_dedups_colliding_provider_uris_first_registered_wins() {
        // Symmetric to the tools case: two providers both expose a resource
        // at the same URI; the merged `resources/list` must contain exactly
        // one entry, with the first-registered provider's definition
        // winning.
        let service = McpService::new()
            .with_resource_provider(Arc::new(NamedEchoResourceProvider {
                uri: "shared://resource",
                reply: "first",
            }))
            .with_resource_provider(Arc::new(NamedEchoResourceProvider {
                uri: "shared://resource",
                reply: "second",
            }));

        let request = JsonRpcRequest::new("resources/list");
        let response = expect_response(service.handle_request(request, &empty_headers()).await);

        assert!(response.error.is_none());
        let result: ResourcesListResult = serde_json::from_value(response.result.unwrap()).unwrap();

        let matches: Vec<_> = result
            .resources
            .iter()
            .filter(|r| r.uri == "shared://resource")
            .collect();
        assert_eq!(matches.len(), 1, "expected exactly one deduped entry");
        assert_eq!(matches[0].description.as_deref(), Some("first"));
    }

    struct GreetingPromptProvider;

    #[async_trait::async_trait]
    impl McpPromptProvider for GreetingPromptProvider {
        fn prompts(&self) -> Vec<PromptDefinition> {
            vec![PromptDefinition {
                name: "greeting".to_string(),
                description: Some("Greet someone".to_string()),
                arguments: vec![PromptArgument {
                    name: "who".to_string(),
                    description: None,
                    required: true,
                }],
            }]
        }

        async fn get_prompt(&self, name: &str, arguments: Value) -> Result<PromptGetResult> {
            if name == "greeting" {
                let who = arguments
                    .get("who")
                    .and_then(|v| v.as_str())
                    .unwrap_or("world");
                Ok(PromptGetResult::text(format!("Hello, {who}!")))
            } else {
                Err(McpError::PromptNotFound(name.to_string()))
            }
        }
    }

    /// The `prompts` capability advertised by `initialize` must actually be
    /// servable: before this fix `prompts/list` and `prompts/get` had no
    /// dispatch arms at all and answered `-32601`.
    #[tokio::test]
    async fn test_prompts_list_and_get_are_served() {
        let service = McpService::new().with_prompt_provider(Arc::new(GreetingPromptProvider));

        let response = expect_response(
            service
                .handle_request(JsonRpcRequest::new("prompts/list"), &empty_headers())
                .await,
        );
        assert!(response.error.is_none());
        let listed: PromptsListResult = serde_json::from_value(response.result.unwrap()).unwrap();
        assert!(listed.prompts.iter().any(|p| p.name == "greeting"));

        let request = JsonRpcRequest::new("prompts/get")
            .with_params(serde_json::json!({ "name": "greeting", "arguments": {"who": "MCP"} }));
        let response = expect_response(service.handle_request(request, &empty_headers()).await);
        assert!(response.error.is_none());
        let got: PromptGetResult = serde_json::from_value(response.result.unwrap()).unwrap();
        assert_eq!(got.messages.len(), 1);
        assert_eq!(got.messages[0].role, PromptRole::User);
    }

    #[tokio::test]
    async fn test_prompts_get_unknown_name_returns_prompt_not_found_error() {
        let service = McpService::new().with_prompt_provider(Arc::new(GreetingPromptProvider));
        let request = JsonRpcRequest::new("prompts/get")
            .with_params(serde_json::json!({ "name": "does_not_exist" }));

        let response = expect_response(service.handle_request(request, &empty_headers()).await);

        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32002);
    }

    /// `initialize` may only advertise `prompts` when the server can serve
    /// them; the flag drives that, and both list/get are wired up above.
    #[tokio::test]
    async fn test_initialize_advertises_prompts_only_when_enabled() {
        let disabled = McpService::new();
        let response = expect_response(
            disabled
                .handle_request(JsonRpcRequest::new("initialize"), &empty_headers())
                .await,
        );
        let result: InitializeResult = serde_json::from_value(response.result.unwrap()).unwrap();
        assert!(result.capabilities.prompts.is_none());

        let enabled = McpService::with_config(McpConfig::default().with_prompts(true));
        let response = expect_response(
            enabled
                .handle_request(JsonRpcRequest::new("initialize"), &empty_headers())
                .await,
        );
        let result: InitializeResult = serde_json::from_value(response.result.unwrap()).unwrap();
        assert!(result.capabilities.prompts.is_some());
    }

    #[tokio::test]
    async fn test_resources_read_unknown_uri_returns_resource_not_found_error() {
        let service = McpService::new().with_resource_provider(Arc::new(EchoResourceProvider));
        let request = JsonRpcRequest::new("resources/read")
            .with_params(serde_json::json!({ "uri": "does://not-exist" }));

        let response = expect_response(service.handle_request(request, &empty_headers()).await);

        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32002);
    }
}
