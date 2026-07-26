//! Behavioral tests for the `#[use_middleware]` / `#[middleware]` decorators.

use armature_core::middleware::{Middleware, Next};
use armature_core::{Error, HttpRequest, HttpResponse};
use armature_proc_macro::{controller, middleware, module, routes, use_middleware};
use std::collections::HashMap;

mod support;
use support::{build_router, get_request};

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

// ---------------------------------------------------------------------------
// Ordering contract: the first-listed struct-level middleware is the outermost
// one, so it observes the request first. Each middleware appends its tag to an
// `X-Order` *request* header on the way in; the handler echoes the accumulated
// value back, making the entry order directly observable.
// ---------------------------------------------------------------------------

/// Middleware that also proves it is built once: constructing it bumps
/// `MIDDLEWARE_CONSTRUCTED`.
static MIDDLEWARE_CONSTRUCTED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

struct OrderMiddleware(&'static str);

impl OrderMiddleware {
    fn new(tag: &'static str) -> Self {
        MIDDLEWARE_CONSTRUCTED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self(tag)
    }
}

#[async_trait::async_trait]
impl Middleware for OrderMiddleware {
    async fn handle(&self, mut req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        let order = match req.headers.get("X-Order") {
            Some(prev) => format!("{},{}", prev, self.0),
            None => self.0.to_string(),
        };
        req.headers.insert("X-Order".to_string(), order);
        next(req).await
    }
}

#[controller("/ordered")]
#[middleware(OrderMiddleware::new("1"), OrderMiddleware::new("2"))]
#[derive(Default)]
struct OrderedController;

#[routes]
impl OrderedController {
    #[get("/thing")]
    async fn thing(req: HttpRequest) -> Result<HttpResponse, Error> {
        let order = req.headers.get("X-Order").cloned().unwrap_or_default();
        Ok(HttpResponse::ok().with_header("X-Order".to_string(), order))
    }
}

#[module(controllers: [OrderedController])]
#[derive(Default)]
struct OrderedModule;

#[tokio::test]
async fn controller_struct_middleware_preserves_declaration_order() {
    let router = build_router::<OrderedModule>();

    // The chain is built at registration: two middlewares, constructed once.
    assert_eq!(
        MIDDLEWARE_CONSTRUCTED.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "each struct-level middleware must be constructed once at registration"
    );

    for _ in 0..3 {
        let resp = router.route(get_request("/ordered/thing")).await.unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(
            resp.headers.get("X-Order"),
            Some(&"1,2".to_string()),
            "the first-listed middleware must run first"
        );
    }

    assert_eq!(
        MIDDLEWARE_CONSTRUCTED.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "middleware must not be rebuilt per request"
    );
}
