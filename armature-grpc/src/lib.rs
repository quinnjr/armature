// Allow dead_code for now as this crate is still under development
#![allow(dead_code)]
// Status from tonic is inherently large; this is acceptable for error handling
#![allow(clippy::result_large_err)]

//! # Armature gRPC
//!
//! gRPC server and client support for Armature applications.
//!
//! ## Features
//!
//! - **Server**: Build gRPC servers with middleware support
//! - **Client**: Type-safe gRPC client with load balancing and *opt-in*
//!   retry — retry does not happen automatically; every call must be
//!   explicitly wrapped via [`GrpcChannel::call_with_retry`](client::GrpcChannel::call_with_retry).
//!   See "Retry" below.
//! - **Interceptors**: Request/response interceptors for auth, logging, etc.
//! - **Health Checking**: Built-in gRPC health checking service
//! - **Reflection**: Server reflection for tools like grpcurl
//! - **Compression**: gzip and zstd compression support, via
//!   [`CompressionMiddleware::wrap_server`]/[`CompressionMiddleware::wrap_channel`]
//!
//! ## Quick Start
//!
//! ### Server
//!
//! ```rust,ignore
//! use armature_grpc::{GrpcServer, GrpcServerConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = GrpcServerConfig::builder()
//!         .bind_address("0.0.0.0:50051")
//!         .enable_health_check()
//!         .enable_reflection()
//!         .build()?;
//!
//!     GrpcServer::builder(config)
//!         .serve(MyServiceServer::new(MyServiceImpl))
//!         .await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Client
//!
//! ```rust,ignore
//! use armature_grpc::{GrpcClient, GrpcClientConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = GrpcClientConfig::builder()
//!         .endpoint("http://localhost:50051")
//!         .timeout(std::time::Duration::from_secs(30))
//!         .build();
//!
//!     let channel = GrpcClient::connect(config).await?;
//!     let mut client = GreeterClient::new(channel.inner().clone());
//!
//!     // Use the client...
//!     Ok(())
//! }
//! ```
//!
//! ### Retry
//!
//! `GrpcClientConfig::retry_enabled` / `max_retry_attempts` are **not**
//! applied automatically — tonic's generated client methods have no generic
//! hook for this crate to intercept. Retry only happens when you explicitly
//! wrap each call via [`GrpcChannel::call_with_retry`](client::GrpcChannel::call_with_retry):
//!
//! ```rust,ignore
//! let channel = GrpcClient::connect(config).await?;
//! let response = channel
//!     .call_with_retry(|| {
//!         let mut client = GreeterClient::new(channel.inner().clone());
//!         let request = HelloRequest { name: "World".into() };
//!         async move { client.say_hello(request).await }
//!     })
//!     .await?;
//! ```

mod client;
mod config;
mod crypto_provider;
mod error;
mod interceptor;
mod middleware;
mod server;

/// Type alias for the body type produced by tonic services. This is the same
/// body type tonic-build's generated `<Service>Server<T>` types use, so our
/// own service wrappers in `server` and `middleware` stay drop-in compatible
/// with real generated services (and with tonic's own `Server::add_service`,
/// which requires `S::Response: axum::response::IntoResponse` — satisfied by
/// `http::Response<TonicBody>`).
pub(crate) type TonicBody = tonic::body::Body;

pub use client::{GrpcChannel, GrpcClient};
pub use config::{
    GrpcClientConfig, GrpcClientConfigBuilder, GrpcClientTlsConfig, GrpcServerConfig,
    GrpcServerConfigBuilder, GrpcServerTlsConfig,
};
pub use error::{GrpcError, Result};
pub use interceptor::{
    AuthInterceptor, Interceptor, LoggingInterceptor, MetricsInterceptor, RequestInterceptor,
    ResponseInterceptor,
};
pub use middleware::{
    CompressionClientService, CompressionEncoding, CompressionMiddleware, CompressionService,
    ConcurrencyLimitMiddleware, GrpcMiddleware, LoadSheddingMiddleware, RateLimitMiddleware,
    RetryMiddleware, TimeoutMiddleware,
};
pub use server::{GrpcServer, GrpcServerBuilder};

// Re-export tonic types
pub use tonic::{
    Code, Request, Response, Status,
    metadata::{MetadataMap, MetadataValue},
    transport::{Channel, Endpoint, Server},
};

#[cfg(feature = "health")]
pub use tonic_health;

#[cfg(feature = "reflection")]
pub use tonic_reflection;

/// Prelude for common imports.
///
/// ```
/// use armature_grpc::prelude::*;
/// ```
pub mod prelude {
    pub use crate::client::{GrpcChannel, GrpcClient};
    pub use crate::config::{
        GrpcClientConfig, GrpcClientTlsConfig, GrpcServerConfig, GrpcServerTlsConfig,
    };
    pub use crate::error::{GrpcError, Result};
    pub use crate::interceptor::{
        AuthInterceptor, Interceptor, LoggingInterceptor, MetricsInterceptor,
    };
    pub use crate::middleware::{
        CompressionEncoding, CompressionMiddleware, ConcurrencyLimitMiddleware, GrpcMiddleware,
        LoadSheddingMiddleware, RateLimitMiddleware, RetryMiddleware, TimeoutMiddleware,
    };
    pub use crate::server::{GrpcServer, GrpcServerBuilder};
    pub use tonic::{
        Code, Request, Response, Status,
        metadata::{MetadataMap, MetadataValue},
        transport::{Channel, Endpoint, Server},
    };
}
