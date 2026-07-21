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
//!     // `ctx.invoke(...)`, not `ctx.call(...)` — `call` is a reserved
//!     // word in Rhai (function-pointer invocation syntax).
//!     let data = ctx.invoke("UserService", "get_users");
//!     // `ok()`, not `Response::ok()` — the response helpers are plain
//!     // global functions, not a `Response` namespace/module.
//!     ok().json(data)
//! });
//!
//! // Assemble into a module — `create_module`, not `module`: `module` is a
//! // reserved keyword in Rhai.
//! let app_module = create_module("AppModule");
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
