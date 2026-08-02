#![allow(clippy::all)]
#![allow(clippy::needless_question_mark)]
//! Pagination & Filtering Example
//!
//! This example demonstrates:
//! - Offset pagination
//! - Cursor pagination
//! - Multi-field sorting
//! - Query parameter filtering
//! - Field selection
//!
//! Run with:
//! ```bash
//! cargo run --example pagination_example
//! ```
//!
//! Test with:
//! ```bash
//! # Pagination
//! curl "http://localhost:3000/users?page=2&per_page=5"
//!
//! # Sorting
//! curl "http://localhost:3000/users?sort=-created_at,name"
//!
//! # Filtering
//! curl "http://localhost:3000/users?status=active&age__gte=18"
//!
//! # Field selection
//! curl "http://localhost:3000/users?fields=id,name,email"
//!
//! # Combined
//! curl "http://localhost:3000/users?page=1&sort=-created_at&status=active&fields=id,name"
//!
//! # Cursor pagination
//! curl "http://localhost:3000/users/cursor?limit=10"
//! ```

use armature_core::handler::from_legacy_handler;
use armature_core::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct User {
    id: u64,
    name: String,
    email: String,
    status: String,
    age: u32,
    created_at: String,
}

// Mock database
fn get_mock_users() -> Vec<User> {
    vec![
        User {
            id: 1,
            name: "Alice Johnson".to_string(),
            email: "alice@example.com".to_string(),
            status: "active".to_string(),
            age: 28,
            created_at: "2024-01-15".to_string(),
        },
        User {
            id: 2,
            name: "Bob Smith".to_string(),
            email: "bob@example.com".to_string(),
            status: "active".to_string(),
            age: 34,
            created_at: "2024-01-10".to_string(),
        },
        User {
            id: 3,
            name: "Charlie Brown".to_string(),
            email: "charlie@example.com".to_string(),
            status: "inactive".to_string(),
            age: 22,
            created_at: "2024-01-20".to_string(),
        },
        User {
            id: 4,
            name: "Diana Prince".to_string(),
            email: "diana@example.com".to_string(),
            status: "active".to_string(),
            age: 31,
            created_at: "2024-01-12".to_string(),
        },
        User {
            id: 5,
            name: "Eve Adams".to_string(),
            email: "eve@example.com".to_string(),
            status: "active".to_string(),
            age: 26,
            created_at: "2024-01-18".to_string(),
        },
        User {
            id: 6,
            name: "Frank Miller".to_string(),
            email: "frank@example.com".to_string(),
            status: "inactive".to_string(),
            age: 45,
            created_at: "2024-01-08".to_string(),
        },
        User {
            id: 7,
            name: "Grace Lee".to_string(),
            email: "grace@example.com".to_string(),
            status: "active".to_string(),
            age: 29,
            created_at: "2024-01-22".to_string(),
        },
        User {
            id: 8,
            name: "Henry Ford".to_string(),
            email: "henry@example.com".to_string(),
            status: "active".to_string(),
            age: 38,
            created_at: "2024-01-14".to_string(),
        },
    ]
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    let _guard = LogConfig::default().init();

    info!("Pagination & Filtering Example");
    info!("==============================");

    // Create router
    let mut router = Router::new();

    // Offset pagination endpoint
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/users".to_string(),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                // Parse query parameters
                let query = QueryParams::from_hashmap(&req.query().to_hash_map());

                // Get all users
                let mut users = get_mock_users();

                // Apply filters
                if !query.filter.is_empty() {
                    users.retain(|user| {
                        for condition in &query.filter.conditions {
                            match condition.field.as_str() {
                                "status" => {
                                    if let Some(ref value) = condition.value {
                                        if user.status != *value {
                                            return false;
                                        }
                                    }
                                }
                                "age" => {
                                    if let Some(ref value) = condition.value {
                                        if let Ok(age) = value.parse::<u32>() {
                                            match condition.operator {
                                                FilterOperator::Eq => {
                                                    if user.age != age {
                                                        return false;
                                                    }
                                                }
                                                FilterOperator::Gte => {
                                                    if user.age < age {
                                                        return false;
                                                    }
                                                }
                                                FilterOperator::Lte => {
                                                    if user.age > age {
                                                        return false;
                                                    }
                                                }
                                                FilterOperator::Gt => {
                                                    if user.age <= age {
                                                        return false;
                                                    }
                                                }
                                                FilterOperator::Lt => {
                                                    if user.age >= age {
                                                        return false;
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        true
                    });
                }

                let total = users.len();

                // Apply sorting
                if !query.sort.is_empty() {
                    for sort_field in query.sort.fields.iter().rev() {
                        match sort_field.field.as_str() {
                            "name" => {
                                users.sort_by(|a, b| {
                                    let cmp = a.name.cmp(&b.name);
                                    match sort_field.direction {
                                        SortDirection::Asc => cmp,
                                        SortDirection::Desc => cmp.reverse(),
                                    }
                                });
                            }
                            "created_at" => {
                                users.sort_by(|a, b| {
                                    let cmp = a.created_at.cmp(&b.created_at);
                                    match sort_field.direction {
                                        SortDirection::Asc => cmp,
                                        SortDirection::Desc => cmp.reverse(),
                                    }
                                });
                            }
                            "age" => {
                                users.sort_by(|a, b| {
                                    let cmp = a.age.cmp(&b.age);
                                    match sort_field.direction {
                                        SortDirection::Asc => cmp,
                                        SortDirection::Desc => cmp.reverse(),
                                    }
                                });
                            }
                            _ => {}
                        }
                    }
                }

                // Apply pagination
                let offset = query.pagination.offset();
                let limit = query.pagination.limit();
                let paginated_users: Vec<User> =
                    users.into_iter().skip(offset).take(limit).collect();

                // Apply field selection
                let response_json = if query.fields.is_active() {
                    let filtered: Vec<serde_json::Value> = paginated_users
                        .iter()
                        .map(|user| {
                            let mut obj = serde_json::json!({});
                            if query.fields.should_include("id") {
                                obj["id"] = serde_json::json!(user.id);
                            }
                            if query.fields.should_include("name") {
                                obj["name"] = serde_json::json!(&user.name);
                            }
                            if query.fields.should_include("email") {
                                obj["email"] = serde_json::json!(&user.email);
                            }
                            if query.fields.should_include("status") {
                                obj["status"] = serde_json::json!(&user.status);
                            }
                            if query.fields.should_include("age") {
                                obj["age"] = serde_json::json!(user.age);
                            }
                            if query.fields.should_include("created_at") {
                                obj["created_at"] = serde_json::json!(&user.created_at);
                            }
                            obj
                        })
                        .collect();

                    let response = PaginatedResponse::new(filtered, query.pagination, total);
                    serde_json::to_value(&response).map_err(|e| Error::Internal(e.to_string()))?
                } else {
                    let response = PaginatedResponse::new(paginated_users, query.pagination, total);
                    serde_json::to_value(&response).map_err(|e| Error::Internal(e.to_string()))?
                };

                Ok(HttpResponse::ok().with_json(&response_json)?)
            })
        })),
        constraints: None,
    });

    // Cursor pagination endpoint
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/users/cursor".to_string(),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                // Parse cursor pagination
                let pagination = CursorPagination::from_query_params(&req.query().to_hash_map());

                let users = get_mock_users();

                // In real app, cursor would be an actual database cursor
                // For demo, we'll use simple offset
                let cursor_offset = pagination
                    .cursor
                    .as_ref()
                    .and_then(|c| c.parse::<usize>().ok())
                    .unwrap_or(0);

                let total = users.len();
                let paginated_users: Vec<User> = users
                    .into_iter()
                    .skip(cursor_offset)
                    .take(pagination.limit)
                    .collect();

                let next_cursor = if cursor_offset + pagination.limit < total {
                    Some((cursor_offset + pagination.limit).to_string())
                } else {
                    None
                };

                let response = PaginatedResponse::with_cursor(
                    paginated_users,
                    pagination.limit,
                    total,
                    next_cursor,
                );

                Ok(HttpResponse::ok().with_json(&response)?)
            })
        })),
        constraints: None,
    });

    // Home endpoint with documentation
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/".to_string(),
        handler: from_legacy_handler(Arc::new(|_req: HttpRequest| {
            Box::pin(async move {
                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "message": "Pagination & Filtering Example",
                    "endpoints": {
                        "GET /users": "List users with pagination",
                        "GET /users/cursor": "List users with cursor pagination"
                    },
                    "features": {
                        "pagination": "?page=2&per_page=5",
                        "sorting": "?sort=-created_at,name (- prefix = DESC)",
                        "filtering": "?status=active&age__gte=18",
                        "field_selection": "?fields=id,name,email",
                        "search": "?q=search term",
                        "combined": "?page=1&sort=-created_at&status=active&fields=id,name"
                    },
                    "filter_operators": {
                        "eq": "Equal (default)",
                        "ne": "Not equal",
                        "gt": "Greater than",
                        "gte": "Greater than or equal",
                        "lt": "Less than",
                        "lte": "Less than or equal",
                        "in": "In list",
                        "contains": "Contains substring",
                        "starts_with": "Starts with",
                        "ends_with": "Ends with"
                    },
                    "examples": {
                        "pagination": "curl 'http://localhost:3000/users?page=2&per_page=5'",
                        "sorting": "curl 'http://localhost:3000/users?sort=-created_at,name'",
                        "filtering": "curl 'http://localhost:3000/users?status=active&age__gte=18'",
                        "fields": "curl 'http://localhost:3000/users?fields=id,name,email'",
                        "combined": "curl 'http://localhost:3000/users?page=1&sort=-age&status=active&fields=id,name'",
                        "cursor": "curl 'http://localhost:3000/users/cursor?limit=3'"
                    }
                }))?)
            })
        })),
        constraints: None,
    });

    // Build application
    let container = Container::new();
    let app = Application::new(container, router);

    info!("\n✓ Server started on http://localhost:3000");
    info!("\nFeatures:");
    info!("  ✓ Offset pagination (page/per_page)");
    info!("  ✓ Cursor pagination (cursor/limit)");
    info!("  ✓ Multi-field sorting");
    info!("  ✓ Query parameter filtering");
    info!("  ✓ Field selection (sparse fieldsets)");
    info!("\nExamples:");
    info!("  curl 'http://localhost:3000/users?page=2&per_page=3'");
    info!("  curl 'http://localhost:3000/users?sort=-created_at,name'");
    info!("  curl 'http://localhost:3000/users?status=active&age__gte=25'");
    info!("  curl 'http://localhost:3000/users?fields=id,name,email'");
    info!("  curl 'http://localhost:3000/users?page=1&sort=-age&status=active'");
    info!("\nPress Ctrl+C to stop\n");

    app.listen(3000).await?;

    Ok(())
}
