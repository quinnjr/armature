#![allow(clippy::needless_question_mark)]
//! Route Groups and Constraints Combined Example
//!
//! This example demonstrates using Route Groups and Route Constraints
//! together to create a well-organized, validated API.
//!
//! Run with:
//! ```bash
//! cargo run --example route_groups_and_constraints
//! ```
//!
//! Test with:
//! ```bash
//! # V1 API - Valid requests
//! curl http://localhost:3000/api/v1/users/123
//! curl http://localhost:3000/api/v1/posts/1/comments
//!
//! # V2 API - Valid requests
//! curl http://localhost:3000/api/v2/users/550e8400-e29b-41d4-a716-446655440000
//! curl http://localhost:3000/api/v2/products/active
//!
//! # Invalid requests
//! curl http://localhost:3000/api/v1/users/abc  # Not an integer
//! curl http://localhost:3000/api/v2/users/123  # Not a UUID
//! curl http://localhost:3000/api/v2/products/unknown  # Invalid status
//! ```

use armature_core::handler::from_legacy_handler;
use armature_core::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    let _guard = LogConfig::default().init();

    info!("Route Groups + Constraints Example");
    info!("===================================");

    let mut router = Router::new();

    // Base API group with logging
    let api = RouteGroup::new().prefix("/api");

    info!("Created base /api group");

    // V1 API - Uses integer IDs
    let v1 = RouteGroup::new().prefix("/v1").with_parent(&api);

    info!("Created /api/v1 group (integer IDs)");

    // V2 API - Uses UUIDs
    let v2 = RouteGroup::new().prefix("/v2").with_parent(&api);

    info!("Created /api/v2 group (UUIDs)");

    // V1 Routes with integer constraints
    info!("\nV1 Routes (Integer IDs):");
    info!("------------------------");

    // V1: GET /api/v1/users/:id
    let v1_user_constraints = RouteConstraints::new().add("id", Box::new(UIntConstraint));

    router.add_route(Route {
        method: HttpMethod::GET,
        path: v1.apply_prefix("/users/:id"),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                let id = req.param("id").unwrap();

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "api_version": "v1",
                    "user_id": id,
                    "username": format!("user{}", id),
                    "id_type": "integer"
                }))?)
            })
        })),
        constraints: Some(v1_user_constraints),
    });
    info!("  ✓ GET {} (id: integer)", v1.apply_prefix("/users/:id"));

    // V1: GET /api/v1/posts/:page/comments
    let v1_posts_constraints =
        RouteConstraints::new().add("page", Box::new(RangeConstraint::new(Some(1), Some(1000))));

    router.add_route(Route {
        method: HttpMethod::GET,
        path: v1.apply_prefix("/posts/:page/comments"),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                let page = req.param("page").unwrap();

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "api_version": "v1",
                    "page": page,
                    "comments": [
                        {"id": 1, "text": "Great post!"},
                        {"id": 2, "text": "Thanks!"}
                    ],
                    "constraint": "page: 1-1000"
                }))?)
            })
        })),
        constraints: Some(v1_posts_constraints),
    });
    info!(
        "  ✓ GET {} (page: 1-1000)",
        v1.apply_prefix("/posts/:page/comments")
    );

    // V2 Routes with UUID constraints
    info!("\nV2 Routes (UUIDs):");
    info!("------------------");

    // V2: GET /api/v2/users/:uuid
    let v2_user_constraints = RouteConstraints::new().add("uuid", Box::new(UuidConstraint));

    router.add_route(Route {
        method: HttpMethod::GET,
        path: v2.apply_prefix("/users/:uuid"),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                let uuid = req.param("uuid").unwrap();

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "api_version": "v2",
                    "user_id": uuid,
                    "username": "alice",
                    "id_type": "uuid"
                }))?)
            })
        })),
        constraints: Some(v2_user_constraints),
    });
    info!("  ✓ GET {} (uuid: UUID)", v2.apply_prefix("/users/:uuid"));

    // V2: GET /api/v2/products/:status
    let v2_products_constraints = RouteConstraints::new().add(
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
        path: v2.apply_prefix("/products/:status"),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                let status = req.param("status").unwrap();

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "api_version": "v2",
                    "status": status,
                    "products": [
                        {"id": "550e8400-e29b-41d4-a716-446655440001", "name": "Product A"},
                        {"id": "550e8400-e29b-41d4-a716-446655440002", "name": "Product B"}
                    ],
                    "constraint": "status: active|inactive|pending|archived"
                }))?)
            })
        })),
        constraints: Some(v2_products_constraints),
    });
    info!(
        "  ✓ GET {} (status: enum)",
        v2.apply_prefix("/products/:status")
    );

    // V2: GET /api/v2/search/:query
    let v2_search_constraints =
        RouteConstraints::new().add("query", Box::new(LengthConstraint::new(Some(3), Some(50))));

    router.add_route(Route {
        method: HttpMethod::GET,
        path: v2.apply_prefix("/search/:query"),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                let query = req.param("query").unwrap();

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "api_version": "v2",
                    "query": query,
                    "results": [
                        {"id": "550e8400-e29b-41d4-a716-446655440001", "title": "Result 1"},
                        {"id": "550e8400-e29b-41d4-a716-446655440002", "title": "Result 2"}
                    ],
                    "constraint": "query: 3-50 chars"
                }))?)
            })
        })),
        constraints: Some(v2_search_constraints),
    });
    info!(
        "  ✓ GET {} (query: 3-50 chars)",
        v2.apply_prefix("/search/:query")
    );

    // Root endpoint
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/".to_string(),
        handler: from_legacy_handler(Arc::new(|_req: HttpRequest| {
            Box::pin(async move {
                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "message": "Route Groups + Constraints API",
                    "v1": {
                        "base": "/api/v1",
                        "id_format": "integer",
                        "endpoints": {
                            "users": "GET /api/v1/users/:id",
                            "comments": "GET /api/v1/posts/:page/comments"
                        }
                    },
                    "v2": {
                        "base": "/api/v2",
                        "id_format": "uuid",
                        "endpoints": {
                            "users": "GET /api/v2/users/:uuid",
                            "products": "GET /api/v2/products/:status",
                            "search": "GET /api/v2/search/:query"
                        }
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
    info!("\nTry these valid requests:");
    info!("  V1: curl http://localhost:3000/api/v1/users/123");
    info!("  V1: curl http://localhost:3000/api/v1/posts/1/comments");
    info!("  V2: curl http://localhost:3000/api/v2/users/550e8400-e29b-41d4-a716-446655440000");
    info!("  V2: curl http://localhost:3000/api/v2/products/active");
    info!("  V2: curl http://localhost:3000/api/v2/search/hello");
    info!("\nTry these invalid requests (will fail with 400):");
    info!("  V1: curl http://localhost:3000/api/v1/users/abc");
    info!("  V1: curl http://localhost:3000/api/v1/posts/0/comments");
    info!("  V2: curl http://localhost:3000/api/v2/users/123");
    info!("  V2: curl http://localhost:3000/api/v2/products/unknown");
    info!("  V2: curl http://localhost:3000/api/v2/search/ab");
    info!("\nPress Ctrl+C to stop");

    app.listen(3000).await?;

    Ok(())
}
