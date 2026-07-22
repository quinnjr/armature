//! Behavioral tests for the `#[use_middleware]` / `#[middleware]` decorators.

use armature_core::middleware::{Middleware, Next};
use armature_core::{Container, Error, HttpRequest, HttpResponse, Module, Router};
use armature_proc_macro::{controller, middleware, module, routes, use_middleware};
use std::collections::HashMap;

/// Middleware that stamps a header on the response, proving the chain ran.
struct StampMiddleware;

#[async_trait::async_trait]
impl Middleware for StampMiddleware {
    async fn handle(&self, req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        let mut resp = next(req).await?;
        resp.headers.insert("X-Stamp".to_string(), "1".to_string());
        Ok(resp)
    }
}

fn request() -> HttpRequest {
    HttpRequest::from_parts(
        "GET".to_string(),
        "/users".to_string(),
        HashMap::new(),
        vec![],
        HashMap::new(),
        HashMap::new(),
    )
}

// The handler parameter is named `req` and used in the body; the middleware
// wrapper must preserve that binding and route the request through the chain.
#[use_middleware(StampMiddleware)]
async fn get_users(req: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = req.method.clone();
    Ok(HttpResponse::ok())
}

#[tokio::test]
async fn use_middleware_runs_chain() {
    let resp = get_users(request()).await.unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(resp.headers.get("X-Stamp"), Some(&"1".to_string()));
}

// `#[middleware(expr)]` applied to a function behaves the same.
#[middleware(StampMiddleware)]
async fn list_items(req: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &req;
    Ok(HttpResponse::ok())
}

#[tokio::test]
async fn middleware_on_fn_runs_chain() {
    let resp = list_items(request()).await.unwrap();
    assert_eq!(resp.headers.get("X-Stamp"), Some(&"1".to_string()));
}

// ---------------------------------------------------------------------------
// Controller-STRUCT-level `#[middleware(...)]` enforcement.
//
// Middleware attached to the controller struct must wrap *every* route
// registered for that controller through the module route registrar. This
// drives a real request through the generated registrar + `Router` and
// asserts the middleware ran (its stamped header is present on the response).
// ---------------------------------------------------------------------------

#[controller("/stamped")]
#[middleware(StampMiddleware)]
#[derive(Default)]
struct StampedController;

#[routes]
impl StampedController {
    #[get("/thing")]
    async fn thing() -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }
}

#[module(controllers: [StampedController])]
#[derive(Default)]
struct StampedModule;

fn build_router<M: Module + Default>() -> Router {
    let container = Container::new();
    let mut router = Router::new();
    let module = M::default();
    for reg in module.controllers() {
        let instance = (reg.factory)(&container).expect("controller factory");
        (reg.route_registrar)(&container, &mut router, instance).expect("route registrar");
    }
    router
}

fn get_request(path: &str) -> HttpRequest {
    HttpRequest::from_parts(
        "GET".to_string(),
        path.to_string(),
        HashMap::new(),
        vec![],
        HashMap::new(),
        HashMap::new(),
    )
}

#[tokio::test]
async fn controller_struct_middleware_wraps_route() {
    let router = build_router::<StampedModule>();
    let resp = router.route(get_request("/stamped/thing")).await.unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(
        resp.headers.get("X-Stamp"),
        Some(&"1".to_string()),
        "controller-struct #[middleware] must wrap the route and stamp the response"
    );
}
