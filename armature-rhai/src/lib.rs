//! # Armature Rhai
//!
//! Rhai scripting language integration for Armature applications.
//!
//! Write HTTP handlers, middleware, and business logic in Rhai scripts while
//! leveraging Armature's high-performance Rust core.
//!
//! ## Features
//!
//! - **Script Handlers**: Define HTTP handlers in `.rhai` files
//! - **Hot Reload**: Automatic script reloading during development
//! - **Type-Safe Bindings**: Access HTTP requests, responses, and Armature features
//! - **Sandboxed Execution**: Safe script execution with configurable limits
//! - **Performance**: Compiled scripts with caching for production
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature_rhai::{RhaiEngine, ScriptRouter};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create Rhai engine with Armature bindings
//!     let engine = RhaiEngine::new()
//!         .with_max_operations(100_000)
//!         .with_scripts_dir("./scripts")
//!         .build()?;
//!
//!     // Create router with script-based handlers
//!     let router = ScriptRouter::new(engine)
//!         .route("/", "handlers/index.rhai")
//!         .route("/users/:id", "handlers/users.rhai")
//!         .route("/api/*", "handlers/api.rhai");
//!
//!     // Run server
//!     router.serve("0.0.0.0:8080").await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Script Example
//!
//! ```rhai
//! // handlers/users.rhai
//!
//! // Access request data
//! let user_id = request.param("id");
//! let method = request.method();
//!
//! // Handle different methods
//! if method == "GET" {
//!     // Return JSON response
//!     response.json(#{
//!         id: user_id,
//!         name: "Alice",
//!         email: "alice@example.com"
//!     })
//! } else if method == "PUT" {
//!     let body = request.json();
//!     // Process update...
//!     response.ok()
//! } else {
//!     response.method_not_allowed()
//! }
//! ```
//!
//! ## Hot Reload (Development)
//!
//! Enable the `hot-reload` feature for automatic script reloading:
//!
//! ```rust,ignore
//! let engine = RhaiEngine::new()
//!     .with_hot_reload(true)
//!     .build()?;
//! ```

mod bindings;
mod context;
mod engine;
mod error;
mod handler;
mod router;
mod script;

#[cfg(feature = "hot-reload")]
mod watcher;

pub use bindings::{RequestBinding, ResponseBinding, register_armature_api};
pub use context::ScriptContext;
pub use engine::{RhaiEngine, RhaiEngineBuilder};
pub use error::{Result, RhaiError};
pub use handler::{ScriptHandler, ScriptMiddleware};
pub use router::ScriptRouter;
pub use script::{CompiledScript, ScriptCache, ScriptLoader};

#[cfg(feature = "hot-reload")]
pub use watcher::ScriptWatcher;

// Re-export rhai for advanced usage
pub use rhai;
