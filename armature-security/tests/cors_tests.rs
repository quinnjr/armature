//! CORS Integration Tests

use armature_core::{HttpRequest, HttpResponse};
use armature_security::cors::CorsConfig;

#[test]
fn test_cors_strict_origin() {
    let cors = CorsConfig::new().allow_origin("https://app.example.com");

    let mut request = create_request("GET", "/api/data");
    request
        .headers
        .insert("origin", "https://app.example.com".to_string());

    let mut response = HttpResponse::ok();
    cors.add_cors_headers(&mut response, "https://app.example.com");

    assert_eq!(
        response.headers.get("Access-Control-Allow-Origin"),
        Some(&"https://app.example.com".to_string())
    );
}

#[test]
fn test_cors_multiple_origins() {
    let cors = CorsConfig::new()
        .allow_origin("https://app.example.com")
        .allow_origin("https://admin.example.com");

    assert!(cors.is_origin_allowed("https://app.example.com"));
    assert!(cors.is_origin_allowed("https://admin.example.com"));
    assert!(!cors.is_origin_allowed("https://evil.com"));
}

#[test]
fn test_cors_origin_regex() {
    let cors = CorsConfig::new()
        .allow_origin_regex(r"https://.*\.example\.com")
        .unwrap();

    assert!(cors.is_origin_allowed("https://app.example.com"));
    assert!(cors.is_origin_allowed("https://api.example.com"));
    assert!(cors.is_origin_allowed("https://admin.example.com"));
    assert!(!cors.is_origin_allowed("https://example.com")); // No subdomain
    assert!(!cors.is_origin_allowed("https://evil.com"));
}

#[test]
fn test_cors_methods() {
    let cors = CorsConfig::new().allow_methods(vec!["GET", "POST", "PUT"]);

    assert!(cors.is_method_allowed("GET"));
    assert!(cors.is_method_allowed("POST"));
    assert!(cors.is_method_allowed("PUT"));
    assert!(cors.is_method_allowed("put")); // Case insensitive
    assert!(!cors.is_method_allowed("DELETE"));
}

#[test]
fn test_cors_headers() {
    let cors = CorsConfig::new().allow_headers(vec!["Content-Type", "Authorization"]);

    assert!(cors.is_header_allowed("content-type"));
    assert!(cors.is_header_allowed("Content-Type"));
    assert!(cors.is_header_allowed("authorization"));
    assert!(!cors.is_header_allowed("X-Custom-Header"));
}

#[test]
fn test_cors_any_header() {
    let cors = CorsConfig::new().allow_any_header();

    assert!(cors.is_header_allowed("Content-Type"));
    assert!(cors.is_header_allowed("X-Custom-Header"));
    assert!(cors.is_header_allowed("anything"));
}

#[test]
fn test_cors_credentials() {
    let cors = CorsConfig::new()
        .allow_origin("https://app.example.com")
        .allow_credentials(true);

    let mut response = HttpResponse::ok();
    cors.add_cors_headers(&mut response, "https://app.example.com");

    assert_eq!(
        response.headers.get("Access-Control-Allow-Credentials"),
        Some(&"true".to_string())
    );
}

#[test]
fn test_cors_exposed_headers() {
    let cors = CorsConfig::new()
        .allow_origin("https://app.example.com")
        .expose_headers(vec!["X-Total-Count", "X-Page-Number"]);

    let mut response = HttpResponse::ok();
    cors.add_cors_headers(&mut response, "https://app.example.com");

    assert_eq!(
        response.headers.get("Access-Control-Expose-Headers"),
        Some(&"X-Total-Count, X-Page-Number".to_string())
    );
}

#[test]
fn test_cors_preflight() {
    let cors = CorsConfig::new()
        .allow_origin("https://app.example.com")
        .allow_methods(vec!["GET", "POST", "PUT"])
        .allow_headers(vec!["Content-Type", "Authorization"])
        .max_age(3600);

    let mut request = create_request("OPTIONS", "/api/users");
    request
        .headers
        .insert("origin", "https://app.example.com".to_string());
    request
        .headers
        .insert("access-control-request-method", "POST".to_string());
    request.headers.insert(
        "access-control-request-headers",
        "Content-Type, Authorization".to_string(),
    );

    let response = cors.handle_preflight(&request).unwrap();

    assert_eq!(response.status, 204);
    assert!(
        response
            .headers
            .contains_key("Access-Control-Allow-Methods")
    );
    assert!(
        response
            .headers
            .contains_key("Access-Control-Allow-Headers")
    );
    assert_eq!(
        response.headers.get("Access-Control-Max-Age"),
        Some(&"3600".to_string())
    );
}

#[test]
fn test_cors_permissive() {
    let cors = CorsConfig::permissive();

    assert!(cors.is_origin_allowed("https://any-origin.com"));
    assert!(cors.is_method_allowed("GET"));
    assert!(cors.is_method_allowed("POST"));
    assert!(cors.is_method_allowed("DELETE"));
    assert!(cors.is_header_allowed("X-Custom-Header"));
}

#[test]
fn test_cors_wildcard_without_credentials() {
    let cors = CorsConfig::new()
        .allow_any_origin()
        .allow_credentials(false);

    let mut response = HttpResponse::ok();
    cors.add_cors_headers(&mut response, "https://any-origin.com");

    assert_eq!(
        response.headers.get("Access-Control-Allow-Origin"),
        Some(&"*".to_string())
    );
}

#[test]
fn test_cors_vary_header() {
    let cors = CorsConfig::new().allow_origin("https://app.example.com");

    let mut response = HttpResponse::ok();
    cors.add_cors_headers(&mut response, "https://app.example.com");

    // Vary header should be set for caching
    assert_eq!(response.headers.get("Vary"), Some(&"Origin".to_string()));
}

#[test]
fn test_cors_apply() {
    let cors = CorsConfig::new().allow_origin("https://app.example.com");

    let mut request = create_request("GET", "/api/data");
    request
        .headers
        .insert("origin", "https://app.example.com".to_string());

    let response = HttpResponse::ok();
    let response = cors.apply(&request, response);

    assert!(response.headers.contains_key("Access-Control-Allow-Origin"));
}

// Helper function
fn create_request(method: &str, path: &str) -> HttpRequest {
    HttpRequest::new(method.to_string(), path.to_string())
}
