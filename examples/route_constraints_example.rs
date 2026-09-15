#![allow(clippy::needless_question_mark)]
//! Route Constraints Example
//!
//! This example demonstrates parameter validation at the route level
//! using Route Constraints.
//!
//! Run with:
//! ```bash
//! cargo run --example route_constraints_example
//! ```
//!
//! Test with:
//! ```bash
//! # Valid requests
//! curl http://localhost:3000/users/123
//! curl http://localhost:3000/users/550e8400-e29b-41d4-a716-446655440000
//! curl http://localhost:3000/users/alice/posts
//! curl http://localhost:3000/posts/5/comments
//! curl http://localhost:3000/products/active
//!
//! # Invalid requests (will return 400 Bad Request)
//! curl http://localhost:3000/users/abc          # Not a number
//! curl http://localhost:3000/users/not-a-uuid   # Not a UUID
//! curl http://localhost:3000/users/alice123/posts  # Not alphabetic
//! curl http://localhost:3000/posts/0/comments   # Page must be >= 1
//! curl http://localhost:3000/products/unknown   # Invalid status
//! ```

use armature_core::handler::from_legacy_handler;
use armature_core::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    let _guard = LogConfig::default().init();

    info!("Route Constraints Example");
    info!("=========================");

    let mut router = Router::new();

    // Example 1: Integer constraint - User by ID
    info!("\n1. Integer Constraint: /users/:id");
    let constraints = RouteConstraints::new().add("id", Box::new(UIntConstraint));

    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/users/:id".to_string(),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                let id = req.param("id").unwrap();
                info!("✓ Valid user ID: {}", id);

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "user_id": id,
                    "username": format!("user{}", id),
                    "constraint": "integer"
                }))?)
            })
        })),
        constraints: Some(constraints),
    });

    // Example 2: UUID constraint - Resource by UUID
    info!("2. UUID Constraint: /resources/:uuid");
    let uuid_constraints = RouteConstraints::new().add("uuid", Box::new(UuidConstraint));

    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/resources/:uuid".to_string(),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                let uuid = req.param("uuid").unwrap();
                info!("✓ Valid UUID: {}", uuid);

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "resource_id": uuid,
                    "type": "document",
                    "constraint": "uuid"
                }))?)
            })
        })),
        constraints: Some(uuid_constraints),
    });

    // Example 3: Alphabetic constraint - User by name
    info!("3. Alphabetic Constraint: /users/:name/posts");
    let alpha_constraints = RouteConstraints::new().add("name", Box::new(AlphaConstraint));

    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/users/:name/posts".to_string(),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                let name = req.param("name").unwrap();
                info!("✓ Valid username: {}", name);

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "username": name,
                    "posts": [
                        {"id": 1, "title": "First Post"},
                        {"id": 2, "title": "Second Post"}
                    ],
                    "constraint": "alphabetic"
                }))?)
            })
        })),
        constraints: Some(alpha_constraints),
    });

    // Example 4: Range constraint - Pagination
    info!("4. Range Constraint: /posts/:page/comments");
    let range_constraints = RouteConstraints::new().add("page", Box::new(RangeConstraint::min(1)));

    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/posts/:page/comments".to_string(),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                let page = req.param("page").unwrap();
                info!("✓ Valid page number: {}", page);

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "page": page,
                    "comments": ["Great post!", "Thanks for sharing!"],
                    "constraint": "range (>= 1)"
                }))?)
            })
        })),
        constraints: Some(range_constraints),
    });

    // Example 5: Enum constraint - Filter by status
    info!("5. Enum Constraint: /products/:status");
    let enum_constraints = RouteConstraints::new().add(
        "status",
        Box::new(EnumConstraint::new(vec![
            "active".to_string(),
            "inactive".to_string(),
            "pending".to_string(),
            "archived".to_string(),
        ])),
    );

    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/products/:status".to_string(),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                let status = req.param("status").unwrap();
                info!("✓ Valid status: {}", status);

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "status": status,
                    "products": [
                        {"id": 1, "name": "Product A"},
                        {"id": 2, "name": "Product B"}
                    ],
                    "constraint": "enum (active, inactive, pending, archived)"
                }))?)
            })
        })),
        constraints: Some(enum_constraints),
    });

    // Example 6: Multiple constraints - Complex route
    info!("6. Multiple Constraints: /api/:version/users/:id");
    let multi_constraints = RouteConstraints::new()
        .add("version", Box::new(AlphaNumConstraint))
        .add("id", Box::new(UIntConstraint));

    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/api/:version/users/:id".to_string(),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                let version = req.param("version").unwrap();
                let id = req.param("id").unwrap();
                info!("✓ Valid version: {}, id: {}", version, id);

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "api_version": version,
                    "user_id": id,
                    "username": format!("user{}", id),
                    "constraints": "version: alphanumeric, id: integer"
                }))?)
            })
        })),
        constraints: Some(multi_constraints),
    });

    // Example 7: Email constraint
    info!("7. Email Constraint: /notify/:email");
    let email_constraints = RouteConstraints::new().add("email", Box::new(EmailConstraint));

    router.add_route(Route {
        method: HttpMethod::POST,
        path: "/notify/:email".to_string(),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                let email = req.param("email").unwrap();
                info!("✓ Valid email: {}", email);

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "email": email,
                    "notification": "sent",
                    "constraint": "email format"
                }))?)
            })
        })),
        constraints: Some(email_constraints),
    });

    // Example 8: Length constraint - Short codes
    info!("8. Length Constraint: /codes/:code");
    let length_constraints =
        RouteConstraints::new().add("code", Box::new(LengthConstraint::new(Some(3), Some(10))));

    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/codes/:code".to_string(),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                let code = req.param("code").unwrap();
                info!("✓ Valid code: {}", code);

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "code": code,
                    "valid": true,
                    "constraint": "length (3-10 chars)"
                }))?)
            })
        })),
        constraints: Some(length_constraints),
    });

    // Example 9: Custom constraint - Postal codes
    info!("9. Custom Constraint: /shipping/:zip");
    let zip_constraints = RouteConstraints::new().add("zip", Box::new(ZipCodeConstraint));

    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/shipping/:zip".to_string(),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                let zip = req.param("zip").unwrap();
                info!("✓ Valid ZIP code: {}", zip);

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "zip_code": zip,
                    "shipping_available": true,
                    "estimated_days": 3,
                    "constraint": "custom (5-digit ZIP)"
                }))?)
            })
        })),
        constraints: Some(zip_constraints),
    });

    // Add a root endpoint with examples
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/".to_string(),
        handler: from_legacy_handler(Arc::new(|_req: HttpRequest| {
            Box::pin(async move {
                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "message": "Route Constraints Example API",
                    "examples": {
                        "integer": "GET /users/123",
                        "uuid": "GET /resources/550e8400-e29b-41d4-a716-446655440000",
                        "alphabetic": "GET /users/alice/posts",
                        "range": "GET /posts/5/comments",
                        "enum": "GET /products/active",
                        "multiple": "GET /api/v1/users/123",
                        "email": "POST /notify/user@example.com",
                        "length": "GET /codes/ABC123",
                        "custom": "GET /shipping/12345"
                    },
                    "try_invalid": {
                        "integer": "GET /users/abc (will fail)",
                        "uuid": "GET /resources/not-a-uuid (will fail)",
                        "alphabetic": "GET /users/alice123/posts (will fail)",
                        "range": "GET /posts/0/comments (will fail)",
                        "enum": "GET /products/unknown (will fail)"
                    }
                }))?)
            })
        })),
        constraints: None,
    });

    // Start server
    let container = Container::new();
    let app = Application::new(container, router);

    info!("\n✓ Server started on http://localhost:3000");
    info!("Try the endpoints listed above!");
    info!("Press Ctrl+C to stop");

    app.listen(3000).await?;

    Ok(())
}

/// Custom constraint for US ZIP codes (5 digits)
struct ZipCodeConstraint;

impl RouteConstraint for ZipCodeConstraint {
    fn validate(&self, value: &str) -> Result<(), String> {
        if value.len() == 5 && value.chars().all(|c| c.is_numeric()) {
            Ok(())
        } else {
            Err(format!(
                "'{}' is not a valid ZIP code. Must be 5 digits.",
                value
            ))
        }
    }

    fn description(&self) -> &str {
        "5-digit ZIP code"
    }
}
