//! Model Context Protocol (MCP) Support for Armature
//!
//! This crate provides MCP server capabilities for Armature applications,
//! enabling AI clients like Cursor and Claude to discover and invoke tools
//! exposed by your application.
//!
//! # Features
//!
//! - 🔧 **Tool Discovery** - Auto-register tools with `#[mcp]` attribute
//! - 📦 **Resource Exposure** - Share data with AI clients
//! - 🔌 **Auto Endpoint** - `/mcp` endpoint automatically added
//! - 📡 **JSON-RPC 2.0** - Full MCP protocol compliance
//!
//! # Quick Start
//!
//! ## 1. Add MCP to your application
//!
//! ```ignore
//! use armature_mcp::McpRouterExt;
//!
//! let router = Router::new()
//!     .with_mcp()  // Adds /mcp endpoint
//!     .get("/", home_handler);
//! ```
//!
//! ## 2. Define MCP tools with the `#[mcp]` attribute
//!
//! ```ignore
//! use armature_mcp::{mcp, ToolCallResult};
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Deserialize)]
//! struct WeatherInput {
//!     location: String,
//! }
//!
//! #[mcp(
//!     name = "get_weather",
//!     description = "Get current weather for a location"
//! )]
//! async fn get_weather(input: WeatherInput) -> ToolCallResult {
//!     let weather = fetch_weather(&input.location).await;
//!     ToolCallResult::text(format!("Weather in {}: {}", input.location, weather))
//! }
//! ```
//!
//! ## 3. Register tools with the macro
//!
//! ```ignore
//! use armature_mcp::register_mcp_tool;
//!
//! struct MyTools;
//!
//! register_mcp_tool!(
//!     MyTools,
//!     "calculate",
//!     "Perform a calculation",
//!     r#"{"type": "object", "properties": {"expression": {"type": "string"}}}"#,
//!     calculate_handler
//! );
//! ```
//!
//! # Protocol Endpoints
//!
//! When using `with_mcp()`, the following endpoints are added:
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | POST | `/mcp` | JSON-RPC 2.0 endpoint for MCP requests |
//! | GET | `/mcp` | Server info and capabilities |
//! | GET | `/mcp/tools` | List available tools |
//! | GET | `/mcp/resources` | List available resources |
//!
//! # MCP Methods Supported
//!
//! - `initialize` - Initialize the MCP connection
//! - `tools/list` - List available tools
//! - `tools/call` - Invoke a tool
//! - `resources/list` - List available resources
//! - `resources/read` - Read a resource
//! - `ping` - Health check
//!
//! # Example Request
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "id": 1,
//!   "method": "tools/list"
//! }
//! ```
//!
//! # Example Response
//!
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "id": 1,
//!   "result": {
//!     "tools": [
//!       {
//!         "name": "get_weather",
//!         "description": "Get current weather for a location",
//!         "inputSchema": {
//!           "type": "object",
//!           "properties": {
//!             "location": { "type": "string" }
//!           },
//!           "required": ["location"]
//!         }
//!       }
//!     ]
//!   }
//! }
//! ```

pub mod controller;
pub mod error;
pub mod resource;
pub mod service;
pub mod tool;
pub mod types;

// Re-export inventory for macro usage
pub use inventory;

// Re-export main types
pub use controller::{McpController, McpRouterExt};
pub use error::{McpError, Result};
pub use resource::{McpResourceEntry, McpResourceProvider, McpResourceRegistry};
pub use service::{McpConfig, McpService, MCP_PROTOCOL_VERSION};
pub use tool::{McpToolEntry, McpToolProvider, McpToolRegistry, ToolHandlerFn};
pub use types::*;

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::controller::{McpController, McpRouterExt};
    pub use crate::error::{McpError, Result};
    pub use crate::resource::{McpResourceEntry, McpResourceProvider, McpResourceRegistry};
    pub use crate::service::{McpConfig, McpService};
    pub use crate::tool::{McpToolEntry, McpToolProvider, McpToolRegistry};
    pub use crate::types::{
        ContentItem, ResourceContent, ResourceDefinition, ToolCallResult, ToolDefinition,
    };
    pub use crate::{register_mcp_resource, register_mcp_tool};
}
