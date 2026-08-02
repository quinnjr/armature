//! Integration tests for armature-core

use armature_core::*;

#[test]
fn test_http_request_creation() {
    let req = HttpRequest::new("GET", "/test".to_string());
    assert_eq!(req.method, "GET");
    assert_eq!(req.path, "/test");
    assert!(req.headers.is_empty());
    assert!(req.body.is_empty());
}

#[test]
fn test_http_response_creation() {
    let res = HttpResponse::ok();
    assert_eq!(res.status, 200);

    let res = HttpResponse::created();
    assert_eq!(res.status, 201);

    let res = HttpResponse::bad_request();
    assert_eq!(res.status, 400);

    let res = HttpResponse::not_found();
    assert_eq!(res.status, 404);

    let res = HttpResponse::internal_server_error();
    assert_eq!(res.status, 500);
}

#[test]
fn test_http_response_with_json() {
    use serde_json::json;

    let data = json!({"message": "Hello"});
    let res = HttpResponse::ok().with_json(&data).unwrap();

    assert_eq!(res.status, 200);
    assert!(res.headers.contains_key("Content-Type"));
    assert_eq!(res.headers.get("Content-Type").unwrap(), "application/json");
}

#[test]
fn test_http_status_codes() {
    assert_eq!(HttpStatus::Ok.code(), 200);
    assert_eq!(HttpStatus::Created.code(), 201);
    assert_eq!(HttpStatus::BadRequest.code(), 400);
    assert_eq!(HttpStatus::Unauthorized.code(), 401);
    assert_eq!(HttpStatus::Forbidden.code(), 403);
    assert_eq!(HttpStatus::NotFound.code(), 404);
    assert_eq!(HttpStatus::InternalServerError.code(), 500);
}

#[test]
fn test_error_conversion() {
    let err = Error::NotFound("Resource not found".to_string());
    assert_eq!(err.status_code(), 404);
    assert!(err.is_client_error());
    assert!(!err.is_server_error());

    let err = Error::Internal("Server error".to_string());
    assert_eq!(err.status_code(), 500);
    assert!(!err.is_client_error());
    assert!(err.is_server_error());
}

#[test]
fn test_middleware_chain_creation() {
    let chain = MiddlewareChain::new();
    // Middleware chain should be created successfully
    // Just check it can be constructed
    let _ = chain;
}

#[test]
fn test_container_creation() {
    let container = Container::new();
    // Container should be created successfully
    let _ = container;
}

#[test]
fn test_router_creation() {
    let router = Router::new();
    // Router should be created successfully
    let _ = router;
}

#[test]
fn test_application_creation() {
    let container = Container::new();
    let router = Router::new();
    let app = Application::new(container, router);
    // Application should be created successfully
    let _ = app;
}
