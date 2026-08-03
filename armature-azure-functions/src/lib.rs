// Allow dead_code while crate is under development
#![allow(dead_code)]
//! # Armature Azure Functions
//!
//! Azure Functions runtime adapter for Armature applications.
//!
//! This crate runs an HTTP handler as an Azure Functions custom handler,
//! translating each HTTP trigger invocation into a [`FunctionRequest`] and the
//! handler's [`FunctionResponse`] back out.
//!
//! ## What this crate does and does not do
//!
//! [`AzureFunctionsRuntime`] drives any type implementing [`RequestHandler`],
//! which is stated in this crate's own [`FunctionRequest`]/[`FunctionResponse`]
//! types. It does **not** convert to or from `armature_core::HttpRequest` /
//! `HttpResponse`, and there is no blanket implementation for an Armature
//! `Application` — wiring one up is the application author's job, most easily
//! via `impl_azure_function_handler!` (see its documentation for the exact
//! shape it expects).
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature_azure_functions::{
//!     AzureFunctionsRuntime, FunctionRequest, FunctionResponse, init_tracing,
//! };
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Initialize tracing for Application Insights
//!     init_tracing();
//!
//!     let handler = |req: FunctionRequest| async move {
//!         FunctionResponse::with_body(200, format!("Hello from {}!", req.path))
//!     };
//!
//!     AzureFunctionsRuntime::new(handler).run().await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Deployment
//!
//! ```bash
//! # Install Azure Functions Core Tools
//! npm install -g azure-functions-core-tools@4
//!
//! # Create function app
//! func init --worker-runtime custom
//!
//! # Add HTTP trigger
//! func new --template "HTTP trigger" --name api
//!
//! # Deploy
//! func azure functionapp publish <app-name>
//! ```
//!
//! ## Azure Functions Features
//!
//! This crate helps with:
//! - **HTTP Triggers**: Handle HTTP requests in Azure Functions via the
//!   custom-handler HTTP server
//! - **Application Insights**: Structured JSON logging for monitoring
//! - **Configuration**: Read runtime settings from environment variables
//!   ([`FunctionConfig::from_env`])
//!
//! Only HTTP triggers are supported. Non-HTTP triggers (Timer/Queue/Blob) and
//! binding-based Azure service I/O are not dispatched by the runtime.

mod config;
mod error;
mod request;
mod response;
mod runtime;

pub use config::FunctionConfig;
pub use error::{AzureFunctionsError, Result};
pub use request::{FunctionRequest, RequestContext, RetryContext, TraceContext};
pub use response::FunctionResponse;
pub use runtime::{AzureFunctionsRuntime, RequestHandler, RuntimeConfig};

// Re-exported so `impl_azure_function_handler!` can name the attribute macro
// through `$crate`. Without this the macro would only expand in crates that
// happen to depend on `async-trait` themselves and under that exact name.
pub use async_trait;

/// Initialize tracing for Azure Application Insights.
///
/// This sets up structured JSON logging suitable for Application Insights.
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

/// Check if running in Azure Functions.
pub fn is_azure_functions() -> bool {
    std::env::var("FUNCTIONS_WORKER_RUNTIME").is_ok()
        || std::env::var("AZURE_FUNCTIONS_ENVIRONMENT").is_ok()
}

/// Get the function app name.
pub fn function_app_name() -> Option<String> {
    std::env::var("WEBSITE_SITE_NAME").ok()
}

/// Get the function name.
pub fn function_name() -> Option<String> {
    std::env::var("AZURE_FUNCTIONS_FUNCTION_NAME").ok()
}

/// Get the invocation ID.
pub fn invocation_id() -> Option<String> {
    std::env::var("AZURE_FUNCTIONS_INVOCATION_ID").ok()
}
