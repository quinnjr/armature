//! MCP Service implementation
//!
//! Handles JSON-RPC 2.0 requests for the Model Context Protocol.

use crate::error::{McpError, Result};
use crate::resource::McpResourceRegistry;
use crate::tool::McpToolRegistry;
use crate::types::*;
use serde_json::Value;

/// MCP protocol version
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Configuration for the MCP service
#[derive(Debug, Clone)]
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
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            server_name: "armature-mcp".to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            enable_tools: true,
            enable_resources: true,
            enable_prompts: false,
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
}

/// MCP service that handles protocol requests
pub struct McpService {
    config: McpConfig,
    tool_registry: McpToolRegistry,
    resource_registry: McpResourceRegistry,
}

impl McpService {
    /// Create a new MCP service with default configuration
    pub fn new() -> Self {
        Self {
            config: McpConfig::default(),
            tool_registry: McpToolRegistry::new(),
            resource_registry: McpResourceRegistry::new(),
        }
    }

    /// Create a new MCP service with custom configuration
    pub fn with_config(config: McpConfig) -> Self {
        Self {
            config,
            tool_registry: McpToolRegistry::new(),
            resource_registry: McpResourceRegistry::new(),
        }
    }

    /// Handle a JSON-RPC request and return a response
    pub async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let id = request.id.clone();

        match self.dispatch_method(&request.method, request.params).await {
            Ok(result) => JsonRpcResponse::success(id, result),
            Err(e) => JsonRpcResponse::error(
                id,
                JsonRpcError::new(e.to_error_code(), e.to_string()),
            ),
        }
    }

    /// Handle a raw JSON request string
    pub async fn handle_json(&self, json: &str) -> String {
        let request: std::result::Result<JsonRpcRequest, _> = serde_json::from_str(json);

        let response = match request {
            Ok(req) => self.handle_request(req).await,
            Err(e) => JsonRpcResponse::error(
                None,
                JsonRpcError::parse_error(e.to_string()),
            ),
        };

        serde_json::to_string(&response).unwrap_or_else(|_| {
            r#"{"jsonrpc":"2.0","error":{"code":-32603,"message":"Internal error"}}"#.to_string()
        })
    }

    /// Dispatch a method call to the appropriate handler
    async fn dispatch_method(&self, method: &str, params: Option<Value>) -> Result<Value> {
        match method {
            "initialize" => self.handle_initialize(params),
            "tools/list" => self.handle_tools_list(params),
            "tools/call" => self.handle_tools_call(params).await,
            "resources/list" => self.handle_resources_list(params),
            "resources/read" => self.handle_resources_read(params).await,
            "ping" => Ok(serde_json::json!({})),
            _ => Err(McpError::MethodNotFound(method.to_string())),
        }
    }

    /// Handle initialize request
    fn handle_initialize(&self, _params: Option<Value>) -> Result<Value> {
        let mut capabilities = ServerCapabilities::default();

        if self.config.enable_tools {
            capabilities.tools = Some(ToolsCapability { list_changed: false });
        }

        if self.config.enable_resources {
            capabilities.resources = Some(ResourcesCapability {
                subscribe: false,
                list_changed: false,
            });
        }

        if self.config.enable_prompts {
            capabilities.prompts = Some(PromptsCapability { list_changed: false });
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
    fn handle_tools_list(&self, _params: Option<Value>) -> Result<Value> {
        let tools = self.tool_registry.list_tools();

        let result = ToolsListResult {
            tools,
            next_cursor: None,
        };

        serde_json::to_value(result).map_err(McpError::from)
    }

    /// Handle tools/call request
    async fn handle_tools_call(&self, params: Option<Value>) -> Result<Value> {
        let params: ToolCallParams = params
            .ok_or_else(|| McpError::InvalidParams("Missing params".to_string()))
            .and_then(|v| serde_json::from_value(v).map_err(|e| McpError::InvalidParams(e.to_string())))?;

        let result = self.tool_registry.call_tool(&params.name, params.arguments).await?;

        serde_json::to_value(result).map_err(McpError::from)
    }

    /// Handle resources/list request
    fn handle_resources_list(&self, _params: Option<Value>) -> Result<Value> {
        let resources = self.resource_registry.list_resources();

        let result = ResourcesListResult {
            resources,
            next_cursor: None,
        };

        serde_json::to_value(result).map_err(McpError::from)
    }

    /// Handle resources/read request
    async fn handle_resources_read(&self, params: Option<Value>) -> Result<Value> {
        let params: ResourceReadParams = params
            .ok_or_else(|| McpError::InvalidParams("Missing params".to_string()))
            .and_then(|v| serde_json::from_value(v).map_err(|e| McpError::InvalidParams(e.to_string())))?;

        let content = self.resource_registry.read_resource(&params.uri).await?;

        let result = ResourceReadResult {
            contents: vec![content],
        };

        serde_json::to_value(result).map_err(McpError::from)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_handle_initialize() {
        let service = McpService::new();
        let request = JsonRpcRequest::new("initialize");

        let response = service.handle_request(request).await;

        assert!(response.error.is_none());
        assert!(response.result.is_some());

        let result: InitializeResult =
            serde_json::from_value(response.result.unwrap()).unwrap();
        assert_eq!(result.protocol_version, MCP_PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn test_handle_ping() {
        let service = McpService::new();
        let request = JsonRpcRequest::new("ping");

        let response = service.handle_request(request).await;

        assert!(response.error.is_none());
    }

    #[tokio::test]
    async fn test_handle_tools_list() {
        let service = McpService::new();
        let request = JsonRpcRequest::new("tools/list");

        let response = service.handle_request(request).await;

        assert!(response.error.is_none());
        assert!(response.result.is_some());
    }

    #[tokio::test]
    async fn test_handle_unknown_method() {
        let service = McpService::new();
        let request = JsonRpcRequest::new("unknown/method");

        let response = service.handle_request(request).await;

        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }

    #[tokio::test]
    async fn test_handle_json() {
        let service = McpService::new();
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;

        let response = service.handle_json(json).await;

        assert!(response.contains("result"));
    }

    #[tokio::test]
    async fn test_handle_invalid_json() {
        let service = McpService::new();
        let json = "not valid json";

        let response = service.handle_json(json).await;

        assert!(response.contains("error"));
        assert!(response.contains("-32700")); // Parse error
    }
}
