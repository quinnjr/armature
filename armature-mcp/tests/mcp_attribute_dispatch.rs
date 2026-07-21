//! Real end-to-end coverage for the `#[mcp]` attribute macro.
//!
//! The trybuild `t.pass()` tests only assert that an annotated function
//! *compiles* (its `main` is empty). This binary instead defines a real
//! `#[mcp]`-annotated tool and drives the actual service to prove that the
//! macro's `inventory::submit!` entry (a) surfaces in the merged `tools/list`
//! output and (b) dispatches through `tools/call` and returns the expected
//! result.

use armature_mcp::mcp;
use armature_mcp::{ContentItem, JsonRpcRequest, McpService, ToolCallResult, ToolsListResult};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct GreetInput {
    name: String,
}

#[mcp(
    name = "greet_attr_tool",
    description = "Greets the caller by name (integration-test tool)"
)]
async fn greet_attr_tool(input: GreetInput) -> ToolCallResult {
    ToolCallResult::text(format!("Hello, {}!", input.name))
}

fn no_headers() -> HashMap<String, String> {
    HashMap::new()
}

#[tokio::test]
async fn mcp_attribute_tool_appears_in_tools_list() {
    let service = McpService::new();
    let request = JsonRpcRequest::new("tools/list");

    let response = service.handle_request(request, &no_headers()).await;
    assert!(response.error.is_none(), "tools/list returned an error");

    let result: ToolsListResult =
        serde_json::from_value(response.result.expect("result present")).unwrap();

    let tool = result
        .tools
        .iter()
        .find(|t| t.name == "greet_attr_tool")
        .expect("the #[mcp]-annotated tool must surface in tools/list");
    assert_eq!(
        tool.description.as_deref(),
        Some("Greets the caller by name (integration-test tool)")
    );
}

#[tokio::test]
async fn mcp_attribute_tool_dispatches_via_tools_call() {
    let service = McpService::new();
    let request = JsonRpcRequest::new("tools/call").with_params(serde_json::json!({
        "name": "greet_attr_tool",
        "arguments": { "name": "Ada" }
    }));

    let response = service.handle_request(request, &no_headers()).await;
    assert!(response.error.is_none(), "tools/call returned an error");

    let result: ToolCallResult =
        serde_json::from_value(response.result.expect("result present")).unwrap();
    assert!(!result.is_error, "tool call reported an error result");

    // `ContentItem` is `#[non_exhaustive]`, so match with a fallback arm.
    let text = result.content.iter().find_map(|item| match item {
        ContentItem::Text { text } => Some(text.clone()),
        _ => None,
    });
    assert_eq!(text.as_deref(), Some("Hello, Ada!"));
}
