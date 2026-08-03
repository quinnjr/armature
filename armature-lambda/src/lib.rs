//! # Armature Lambda
//!
//! AWS Lambda runtime adapter for Armature applications.
//!
//! This crate runs an HTTP handler on AWS Lambda behind API Gateway, ALB, or
//! Lambda Function URLs, translating the incoming event into a
//! [`LambdaRequest`] and the handler's [`LambdaResponse`] back out.
//!
//! ## What this crate does and does not do
//!
//! [`LambdaRuntime`] drives any type implementing [`RequestHandler`], which is
//! stated in this crate's own [`LambdaRequest`]/[`LambdaResponse`] types. It
//! does **not** convert to or from `armature_core::HttpRequest` /
//! `HttpResponse`, and there is no blanket implementation for an Armature
//! `Application` — wiring one up is the application author's job, most easily
//! via `impl_lambda_handler!` (see its documentation for the exact shape it
//! expects).
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature_lambda::{LambdaRequest, LambdaResponse, LambdaRuntime};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), lambda_runtime::Error> {
//!     // Initialize tracing for CloudWatch
//!     armature_lambda::init_tracing();
//!
//!     let handler = |req: LambdaRequest| async move {
//!         LambdaResponse::ok(format!("Hello from {}!", req.path))
//!     };
//!
//!     LambdaRuntime::new(handler).run().await
//! }
//! ```
//!
//! ## Deployment
//!
//! Build for Lambda with:
//!
//! ```bash
//! # Install cargo-lambda
//! cargo install cargo-lambda
//!
//! # Build for Lambda
//! cargo lambda build --release
//!
//! # Deploy
//! cargo lambda deploy
//! ```

mod error;
mod request;
mod response;
mod runtime;

pub use error::{LambdaError, Result};
pub use request::{LambdaRequest, RequestContext};
pub use response::LambdaResponse;
pub use runtime::{LambdaConfig, LambdaRuntime, RequestHandler};

// Re-export lambda types
pub use lambda_http;
pub use lambda_runtime;

// Re-exported so `impl_lambda_handler!` can name the attribute macro through
// `$crate`. Without this the macro would only expand in crates that happen to
// depend on `async-trait` themselves and under that exact name.
pub use async_trait;

/// Initialize tracing for Lambda/CloudWatch.
///
/// This sets up structured JSON logging suitable for CloudWatch Logs.
pub fn init_tracing() {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().json().flatten_event(true))
        .init();
}

/// Initialize tracing with a custom log level.
pub fn init_tracing_with_level(level: &str) {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    let filter = tracing_subscriber::EnvFilter::new(level);

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().json().flatten_event(true))
        .init();
}
