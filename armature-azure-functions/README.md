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

The runtime drives any type implementing `RequestHandler`. The simplest handler
is a closure taking a `FunctionRequest` and returning a `FunctionResponse`:

```rust
use armature_azure_functions::{AzureFunctionsRuntime, FunctionRequest, FunctionResponse};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    armature_azure_functions::init_tracing();

    let handler = |req: FunctionRequest| async move {
        FunctionResponse::with_body(200, format!("Hello from {}!", req.path))
    };

    AzureFunctionsRuntime::new(handler).run().await?;
    Ok(())
}
```

## Adapting an existing application type

This crate does **not** convert between `armature_core`'s `HttpRequest` /
`HttpResponse` and the Azure Functions types, and there is no blanket
`RequestHandler` implementation for an Armature `Application`. What it offers is
`impl_azure_function_handler!`, which removes the trait boilerplate around a
`handle_request` method **you** write:

```rust
use armature_azure_functions::{impl_azure_function_handler, FunctionRequest};

struct MyApp { /* your Armature Application, router, etc. */ }

struct MyResponse {
    status: u16,
    body: String,
    headers: Vec<(String, String)>,
}

impl MyApp {
    // You write this: translate `FunctionRequest` into whatever your
    // application consumes, and its result back into the shape below.
    async fn handle_request(
        &self,
        request: FunctionRequest,
    ) -> Result<MyResponse, std::io::Error> {
        // ...
    }
}

impl_azure_function_handler!(MyApp);
```

The macro forwards the whole `FunctionRequest` — headers, query string, route
parameters and invocation context included — so the application can see
`Authorization`, `Content-Type`, cookies and the query, not just the method,
path and body. Any `Display` error becomes a 500.

## Headers and query parameters

Request headers, response headers and query parameters are all
`Vec<(String, String)>` rather than maps, so repeated names survive in wire
order. In particular a handler can emit more than one `Set-Cookie`:

```rust
FunctionResponse::ok()
    .header("set-cookie", "session=abc; HttpOnly")
    .header("set-cookie", "csrf=xyz");
```

`header(..)` appends; use `set_header(..)` to replace lines with the same name.
Read them back with `header_value(..)` / `header_values(..)` and
`query_param(..)` / `query_params(..)`.

A query pair with no `=` keeps its key with an empty value (`?flag`), and a
component whose percent-escapes do not decode is passed through verbatim with a
`warn`-level log rather than being silently replaced with an empty string.

## Configuration

`FunctionConfig` (populated from the environment by `FunctionConfig::from_env`)
controls the base path to strip, the maximum request body size, and the request
timeout. `timeout_seconds` is a single budget covering both reading the request
body and running the handler; `0` disables it.

## License

MIT OR Apache-2.0

