//! Integration tests for Route Constraints

use armature_core::handler::from_legacy_handler;
use armature_core::*;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

// IntConstraint tests
#[test]
fn test_int_constraint_valid() {
    let constraint = IntConstraint;
    assert!(constraint.validate("123").is_ok());
    assert!(constraint.validate("-456").is_ok());
    assert!(constraint.validate("0").is_ok());
    assert!(constraint.validate("-0").is_ok());
}

#[test]
fn test_int_constraint_invalid() {
    let constraint = IntConstraint;
    assert!(constraint.validate("abc").is_err());
    assert!(constraint.validate("12.5").is_err());
    assert!(constraint.validate("").is_err());
    assert!(constraint.validate("12a").is_err());
}

// UIntConstraint tests
#[test]
fn test_uint_constraint_valid() {
    let constraint = UIntConstraint;
    assert!(constraint.validate("123").is_ok());
    assert!(constraint.validate("0").is_ok());
    assert!(constraint.validate("999999").is_ok());
}

#[test]
fn test_uint_constraint_invalid() {
    let constraint = UIntConstraint;
    assert!(constraint.validate("-1").is_err());
    assert!(constraint.validate("-456").is_err());
    assert!(constraint.validate("abc").is_err());
    assert!(constraint.validate("12.5").is_err());
}

// FloatConstraint tests
#[test]
fn test_float_constraint_valid() {
    let constraint = FloatConstraint;
    assert!(constraint.validate("123.45").is_ok());
    assert!(constraint.validate("-0.5").is_ok());
    assert!(constraint.validate("0").is_ok());
    assert!(constraint.validate("3.14159").is_ok());
}

#[test]
fn test_float_constraint_invalid() {
    let constraint = FloatConstraint;
    assert!(constraint.validate("abc").is_err());
    assert!(constraint.validate("12.5a").is_err());
    assert!(constraint.validate("").is_err());
}

// AlphaConstraint tests
#[test]
fn test_alpha_constraint_valid() {
    let constraint = AlphaConstraint;
    assert!(constraint.validate("abc").is_ok());
    assert!(constraint.validate("ABC").is_ok());
    assert!(constraint.validate("Hello").is_ok());
}

#[test]
fn test_alpha_constraint_invalid() {
    let constraint = AlphaConstraint;
    assert!(constraint.validate("abc123").is_err());
    assert!(constraint.validate("hello world").is_err());
    assert!(constraint.validate("hello-world").is_err());
    assert!(constraint.validate("123").is_err());
}

// AlphaNumConstraint tests
#[test]
fn test_alphanum_constraint_valid() {
    let constraint = AlphaNumConstraint;
    assert!(constraint.validate("abc123").is_ok());
    assert!(constraint.validate("ABC").is_ok());
    assert!(constraint.validate("123").is_ok());
    assert!(constraint.validate("user123").is_ok());
}

#[test]
fn test_alphanum_constraint_invalid() {
    let constraint = AlphaNumConstraint;
    assert!(constraint.validate("abc-123").is_err());
    assert!(constraint.validate("hello world").is_err());
    assert!(constraint.validate("user@123").is_err());
}

// UuidConstraint tests
#[test]
fn test_uuid_constraint_valid() {
    let constraint = UuidConstraint;
    assert!(
        constraint
            .validate("550e8400-e29b-41d4-a716-446655440000")
            .is_ok()
    );
    assert!(
        constraint
            .validate("123e4567-e89b-12d3-a456-426614174000")
            .is_ok()
    );
}

#[test]
fn test_uuid_constraint_invalid() {
    let constraint = UuidConstraint;
    assert!(constraint.validate("not-a-uuid").is_err());
    assert!(constraint.validate("12345").is_err());
    assert!(constraint.validate("550e8400-e29b-41d4-a716").is_err()); // Too short
    assert!(
        constraint
            .validate("550e8400e29b41d4a716446655440000")
            .is_err()
    ); // No hyphens
}

// EmailConstraint tests
#[test]
fn test_email_constraint_valid() {
    let constraint = EmailConstraint;
    assert!(constraint.validate("user@example.com").is_ok());
    assert!(constraint.validate("test.user@domain.co.uk").is_ok());
    assert!(constraint.validate("alice+tag@example.com").is_ok());
}

#[test]
fn test_email_constraint_invalid() {
    let constraint = EmailConstraint;
    assert!(constraint.validate("invalid-email").is_err());
    assert!(constraint.validate("@example.com").is_err());
    assert!(constraint.validate("user@").is_err());
    assert!(constraint.validate("user").is_err());
}

// LengthConstraint tests
#[test]
fn test_length_constraint_range() {
    let constraint = LengthConstraint::new(Some(3), Some(10));
    assert!(constraint.validate("hello").is_ok());
    assert!(constraint.validate("abc").is_ok());
    assert!(constraint.validate("1234567890").is_ok());
    assert!(constraint.validate("ab").is_err());
    assert!(constraint.validate("verylongstring").is_err());
}

#[test]
fn test_length_constraint_min() {
    let constraint = LengthConstraint::min(5);
    assert!(constraint.validate("hello").is_ok());
    assert!(constraint.validate("verylongstring").is_ok());
    assert!(constraint.validate("abcd").is_err());
}

#[test]
fn test_length_constraint_max() {
    let constraint = LengthConstraint::max(10);
    assert!(constraint.validate("hello").is_ok());
    assert!(constraint.validate("a").is_ok());
    assert!(constraint.validate("verylongstring").is_err());
}

#[test]
fn test_length_constraint_exact() {
    let constraint = LengthConstraint::exact(5);
    assert!(constraint.validate("hello").is_ok());
    assert!(constraint.validate("world").is_ok());
    assert!(constraint.validate("hi").is_err());
    assert!(constraint.validate("toolong").is_err());
}

// RangeConstraint tests
#[test]
fn test_range_constraint_range() {
    let constraint = RangeConstraint::new(Some(1), Some(100));
    assert!(constraint.validate("50").is_ok());
    assert!(constraint.validate("1").is_ok());
    assert!(constraint.validate("100").is_ok());
    assert!(constraint.validate("0").is_err());
    assert!(constraint.validate("101").is_err());
}

#[test]
fn test_range_constraint_min() {
    let constraint = RangeConstraint::min(0);
    assert!(constraint.validate("0").is_ok());
    assert!(constraint.validate("100").is_ok());
    assert!(constraint.validate("-1").is_err());
}

#[test]
fn test_range_constraint_max() {
    let constraint = RangeConstraint::max(1000);
    assert!(constraint.validate("500").is_ok());
    assert!(constraint.validate("1000").is_ok());
    assert!(constraint.validate("1001").is_err());
}

#[test]
fn test_range_constraint_invalid_number() {
    let constraint = RangeConstraint::new(Some(1), Some(100));
    assert!(constraint.validate("abc").is_err());
    assert!(constraint.validate("12.5").is_err());
}

// EnumConstraint tests
#[test]
fn test_enum_constraint_valid() {
    let constraint = EnumConstraint::new(vec![
        "active".to_string(),
        "inactive".to_string(),
        "pending".to_string(),
    ]);
    assert!(constraint.validate("active").is_ok());
    assert!(constraint.validate("pending").is_ok());
}

#[test]
fn test_enum_constraint_invalid() {
    let constraint = EnumConstraint::new(vec!["active".to_string(), "inactive".to_string()]);
    assert!(constraint.validate("unknown").is_err());
    assert!(constraint.validate("pending").is_err());
}

// RegexConstraint tests
#[test]
fn test_regex_constraint_valid() {
    let constraint = RegexConstraint::new(r"^[a-z]+$", "lowercase letters").unwrap();
    assert!(constraint.validate("hello").is_ok());
    assert!(constraint.validate("world").is_ok());
}

#[test]
fn test_regex_constraint_invalid() {
    let constraint = RegexConstraint::new(r"^[a-z]+$", "lowercase letters").unwrap();
    assert!(constraint.validate("Hello").is_err());
    assert!(constraint.validate("hello123").is_err());
}

// RouteConstraints tests
#[test]
fn test_route_constraints_single() {
    let constraints = RouteConstraints::new().add("id", Box::new(IntConstraint));

    let mut params = HashMap::new();
    params.insert("id".to_string(), "123".to_string());

    assert!(constraints.validate(&params).is_ok());
}

#[test]
fn test_route_constraints_single_invalid() {
    let constraints = RouteConstraints::new().add("id", Box::new(IntConstraint));

    let mut params = HashMap::new();
    params.insert("id".to_string(), "abc".to_string());

    assert!(constraints.validate(&params).is_err());
}

#[test]
fn test_route_constraints_multiple() {
    let constraints = RouteConstraints::new()
        .add("id", Box::new(IntConstraint))
        .add("name", Box::new(AlphaConstraint));

    let mut params = HashMap::new();
    params.insert("id".to_string(), "123".to_string());
    params.insert("name".to_string(), "john".to_string());

    assert!(constraints.validate(&params).is_ok());
}

#[test]
fn test_route_constraints_multiple_one_invalid() {
    let constraints = RouteConstraints::new()
        .add("id", Box::new(IntConstraint))
        .add("name", Box::new(AlphaConstraint));

    let mut params = HashMap::new();
    params.insert("id".to_string(), "abc".to_string());
    params.insert("name".to_string(), "john".to_string());

    assert!(constraints.validate(&params).is_err());
}

#[test]
fn test_route_constraints_missing_param() {
    let constraints = RouteConstraints::new().add("id", Box::new(IntConstraint));

    let params = HashMap::new();

    // Missing parameter should not fail validation
    // (it only validates parameters that exist)
    assert!(constraints.validate(&params).is_ok());
}

#[test]
fn test_route_constraints_empty() {
    let constraints = RouteConstraints::new();

    assert!(constraints.is_empty());
    assert_eq!(constraints.len(), 0);

    let mut params = HashMap::new();
    params.insert("id".to_string(), "anything".to_string());

    assert!(constraints.validate(&params).is_ok());
}

#[test]
fn test_route_constraints_add_mut() {
    let mut constraints = RouteConstraints::new();
    constraints.add_mut("id", Box::new(IntConstraint));
    constraints.add_mut("name", Box::new(AlphaConstraint));

    assert_eq!(constraints.len(), 2);
    assert!(!constraints.is_empty());
}

#[test]
fn test_route_constraints_clone() {
    let constraints1 = RouteConstraints::new().add("id", Box::new(IntConstraint));

    let constraints2 = constraints1.clone();

    assert_eq!(constraints1.len(), constraints2.len());
    assert_eq!(constraints1.is_empty(), constraints2.is_empty());
}

// Integration tests with routing
#[tokio::test]
async fn test_route_with_constraints_valid() {
    let constraints = RouteConstraints::new().add("id", Box::new(UIntConstraint));

    let mut router = Router::new();
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/users/:id".to_string(),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>> {
            Box::pin(async move {
                let id = req.path_params.get("id").unwrap();
                Ok(HttpResponse::ok().with_body(id.as_bytes().to_vec()))
            })
        })),
        constraints: Some(constraints),
    });

    let request = HttpRequest::new("GET", "/users/123".to_string());
    let response = router.route(request).await.unwrap();

    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn test_route_with_constraints_invalid() {
    let constraints = RouteConstraints::new().add("id", Box::new(UIntConstraint));

    let mut router = Router::new();
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/users/:id".to_string(),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>> {
            Box::pin(async move {
                let id = req.path_params.get("id").unwrap();
                Ok(HttpResponse::ok().with_body(id.as_bytes().to_vec()))
            })
        })),
        constraints: Some(constraints),
    });

    let request = HttpRequest::new("GET", "/users/abc".to_string());
    let result = router.route(request).await;

    assert!(result.is_err());
    match result {
        Err(Error::BadRequest(_)) => (),
        _ => panic!("Expected BadRequest error"),
    }
}

#[tokio::test]
async fn test_route_with_multiple_constraints() {
    let constraints = RouteConstraints::new()
        .add("user_id", Box::new(UIntConstraint))
        .add("post_id", Box::new(UIntConstraint));

    let mut router = Router::new();
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/users/:user_id/posts/:post_id".to_string(),
        handler: from_legacy_handler(Arc::new(|_req: HttpRequest| -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>> {
            Box::pin(async move { Ok(HttpResponse::ok()) })
        })),
        constraints: Some(constraints),
    });

    // Valid request
    let request = HttpRequest::new("GET", "/users/123/posts/456".to_string());
    let response = router.route(request).await.unwrap();
    assert_eq!(response.status, 200);

    // Invalid user_id
    let request2 = HttpRequest::new("GET", "/users/abc/posts/456".to_string());
    assert!(router.route(request2).await.is_err());

    // Invalid post_id
    let request3 = HttpRequest::new("GET", "/users/123/posts/abc".to_string());
    assert!(router.route(request3).await.is_err());
}
