use armature_mcp::mcp;
use armature_mcp::types::ToolCallResult;
use serde::Deserialize;

#[derive(Deserialize)]
struct EchoInput {
    value: String,
}

#[mcp(name = "echo", description = "Echoes the input value")]
async fn echo(input: EchoInput) -> ToolCallResult {
    ToolCallResult::text(input.value)
}

fn main() {}
