//! Behavioral tests for the `#[use_middleware]` / `#[middleware]` decorators.

use armature_core::middleware::{Middleware, Next};
use armature_core::{Error, HttpRequest, HttpResponse};
use armature_proc_macro::{middleware, use_middleware};
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
