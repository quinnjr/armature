# armature-grpc

gRPC server and client support for the Armature framework.

## Features

- **Tonic Integration** - Built on the Tonic gRPC library
- **Protobuf compilation** - Not performed by this crate. Depend on
  [`tonic-build`](https://docs.rs/tonic-build) directly in your own crate's
  `build.rs` to generate service code from `.proto` files, then use the
  generated types together with `armature-grpc`'s server/client/interceptor
  helpers.
- **Streaming** - Unary, server, client, and bidirectional streaming
- **Interceptors** - Request/response middleware
- **TLS** - Secure client and server connections with rustls (`tonic`'s
  `tls-ring` feature), configured via `GrpcClientTlsConfig` / `GrpcServerTlsConfig`
- **Retry** - *Opt-in*: `GrpcClientConfig::retry_enabled` / `max_retry_attempts`
  do nothing on their own. Retry only happens for calls explicitly wrapped in
  `GrpcChannel::call_with_retry` (see below) — there is no automatic retry.
- **Compression** - gzip/zstd, applied via `CompressionMiddleware::wrap_server`
  (server) / `CompressionMiddleware::wrap_channel` (client)

## Installation

```toml
[dependencies]
armature-grpc = "0.1"
```

## Quick Start

### Server

```rust,ignore
use armature_grpc::{GrpcServer, GrpcServerConfig, Request, Response, Status};

pub struct MyService;

#[tonic::async_trait]
impl Greeter for MyService {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let reply = HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = GrpcServerConfig::builder()
        .bind_address("0.0.0.0:50051")
        .build()?;

    GrpcServer::builder(config)
        .serve(GreeterServer::new(MyService))
        .await?;

    Ok(())
}
```

### Client

```rust,ignore
use armature_grpc::{GrpcClient, GrpcClientConfig};

let config = GrpcClientConfig::builder()
    .endpoint("http://localhost:50051")
    .build();
let channel = GrpcClient::connect(config).await?;
let mut client = GreeterClient::new(channel.inner().clone());
let response = client.say_hello(HelloRequest { name: "World".into() }).await?;
println!("Response: {}", response.into_inner().message);
```

### Retry

`retry_enabled` / `max_retry_attempts` on `GrpcClientConfig` are not applied
automatically. To actually retry, wrap each call in `GrpcChannel::call_with_retry`:

```rust,ignore
let config = GrpcClientConfig::builder()
    .endpoint("http://localhost:50051")
    .retry(true)
    .max_retry_attempts(3)
    .build();
let channel = GrpcClient::connect(config).await?;

let response = channel
    .call_with_retry(|| {
        let mut client = GreeterClient::new(channel.inner().clone());
        let request = HelloRequest { name: "World".into() };
        async move { client.say_hello(request).await }
    })
    .await?;
```

## License

MIT OR Apache-2.0

