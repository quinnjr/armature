//! Parameter extractors declared on `#[routes]` controller methods must
//! actually reach the registered handler.
//!
//! `#[routes]` is the outer macro on the impl block, so it expands before any
//! per-method route attribute can. Until the extractor codegen was wired into
//! it, a controller method written the documented way kept inert `#[body]` /
//! `#[param("id")]` attributes on its parameters and failed to compile — the
//! extractor path only ever ran for free functions, which are never registered
//! as routes.
//!
//! Also covers a method declaring more than one route attribute: every one of
//! them must be registered, not just the first.
#![allow(dead_code)]

use armature_core::extractors::{Header, Headers, Path};
use armature_core::{Controller, Error, HttpMethod, HttpRequest, HttpResponse};
use armature_proc_macro::{controller, module, routes};
use std::collections::HashMap;

mod support;
use support::build_router;

fn request(method: &str, path: &str) -> HttpRequest {
    HttpRequest::from_parts(
        method,
        path.to_string(),
        HashMap::new(),
        vec![],
        HashMap::new(),
        HashMap::new(),
    )
}

fn body_of(response: &HttpResponse) -> String {
    String::from_utf8(response.body.to_vec()).expect("response body must be UTF-8")
}

// ---------------------------------------------------------------------------
// Path parameter extraction on an associated function.
// ---------------------------------------------------------------------------

#[controller("/users")]
#[derive(Default)]
struct UserController;

#[routes]
impl UserController {
    #[get("/:id")]
    async fn show(#[param("id")] id: Path<u32>) -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok().with_body(format!("user {}", *id).into_bytes()))
    }
}

#[module(controllers: [UserController])]
#[derive(Default)]
struct UserModule;

#[tokio::test]
async fn param_extractor_receives_the_path_value() {
    let router = build_router::<UserModule>();
    let resp = router
        .route(request("GET", "/users/42"))
        .await
        .expect("GET /users/42 must dispatch");
    assert_eq!(resp.status, 200);
    assert_eq!(body_of(&resp), "user 42");
}

// ---------------------------------------------------------------------------
// Query and header extractors on an instance method (`&self`), mixing the
// named forms with the `#[headers]` marker.
// ---------------------------------------------------------------------------

#[controller("/search")]
#[derive(Default)]
struct SearchController;

#[routes]
impl SearchController {
    #[get("/items")]
    async fn items(
        &self,
        #[query("q")] q: String,
        #[header("x-agent")] agent: Header,
        #[headers] headers: Headers,
    ) -> Result<HttpResponse, Error> {
        let via_map = headers.get("X-Agent").cloned().unwrap_or_default();
        Ok(HttpResponse::ok().with_body(format!("{q}|{}|{via_map}", agent.value()).into_bytes()))
    }
}

#[module(controllers: [SearchController])]
#[derive(Default)]
struct SearchModule;

#[tokio::test]
async fn query_and_header_extractors_reach_an_instance_method() {
    let router = build_router::<SearchModule>();
    let mut req = request("GET", "/search/items?q=hello");
    req.headers.insert("x-agent", "probe".to_string());

    let resp = router.route(req).await.expect("route must dispatch");
    assert_eq!(resp.status, 200);
    assert_eq!(body_of(&resp), "hello|probe|probe");
}

// ---------------------------------------------------------------------------
// A handler declaring two route attributes must register both.
// ---------------------------------------------------------------------------

#[controller("/ping")]
#[derive(Default)]
struct PingController;

#[routes]
impl PingController {
    #[get("/beat")]
    #[head("/beat")]
    async fn beat() -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }
}

#[module(controllers: [PingController])]
#[derive(Default)]
struct PingModule;

#[test]
fn every_route_attribute_is_reported() {
    let routes = PingController.routes();
    assert_eq!(
        routes.len(),
        2,
        "both #[get] and #[head] must be registered, got: {routes:?}"
    );
    assert!(routes.iter().any(|r| r.method == HttpMethod::GET));
    assert!(routes.iter().any(|r| r.method == HttpMethod::HEAD));
}

#[tokio::test]
async fn every_route_attribute_is_dispatchable() {
    let router = build_router::<PingModule>();
    for method in ["GET", "HEAD"] {
        let resp = router
            .route(request(method, "/ping/beat"))
            .await
            .unwrap_or_else(|e| panic!("{method} /ping/beat must dispatch: {e}"));
        assert_eq!(resp.status, 200, "{method} /ping/beat");
    }
}
