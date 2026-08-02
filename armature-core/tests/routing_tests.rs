use armature_core::handler::from_legacy_handler;
use armature_core::{Error, HttpMethod, HttpRequest, HttpResponse, Route, Router};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

type LegacyHandler = Arc<
    dyn Fn(HttpRequest) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>
        + Send
        + Sync,
>;

#[tokio::test]
async fn test_static_route() {
    let mut router = Router::new();

    let handler: LegacyHandler = Arc::new(|_req| {
        Box::pin(async { Ok(HttpResponse::ok().with_body(b"Hello, World!".to_vec())) })
    });
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/hello".to_string(),
        handler: from_legacy_handler(handler),
        constraints: None,
    });

    let request = HttpRequest::new("GET", "/hello".to_string());
    let response = router.route(request).await.unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(response.body_ref(), b"Hello, World!");
}

#[tokio::test]
async fn test_path_parameter() {
    let mut router = Router::new();

    let handler: LegacyHandler = Arc::new(|req| {
        Box::pin(async move {
            let id = req.param("id").unwrap();
            Ok(HttpResponse::ok().with_body(id.as_bytes().to_vec()))
        })
    });
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/users/:id".to_string(),
        handler: from_legacy_handler(handler),
        constraints: None,
    });

    let request = HttpRequest::new("GET", "/users/123".to_string());
    let response = router.route(request).await.unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(response.body_ref(), b"123");
}

#[tokio::test]
async fn test_route_not_found() {
    let router = Router::new();

    let request = HttpRequest::new("GET", "/nonexistent".to_string());
    let result = router.route(request).await;

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), Error::RouteNotFound(_)));
}

#[tokio::test]
async fn test_query_parameters() {
    let mut router = Router::new();

    let handler: LegacyHandler = Arc::new(|req| {
        Box::pin(async move {
            let query = req.query("q").unwrap();
            Ok(HttpResponse::ok().with_body(query.as_bytes().to_vec()))
        })
    });
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/search".to_string(),
        handler: from_legacy_handler(handler),
        constraints: None,
    });

    let request = HttpRequest::new("GET", "/search?q=rust".to_string());
    let response = router.route(request).await.unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(response.body_ref(), b"rust");
}
