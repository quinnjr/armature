use armature_mcp::types::ToolCallResult;
use armature_mcp::error::Result as McpResult;
use serde_json::Value;

async fn handler(_args: Value) -> McpResult<ToolCallResult> {
    Ok(ToolCallResult::text("ok"))
}

struct Owner;

armature_mcp::register_mcp_tool!(
    Owner,
    "ping",
    "ping",
    r#"{"type":"object"}"#,
    handler
);

fn main() {}
