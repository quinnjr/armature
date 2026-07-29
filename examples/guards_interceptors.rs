#![allow(
    dead_code,
    unused_imports,
    clippy::default_constructed_unit_structs,
    clippy::needless_borrow,
    clippy::unnecessary_lazy_evaluations
)]
// Guards and Interceptors example
//
// This example wires up the framework's REAL guard system --
// `armature_core::guard::{AuthenticationGuard, RolesGuard}` applied via the
// `#[guard(...)]` attribute macro -- rather than simulating guard behavior
// with inline `if let` header checks. `/api/protected` and `/api/admin`
// below are genuinely rejected by these guards before the handler body ever
// runs; see the doc comments on each endpoint for exactly what each guard
// checks (and does not check).

use armature::prelude::*;
use armature_core::guard::{AuthenticationGuard, RequestRoles, RolesGuard};
use armature_core::middleware::{Middleware, Next};
use armature_proc_macro::{guard, middleware};
use serde::{Deserialize, Serialize};

// ========== DTOs ==========

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    content: String,
}

// ========== Services ==========

#[injectable]
#[derive(Clone, Default)]
struct DataService;

impl DataService {
    fn get_public_data(&self) -> Message {
        Message {
            content: "This is public data".to_string(),
        }
    }

    fn get_protected_data(&self) -> Message {
        Message {
            content: "This is protected data - you're authenticated!".to_string(),
        }
    }

    fn get_admin_data(&self) -> Message {
        Message {
            content: "This is admin-only data - you have admin role!".to_string(),
        }
    }
}

// ========== Role attachment (stand-in for real JWT verification) ==========

/// `RolesGuard` (`armature_core::guard`) authorizes a request by reading a
/// [`RequestRoles`] extension that is expected to have been attached by an
/// upstream authentication layer *after it verified the caller's token* --
/// in a real deployment that layer is `armature_auth::JwtAuthMiddleware`,
/// which decodes and verifies a JWT before attaching the caller's verified
/// roles. `RolesGuard` fails closed (403) when that extension is absent, so
/// it can never be satisfied by a bare header alone.
///
/// This example has no JWT infrastructure, so `AdminRoleMiddleware` stands
/// in for that verification layer: it is **not** real token verification --
/// it just recognizes one hardcoded demo credential
/// (`Bearer admin-token`) and attaches the `admin` role when it sees it.
/// Swap this middleware for `armature_auth::JwtAuthMiddleware` (or your own
/// verified-token layer) in a real application.
///
/// It is registered on `admin_endpoint` via `#[middleware(...)]` listed
/// *below* `#[guard(...)]`: at the function level, attribute macros are
/// expanded top-down and each wraps the *next* one's expansion, so the
/// bottom-most attribute (middleware) ends up as the outermost runtime
/// layer and genuinely runs before the guard it's stacked under -- letting
/// it populate `RequestRoles` in time for `RolesGuard` to read it.
struct AdminRoleMiddleware;

#[async_trait::async_trait]
impl Middleware for AdminRoleMiddleware {
    async fn handle(&self, mut req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        if req.headers.get("authorization").map(String::as_str) == Some("Bearer admin-token") {
            req.extensions.insert(RequestRoles::new(["admin"]));
        }
        next(req).await
    }
}

// ========== Controllers ==========

#[controller("/api")]
#[derive(Default, Clone)]
struct DataController;

#[routes]
impl DataController {
    #[get("/public")]
    async fn public_endpoint() -> Result<HttpResponse, Error> {
        let service = DataService::default();
        HttpResponse::json(&service.get_public_data())
    }

    /// Protected by the real `AuthenticationGuard`: it only checks that an
    /// `Authorization` header is present and starts with `Bearer ` -- it does
    /// **not** verify the token itself (see the guard's own doc comment in
    /// `armature-core/src/guard.rs`). A missing/malformed header never
    /// reaches this body; the guard rejects it with 403 first.
    #[get("/protected")]
    #[guard(AuthenticationGuard)]
    // `_req` must stay an explicit parameter here (rather than being
    // dropped like the zero-arg `public_endpoint` above): `#[routes]`
    // decides the call-site argument count from this handler's signature
    // *before* `#[guard(...)]` expands, and `#[guard(...)]` injects an
    // implicit request parameter into guarded handlers that have none --
    // dropping this parameter would make the two disagree on arity.
    async fn protected_endpoint(_req: HttpRequest) -> Result<HttpResponse, Error> {
        let service = DataService::default();
        HttpResponse::json(&service.get_protected_data())
    }

    /// Protected by the real `RolesGuard`, requiring the `admin` role. See
    /// `AdminRoleMiddleware` above for how (and why) the `admin` role gets
    /// attached to the request before this guard evaluates it.
    #[get("/admin")]
    #[guard(RolesGuard::new(vec!["admin".to_string()]))]
    #[middleware(AdminRoleMiddleware)]
    // See `protected_endpoint` above for why `_req` stays an explicit
    // parameter on guarded handlers.
    async fn admin_endpoint(_req: HttpRequest) -> Result<HttpResponse, Error> {
        let service = DataService::default();
        HttpResponse::json(&service.get_admin_data())
    }
}

// ========== Module ==========

#[module(
    providers: [DataService],
    controllers: [DataController]
)]
#[derive(Default, Clone)]
struct AppModule;

#[tokio::main]
async fn main() {
    println!("🛡️  Armature Guards & Interceptors Example");
    println!("==========================================\n");

    println!("Server running on http://localhost:3012");
    println!();
    println!("API Endpoints:");
    println!("  GET /api/public     - Public (no auth required)");
    println!("  GET /api/protected  - Protected (requires Bearer token)");
    println!("  GET /api/admin      - Admin only (requires Bearer admin-token)");
    println!();
    println!("Example usage:");
    println!();
    println!("1. Public endpoint (no auth):");
    println!("   curl http://localhost:3012/api/public");
    println!();
    println!("2. Protected endpoint (requires auth):");
    println!("   curl http://localhost:3012/api/protected \\");
    println!("     -H \"Authorization: Bearer token123\"");
    println!(
        "   (omit the header, or send a non-Bearer header, for a 403 from AuthenticationGuard)"
    );
    println!();
    println!("3. Admin endpoint:");
    println!("   curl http://localhost:3012/api/admin \\");
    println!("     -H \"Authorization: Bearer admin-token\"");
    println!(
        "   (any other bearer token gets a 403 from RolesGuard: authenticated, but not admin)"
    );
    println!();
    println!("Guards (armature_core::guard, applied via #[guard(...)]):");
    println!("  ✓ AuthenticationGuard - Checks for Bearer token (/api/protected)");
    println!("  ✓ RolesGuard - Checks user roles (/api/admin, requires \"admin\")");
    println!();

    let app = Application::create::<AppModule>().await;

    if let Err(e) = app.listen(3012).await {
        eprintln!("Server error: {}", e);
    }
}

// ========== Tests ==========
//
// Verifies -- in-process, without a human running curl -- the claim in this
// file's top doc comment: `/api/protected` and `/api/admin` are genuinely
// rejected by the real `armature_core::guard` guards before their handler
// bodies ever run.
//
// `armature_testing::TestAppBuilder::add_module` wires `AppModule`'s
// providers/controllers through the same `ProviderRegistration` /
// `ControllerRegistration` machinery `Application` itself uses, so the
// controller instances it builds are the real, `#[guard(...)]`/
// `#[middleware(...)]`-wrapped handlers -- those attributes bake the guard
// checks and middleware chain directly into the generated handler function
// body (see `armature-proc-macro/src/guard_attr.rs` and
// `middleware_attr.rs`), so they run under `TestClient`/`Router::route`
// exactly as they would under a real `Application`. (`TestAppBuilder`'s own
// doc comment notes that *struct-level* `#[guard(...)]`/`#[middleware(...)]`
// on a controller are not enforced this way; this file only uses the
// function-level form, so that caveat does not apply here.)
//
// `armature_testing::TestClient`'s `get`/`post`/etc. convenience methods
// don't expose custom headers, so requests are built directly with
// `HttpRequest::from_parts` and routed via `Router::route` to set the
// `Authorization` header these guards inspect.
#[cfg(test)]
mod tests {
    use super::*;
    use armature_testing::TestAppBuilder;
    use std::collections::HashMap;

    fn build_app() -> armature_testing::TestApp {
        TestAppBuilder::new().add_module(AppModule).build()
    }

    fn get_request(path: &str, headers: HashMap<String, String>) -> HttpRequest {
        HttpRequest::from_parts(
            "GET".to_string(),
            path.to_string(),
            headers,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
        )
    }

    fn bearer(token: &str) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), format!("Bearer {token}"));
        headers
    }

    #[tokio::test]
    async fn public_endpoint_requires_no_authentication() {
        let app = build_app();
        let response = app
            .app
            .router
            .route(get_request("/api/public", HashMap::new()))
            .await
            .expect("/api/public has no guard and must succeed");
        assert_eq!(response.status, 200);
    }

    #[tokio::test]
    async fn protected_endpoint_rejects_missing_authorization_header() {
        let app = build_app();
        let err = app
            .app
            .router
            .route(get_request("/api/protected", HashMap::new()))
            .await
            .expect_err(
                "AuthenticationGuard must reject a request with no Authorization header \
                 before protected_endpoint's body runs",
            );
        assert_eq!(err.status_code(), 403);
    }

    #[tokio::test]
    async fn protected_endpoint_accepts_any_bearer_token() {
        let app = build_app();
        let response = app
            .app
            .router
            .route(get_request("/api/protected", bearer("token123")))
            .await
            .expect(
                "AuthenticationGuard only checks that a Bearer token is present, \
                 so any bearer value must be accepted",
            );
        assert_eq!(response.status, 200);
    }

    #[tokio::test]
    async fn admin_endpoint_rejects_missing_authorization_header() {
        let app = build_app();
        let err = app
            .app
            .router
            .route(get_request("/api/admin", HashMap::new()))
            .await
            .expect_err(
                "RolesGuard must reject a request with no admin role attached \
                 before admin_endpoint's body runs",
            );
        assert_eq!(err.status_code(), 403);
    }

    #[tokio::test]
    async fn admin_endpoint_rejects_authenticated_non_admin_token() {
        let app = build_app();
        let err = app
            .app
            .router
            .route(get_request("/api/admin", bearer("token123")))
            .await
            .expect_err(
                "a bearer token other than the demo admin credential never gets the \
                 `admin` role attached by AdminRoleMiddleware, so RolesGuard must reject it",
            );
        assert_eq!(err.status_code(), 403);
    }

    #[tokio::test]
    async fn admin_endpoint_accepts_admin_token() {
        let app = build_app();
        let response = app
            .app
            .router
            .route(get_request("/api/admin", bearer("admin-token")))
            .await
            .expect(
                "the demo admin credential makes AdminRoleMiddleware attach the `admin` \
                 role, which RolesGuard must then accept",
            );
        assert_eq!(response.status, 200);
    }
}
