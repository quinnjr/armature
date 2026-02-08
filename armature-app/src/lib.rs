//! # Armature App
//!
//! Build complete Armature applications in Rhai scripts — modules, controllers,
//! services, guards, middleware, lifecycle hooks — with zero Rust code.
//!
//! ## Quick Start
//!
//! Write an `app.rhai` script:
//!
//! ```rhai
//! // Define a service
//! let user_service = service("UserService");
//! user_service.define("get_users", || {
//!     [#{ id: 1, name: "Alice" }, #{ id: 2, name: "Bob" }]
//! });
//!
//! // Define a controller with routes
//! let users = controller("/api/users");
//! users.get("/", |req, ctx| {
//!     let data = ctx.call("UserService", "get_users");
//!     Response::ok().json(data)
//! });
//!
//! // Assemble into a module
//! let app_module = module("AppModule");
//! app_module.providers([user_service]);
//! app_module.controllers([users]);
//!
//! // Create and start the application
//! let app = create_app(app_module);
//! app.listen(3000);
//! ```
//!
//! Then run it:
//!
//! ```bash
//! armature run app.rhai
//! ```

pub mod bindings;
pub mod builder;
pub mod error;
pub mod runner;
pub mod types;

pub use error::{AppError, Result};
pub use runner::{RunConfig, run};
pub use types::{
    ScriptApp, ScriptController, ScriptGuard, ScriptMiddleware, ScriptModule, ScriptService,
    ServiceContext,
};
