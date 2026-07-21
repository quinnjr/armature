//! `Controller::routes()` must report the routes declared by `#[routes]`
//! instead of an empty list.
#![allow(dead_code)]

use armature_core::{Controller, Error, HttpMethod, HttpRequest, HttpResponse};
use armature_proc_macro::{controller, routes};

#[controller("/api")]
#[derive(Default)]
struct ApiController;

#[routes]
impl ApiController {
    #[get("/hello")]
    async fn hello() -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }

    #[post("/echo")]
    async fn echo(_req: HttpRequest) -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }
}

#[test]
fn routes_reports_declared_routes() {
    let controller = ApiController;
    let routes = controller.routes();

    assert_eq!(routes.len(), 2, "routes() must not be empty");
    assert!(
        routes
            .iter()
            .any(|r| r.method == HttpMethod::GET && r.path == "/hello"),
        "GET /hello should be reported, got: {routes:?}"
    );
    assert!(
        routes
            .iter()
            .any(|r| r.method == HttpMethod::POST && r.path == "/echo"),
        "POST /echo should be reported, got: {routes:?}"
    );
}

/// A controller with no `#[routes]` impl must still compile and report an
/// empty route list (no silent breakage of the delegation).
#[controller("/empty")]
#[derive(Default)]
struct EmptyController;

#[test]
fn controller_without_routes_impl_is_empty() {
    let controller = EmptyController;
    assert!(controller.routes().is_empty());
}
