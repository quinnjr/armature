// Allow dead_code while crate is under development
#![allow(dead_code)]
//! # Armature Azure Functions
//!
//! Azure Functions runtime adapter for Armature applications.
//!
//! This crate allows you to deploy Armature applications to Azure Functions
//! with HTTP triggers.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature::prelude::*;
//! use armature_azure_functions::{AzureFunctionsRuntime, init_tracing};
//!
//! #[controller("/")]
//! struct HelloController;
//!
//! #[controller_impl]
//! impl HelloController {
//!     #[get("/")]
//!     async fn hello() -> &'static str {
//!         "Hello from Azure Functions!"
//!     }
//! }
//!
//! #[module(controllers: [HelloController])]
//! struct AppModule;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Initialize tracing for Application Insights
//!     init_tracing();
//!
//!     // Create Armature application
//!     let app = Application::create::<AppModule>();
//!
//!     // Run on Azure Functions
//!     AzureFunctionsRuntime::new(app).run().await
//! }
//! ```
//!
//! ## With Azure Services
//!
//! ```rust,ignore
//! use armature_azure_functions::{AzureFunctionsRuntime, init_tracing};
//! use armature_azure::{AzureServices, AzureConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     init_tracing();
//!
//!     // Initialize Azure services
//!     let azure = AzureServices::new(
//!         AzureConfig::from_env()
//!             .enable_cosmos()
//!             .enable_blob()
//!             .build()
//!     ).await?;
//!
//!     // Register in DI container
//!     let app = Application::create::<AppModule>();
//!     app.container().register(azure);
//!
//!     AzureFunctionsRuntime::new(app).run().await
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
pub use request::FunctionRequest;
pub use response::FunctionResponse;
pub use runtime::{AzureFunctionsRuntime, RuntimeConfig};

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
