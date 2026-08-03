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

## Adapting an existing application type

This crate does **not** convert between `armature_core`'s `HttpRequest` /
`HttpResponse` and the Lambda event types, and there is no blanket
`RequestHandler` implementation for an Armature `Application`. What it offers is
`impl_lambda_handler!`, which removes the trait boilerplate around a
`handle_request` method **you** write:

```rust
use armature_lambda::{impl_lambda_handler, LambdaRequest, LambdaRuntime};

struct MyApp { /* your Armature Application, router, etc. */ }

struct MyResponse {
    status: u16,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
}

impl MyApp {
    // You write this: translate `LambdaRequest` into whatever your application
    // consumes, and its result back into the shape below.
    async fn handle_request(
        &self,
        request: LambdaRequest,
    ) -> Result<MyResponse, std::io::Error> {
        // ...
    }
}

impl_lambda_handler!(MyApp);

#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    armature_lambda::init_tracing();
    LambdaRuntime::new(MyApp { /* .. */ }).run().await
}
```

The macro forwards the whole `LambdaRequest` — method, path, headers, query
string, path parameters, stage variables, and authorizer claims — so nothing is
lost on the way in, and maps `status`/`body`/`headers` back out (any
`Display` error becomes a 500).

The runtime auto-detects API Gateway (REST v1 / HTTP v2), ALB, and Lambda
Function URL events — no separate constructor is required.

## Headers

Request and response headers are `Vec<(String, String)>`, not maps, so repeated
names survive in both directions. In particular a handler can emit more than one
`Set-Cookie`:

```rust
LambdaResponse::ok("done")
    .header("set-cookie", "session=abc; HttpOnly")
    .header("set-cookie", "csrf=xyz");
```

`header(..)` appends; use `set_header(..)` to replace existing lines with the
same name. Read them back with `header_value(..)` (first match) or
`header_values(..)` (all, in order).

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

