# armature-lambda

AWS Lambda runtime adapter for the Armature framework.

## Features

- **Lambda Runtime** - Run Armature apps on Lambda
- **API Gateway** - HTTP event handling
- **ALB** - Application Load Balancer support
- **Cold Start Optimization** - Minimal startup time
- **Layers** - Shared dependencies

## Installation

```toml
[dependencies]
armature-lambda = "0.1"
```

## Quick Start

The runtime wraps any type that implements `RequestHandler`. The simplest
handler is a closure taking a `LambdaRequest` and returning a `LambdaResponse`:

```rust
use armature_lambda::{LambdaRequest, LambdaResponse, LambdaRuntime};

#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    armature_lambda::init_tracing();

    let handler = |req: LambdaRequest| async move {
        LambdaResponse::ok(format!("Hello from {}!", req.path))
    };

    LambdaRuntime::new(handler).run().await
}
```

## Configuration

Use `LambdaConfig` (via `with_config`) to control logging and strip an
API Gateway stage prefix such as `/prod`:

```rust
use armature_lambda::{LambdaConfig, LambdaRuntime};

let config = LambdaConfig::default()
    .log_requests(true)
    .log_responses(false)
    .base_path("/prod");

// `app` is any type implementing `RequestHandler`.
let runtime = LambdaRuntime::new(app).with_config(config);
runtime.run().await?;
```

## API Gateway Integration

An Armature `Application` becomes a handler via the `impl_request_handler!`
macro, which forwards the full request (method, path, headers, query string,
path parameters, stage variables, and authorizer claims) to your app:

```rust
use armature::prelude::*;
use armature_lambda::{impl_request_handler, LambdaRuntime};

// #[module(controllers: [HelloController])]
// struct AppModule;

impl_request_handler!(MyApplication);

#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    armature_lambda::init_tracing();

    let app = Application::create::<AppModule>();

    LambdaRuntime::new(app).run().await
}
```

The runtime auto-detects API Gateway (REST v1 / HTTP v2), ALB, and Lambda
Function URL events — no separate constructor is required.

## Build for Lambda

```bash
# Install cargo-lambda
cargo install cargo-lambda

# Build
cargo lambda build --release

# Deploy
cargo lambda deploy my-function
```

## License

MIT OR Apache-2.0

