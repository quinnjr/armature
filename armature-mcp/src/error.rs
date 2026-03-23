//! MCP error types

use thiserror::Error;

/// MCP-specific errors
#[derive(Error, Debug)]
pub enum McpError {
    #[error("Tool not found: {0}")]
    ToolNotFound(String),

    #[error("Resource not found: {0}")]
    ResourceNotFound(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Invalid parameters: {0}")]
    InvalidParams(String),

    #[error("Tool execution failed: {0}")]
    ToolExecutionFailed(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Method not found: {0}")]
    MethodNotFound(String),

    #[error("Parse error: {0}")]
    ParseError(String),
}

impl McpError {
    /// Convert to JSON-RPC error code
    pub fn to_error_code(&self) -> i32 {
        match self {
            McpError::ParseError(_) => -32700,
            McpError::InvalidRequest(_) => -32600,
            McpError::MethodNotFound(_) => -32601,
            McpError::InvalidParams(_) => -32602,
            McpError::Internal(_) => -32603,
            McpError::ToolNotFound(_) => -32002,
            McpError::ResourceNotFound(_) => -32002,
            McpError::ToolExecutionFailed(_) => -32000,
            McpError::Serialization(_) => -32700,
        }
    }
}

/// Result type alias for MCP operations
pub type Result<T> = std::result::Result<T, McpError>;
