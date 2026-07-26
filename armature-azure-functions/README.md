# armature-azure-functions

Azure Functions runtime adapter for the Armature framework.

## Features

- **Functions Runtime** - Run Armature apps on Azure Functions
- **HTTP Triggers** - Handle HTTP events via the custom-handler HTTP server
- **Application Insights** - Structured JSON logging for monitoring

## Installation

```toml
[dependencies]
armature-azure-functions = "0.1"
```

## Quick Start

```rust
use armature_azure_functions::AzureFunctionsRuntime;
use armature_core::Application;

#[tokio::main]
async fn main() {
    let app = Application::new()
        .get("/api/hello", |_| async {
            Ok(HttpResponse::ok().with_text("Hello from Azure!"))
        });

    AzureFunctionsRuntime::new(app).run().await;
}
```

## License

MIT OR Apache-2.0

