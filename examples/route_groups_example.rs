#![allow(clippy::needless_question_mark)]
//! Route Groups Example
//!
//! This example demonstrates `RouteGroup` for composing shared path
//! *prefixes* across related controllers, and the framework's real
//! guard/middleware enforcement mechanism: the `#[guard(...)]` /
//! `#[middleware(...)]` attributes on `#[routes]`-annotated controller
//! methods.
//!
//! **Important:** `RouteGroup::middleware(...)` / `RouteGroup::guard(...)`
//! only *record* values on the `RouteGroup` itself -- nothing in the router
//! or application ever reads them back out to enforce anything. Below,
//! `RouteGroup` is used purely to compute the shared prefix strings
//! ("/api/public", "/api/v1", "/api/v1/admin"); the guards and middleware
//! that actually run on every request are wired separately, via
//! `#[guard(...)]` on each protected handler and `#[middleware(...)]` on
//! each controller struct.
//!
//! Run with:
//! ```bash
//! cargo run --example route_groups_example
//! ```
//!
//! Test with:
//! ```bash
//! # Public API (no auth required)
//! curl http://localhost:3000/api/public/health
//!
//! # Authenticated API: AuthenticationGuard only checks that an
//! # `Authorization: Bearer <token>` header is present/well-formed, so any
//! # bearer value passes.
//! curl http://localhost:3000/api/v1/users -H "Authorization: Bearer token123"
//! # Missing/invalid Authorization header -> 403:
//! curl -i http://localhost:3000/api/v1/users
//!
//! # Admin API: requires authentication AND the admin role. This
//! # standalone example ships no JWT/session layer to populate verified
//! # roles, so the admin check is simulated by requiring a distinguished
//! # bearer token; see `require_admin_role` below for the real-world
//! # equivalent (`armature_core::guard::RolesGuard` plus an authentication
//! # middleware).
//! curl http://localhost:3000/api/v1/admin/users -H "Authorization: Bearer admin-token"
//! # Authenticated but non-admin token -> 403 (role required):
//! curl -i http://localhost:3000/api/v1/admin/users -H "Authorization: Bearer token123"
//! # No Authorization header at all -> 403:
//! curl -i http://localhost:3000/api/v1/admin/users
//! ```

use armature_core::*;
use armature_proc_macro::{controller, guard, middleware, module, routes};

/// Demo-only "admin role" check for this standalone example.
///
/// Real applications should guard admin routes with
/// [`armature_core::guard::RolesGuard`], which reads a `RequestRoles`
/// extension populated by a genuine authentication layer (e.g.
/// `armature-auth`'s `JwtAuthMiddleware`) *after* it verifies the caller's
/// token -- `RolesGuard` fails closed without that upstream population, so
/// a bare bearer token is never by itself sufficient. This example ships no
/// such layer, so the "admin" designation here is simulated by checking for
/// a distinguished bearer token value instead.
fn require_admin_role(ctx: &GuardContext) -> Result<bool, Error> {
    match ctx.get_header("authorization").map(String::as_str) {
        Some("Bearer admin-token") => Ok(true),
        _ => Err(Error::Forbidden("Admin role required".to_string())),
    }
}

/// Logger middleware for demonstration
struct LoggerMiddleware;

#[async_trait::async_trait]
impl Middleware for LoggerMiddleware {
    async fn handle(
        &self,
        request: HttpRequest,
        next: middleware::Next,
    ) -> Result<HttpResponse, Error> {
        info!("→ {} {}", request.method, request.path);
        let response = next(request).await?;
        info!("← Status: {}", response.status);
        Ok(response)
    }
}

// ===== Public API: no guard =====

#[controller("/api/public")]
#[middleware(LoggerMiddleware)]
#[derive(Default)]
struct PublicController;

#[routes]
impl PublicController {
    #[get("/health")]
    async fn health() -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok().with_json(&serde_json::json!({
            "status": "healthy",
            "group": "public"
        }))?)
    }
}

// ===== V1 API: requires authentication =====

#[controller("/api/v1")]
#[middleware(LoggerMiddleware)]
#[derive(Default)]
struct V1Controller;

#[routes]
impl V1Controller {
    // `_req` must stay an explicit parameter on guarded handlers: `#[routes]`
    // decides the call-site argument count from this handler's signature
    // *before* `#[guard(...)]` expands, and `#[guard(...)]` injects an
    // implicit request parameter into guarded handlers that have none --
    // dropping this parameter would make the two disagree on arity.
    #[guard(AuthenticationGuard)]
    #[get("/users")]
    async fn users(_req: HttpRequest) -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok().with_json(&serde_json::json!({
            "users": ["alice", "bob", "charlie"],
            "version": "v1"
        }))?)
    }

    #[guard(AuthenticationGuard)]
    #[get("/posts")]
    async fn posts(_req: HttpRequest) -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok().with_json(&serde_json::json!({
            "posts": [
                {"id": 1, "title": "First Post"},
                {"id": 2, "title": "Second Post"}
            ],
            "version": "v1"
        }))?)
    }
}

// ===== Admin API: requires authentication + admin role =====

#[controller("/api/v1/admin")]
#[middleware(LoggerMiddleware)]
#[derive(Default)]
struct AdminController;

#[routes]
impl AdminController {
    // See `V1Controller::users` above for why `_req` stays an explicit
    // parameter on guarded handlers.
    #[guard(AuthenticationGuard, CustomGuard::new(require_admin_role))]
    #[get("/users")]
    async fn users(_req: HttpRequest) -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok().with_json(&serde_json::json!({
            "users": [
                {"id": 1, "username": "alice", "role": "admin"},
                {"id": 2, "username": "bob", "role": "user"},
                {"id": 3, "username": "charlie", "role": "user"}
            ],
            "group": "admin",
            "version": "v1"
        }))?)
    }

    #[guard(AuthenticationGuard, CustomGuard::new(require_admin_role))]
    #[get("/stats")]
    async fn stats(_req: HttpRequest) -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok().with_json(&serde_json::json!({
            "total_users": 3,
            "total_posts": 2,
            "total_requests": 42,
            "group": "admin"
        }))?)
    }
}

#[module(controllers: [PublicController, V1Controller, AdminController])]
#[derive(Default)]
struct AppModule;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    let _guard = LogConfig::default().init();

    info!("Route Groups Example");
    info!("====================");

    // ----- RouteGroup: prefix composition only -----
    //
    // `RouteGroup` composes shared path *prefixes* correctly (that part is
    // real and tested). It also exposes `.middleware(...)`/`.guard(...)`
    // builders, but those only *record* values on the `RouteGroup` value
    // itself -- no router or application code ever reads them back out, so
    // they would silently enforce nothing. This example therefore uses
    // `RouteGroup` only for prefix composition below; the middleware and
    // guards that actually run are wired separately via `#[middleware(...)]`
    // and `#[guard(...)]` on each controller/handler.
    let api = RouteGroup::new().prefix("/api");

    info!("Created base /api group");

    let public = RouteGroup::new().prefix("/public").with_parent(&api);
    info!("Created /api/public group (no auth)");

    let v1 = RouteGroup::new().prefix("/v1").with_parent(&api);
    info!("Created /api/v1 group");

    let admin = RouteGroup::new().prefix("/admin").with_parent(&v1);
    info!("Created /api/v1/admin group");

    // Demonstrate route prefix application
    info!("\nRoute Prefix Examples:");
    info!("----------------------");

    let public_health = public.apply_prefix("/health");
    info!("Public health: {}", public_health);

    let v1_users = v1.apply_prefix("/users");
    info!("V1 users: {}", v1_users);

    let admin_users = admin.apply_prefix("/users");
    info!("Admin users: {}", admin_users);

    // ----- Real routing + enforcement -----
    //
    // These controllers are registered at paths matching the prefixes
    // computed above ("/api/public", "/api/v1", "/api/v1/admin"). Auth and
    // role enforcement genuinely happen here via `#[guard(...)]` on the
    // protected handlers, and `LoggerMiddleware` genuinely runs via
    // `#[middleware(...)]` on each controller struct -- this is the
    // framework's real, tested enforcement mechanism, unlike the inert
    // `RouteGroup.middleware()`/`.guard()` builders described above.
    info!("\nStarting server on http://localhost:3000");
    info!("Try these endpoints:");
    info!("  GET /api/public/health          (no auth)");
    info!("  GET /api/v1/users               (requires Authorization: Bearer <token>)");
    info!("  GET /api/v1/posts               (requires Authorization: Bearer <token>)");
    info!("  GET /api/v1/admin/users         (requires Authorization: Bearer admin-token)");
    info!("  GET /api/v1/admin/stats         (requires Authorization: Bearer admin-token)");

    let app = Application::create::<AppModule>().await;

    info!("\n✓ Server started successfully");
    info!("Press Ctrl+C to stop");

    app.listen(3000).await?;

    Ok(())
}
