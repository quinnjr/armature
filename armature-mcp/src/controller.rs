//! MCP Controller for HTTP endpoint
//!
//! Provides an auto-registering `/mcp` endpoint that handles MCP JSON-RPC requests.

use crate::service::{McpConfig, McpService};
use armature_core::{Error, HttpRequest, HttpResponse};
use std::sync::Arc;

/// MCP Controller that provides the `/mcp` HTTP endpoint
///
/// This controller handles JSON-RPC 2.0 requests for the Model Context Protocol,
/// enabling AI clients like Cursor and Claude to discover and invoke tools.
///
/// # Usage
///
/// ```ignore
/// use armature_mcp::{McpController, McpConfig};
/// use armature::{module, Application};
///
/// #[module(
///     controllers: [McpController]
/// )]
/// struct AppModule;
///
/// #[tokio::main]
/// async fn main() {
///     Application::create::<AppModule>()
///         .listen("127.0.0.1:3000")
///         .await;
/// }
/// ```
#[derive(Clone)]
pub struct McpController {
    service: Arc<McpService>,
}

impl McpController {
    /// Create a new MCP controller with default configuration
    pub fn new() -> Self {
        Self {
            service: Arc::new(McpService::new()),
        }
    }

    /// Create a new MCP controller with custom configuration
    pub fn with_config(config: McpConfig) -> Self {
        Self {
            service: Arc::new(McpService::with_config(config)),
        }
    }

    /// Get a reference to the MCP service
    pub fn service(&self) -> &McpService {
        &self.service
    }

    /// Handle POST /mcp - JSON-RPC endpoint
    pub async fn handle_request(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        let body = String::from_utf8(req.body.clone())
            .map_err(|e| Error::BadRequest(format!("Invalid UTF-8: {}", e)))?;

        let response_json = self.service.handle_json(&body).await;

        Ok(HttpResponse::ok()
            .with_header("Content-Type".to_string(), "application/json".to_string())
            .with_body(response_json.into_bytes()))
    }

    /// Handle GET /mcp - Returns server info and capabilities
    pub async fn handle_info(&self) -> Result<HttpResponse, Error> {
        let config = self.service.config();
        let tools = self.service.tools().list_tools();
        let resources = self.service.resources().list_resources();

        let info = serde_json::json!({
            "name": config.server_name,
            "version": config.server_version,
            "protocol": crate::service::MCP_PROTOCOL_VERSION,
            "capabilities": {
                "tools": config.enable_tools,
                "resources": config.enable_resources,
                "prompts": config.enable_prompts
            },
            "tools_count": tools.len(),
            "resources_count": resources.len(),
            "endpoints": {
                "jsonrpc": "POST /mcp",
                "info": "GET /mcp",
                "tools": "GET /mcp/tools",
                "resources": "GET /mcp/resources"
            }
        });

        Ok(HttpResponse::ok()
            .with_header("Content-Type".to_string(), "application/json".to_string())
            .with_body(serde_json::to_string_pretty(&info).unwrap().into_bytes()))
    }

    /// Handle GET /mcp/tools - List all available tools
    pub async fn handle_list_tools(&self) -> Result<HttpResponse, Error> {
        let tools = self.service.tools().list_tools();

        let response = serde_json::json!({
            "tools": tools
        });

        Ok(HttpResponse::ok()
            .with_header("Content-Type".to_string(), "application/json".to_string())
            .with_body(serde_json::to_string_pretty(&response).unwrap().into_bytes()))
    }

    /// Handle GET /mcp/resources - List all available resources
    pub async fn handle_list_resources(&self) -> Result<HttpResponse, Error> {
        let resources = self.service.resources().list_resources();

        let response = serde_json::json!({
            "resources": resources
        });

        Ok(HttpResponse::ok()
            .with_header("Content-Type".to_string(), "application/json".to_string())
            .with_body(serde_json::to_string_pretty(&response).unwrap().into_bytes()))
    }
}

impl Default for McpController {
    fn default() -> Self {
        Self::new()
    }
}

/// Extension trait to add MCP routes to a Router
pub trait McpRouterExt {
    /// Add MCP endpoint routes to the router
    fn with_mcp(self) -> Self;
    
    /// Add MCP endpoint routes with custom configuration
    fn with_mcp_config(self, config: McpConfig) -> Self;
}

impl McpRouterExt for armature_core::routing::Router {
    fn with_mcp(self) -> Self {
        self.with_mcp_config(McpConfig::default())
    }

    fn with_mcp_config(mut self, config: McpConfig) -> Self {
        let controller = Arc::new(McpController::with_config(config));

        // POST /mcp - JSON-RPC endpoint
        let ctrl = controller.clone();
        self.post("/mcp", move |req: HttpRequest| {
            let ctrl = ctrl.clone();
            async move { ctrl.handle_request(req).await }
        });

        // GET /mcp - Server info
        let ctrl = controller.clone();
        self.get("/mcp", move |_req: HttpRequest| {
            let ctrl = ctrl.clone();
            async move { ctrl.handle_info().await }
        });

        // GET /mcp/tools - List tools
        let ctrl = controller.clone();
        self.get("/mcp/tools", move |_req: HttpRequest| {
            let ctrl = ctrl.clone();
            async move { ctrl.handle_list_tools().await }
        });

        // GET /mcp/resources - List resources
        let ctrl = controller;
        self.get("/mcp/resources", move |_req: HttpRequest| {
            let ctrl = ctrl.clone();
            async move { ctrl.handle_list_resources().await }
        });

        self
    }
}
