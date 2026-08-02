//! Benchmark Server
//!
//! A minimal Armature server optimized for benchmarking.
//! Implements the standard TechEmpower-style benchmark endpoints.
//!
//! ## Endpoints
//!
//! - `GET /` - Plaintext "Hello, World!"
//! - `GET /json` - JSON response
//! - `GET /users/:id` - Path parameter extraction
//! - `POST /api/users` - JSON body parsing
//!
//! ## Usage
//!
//! ```bash
//! # Run the benchmark server
//! cargo run --release --example benchmark_server
//!
//! # Test endpoints
//! curl http://localhost:3000/
//! curl http://localhost:3000/json
//! curl http://localhost:3000/users/123
//! curl -X POST http://localhost:3000/api/users -d '{"name":"test"}'
//!
//! # Benchmark with oha
//! oha -z 10s -c 50 http://localhost:3000/
//! oha -z 10s -c 50 http://localhost:3000/json
//! ```

#![allow(clippy::needless_question_mark)]

use armature_core::handler::from_legacy_handler;
use armature_core::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::Level;

// Response structures
#[derive(Serialize)]
struct JsonResponse {
    message: &'static str,
}

#[derive(Serialize)]
struct UserResponse {
    id: String,
    name: String,
    email: String,
}

#[derive(Deserialize)]
struct CreateUserRequest {
    name: String,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Serialize)]
struct CreateUserResponse {
    id: u64,
    name: String,
    email: String,
    created: bool,
}

#[derive(Serialize)]
struct ProductInventory {
    quantity: u32,
    warehouse: String,
    last_updated: String,
}

#[derive(Serialize)]
struct ProductMetadata {
    views: u32,
    rating: f32,
    reviews_count: u32,
}

#[derive(Serialize)]
struct Product {
    id: u32,
    name: String,
    description: String,
    price: f64,
    category: String,
    tags: Vec<String>,
    inventory: ProductInventory,
    metadata: ProductMetadata,
}

#[derive(Serialize)]
struct DataMeta {
    total: usize,
    page: u32,
    per_page: usize,
    timestamp: u64,
}

#[derive(Serialize)]
struct DataResponse {
    data: Vec<Product>,
    meta: DataMeta,
}

fn generate_products(count: usize) -> Vec<Product> {
    let categories = ["Electronics", "Clothing", "Home", "Sports"];
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    (0..count)
        .map(|i| Product {
            id: i as u32 + 1,
            name: format!("Product {}", i + 1),
            description: format!(
                "This is the description for product {}. It contains detailed information about the product.",
                i + 1
            ),
            price: ((i as f64 * 17.3 + 10.0) % 1000.0 * 100.0).round() / 100.0,
            category: categories[i % 4].to_string(),
            tags: ["sale", "new", "popular"][..(i % 3) + 1].iter().map(|s| s.to_string()).collect(),
            inventory: ProductInventory {
                quantity: (i as u32 * 7) % 100,
                warehouse: format!("WH-{}", (i % 5) + 1),
                last_updated: format!("{}", now),
            },
            metadata: ProductMetadata {
                views: (i as u32 * 123) % 10000,
                rating: ((i as f32 * 0.3) % 2.0 + 3.0) * 10.0_f32.round() / 10.0,
                reviews_count: (i as u32 * 11) % 500,
            },
        })
        .collect()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Minimal logging for benchmarks
    tracing_subscriber::fmt().with_max_level(Level::WARN).init();

    println!("🚀 Armature Benchmark Server");
    println!("════════════════════════════");
    println!();

    let mut router = Router::new();

    // Plaintext endpoint
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/".to_string(),
        handler: from_legacy_handler(Arc::new(|_req: HttpRequest| {
            Box::pin(async {
                Ok(HttpResponse::ok()
                    .with_header("Content-Type".to_string(), "text/plain".to_string())
                    .with_body(b"Hello, World!".to_vec()))
            })
        })),
        constraints: None,
    });

    // JSON endpoint
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/json".to_string(),
        handler: from_legacy_handler(Arc::new(|_req: HttpRequest| {
            Box::pin(async {
                Ok(HttpResponse::ok().with_json(&JsonResponse {
                    message: "Hello, World!",
                })?)
            })
        })),
        constraints: None,
    });

    // Path parameter endpoint
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/users/:id".to_string(),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                let id = req
                    .path_params
                    .get("id")
                    .cloned()
                    .unwrap_or_else(|| "0".to_string());

                Ok(HttpResponse::ok().with_json(&UserResponse {
                    id: id.clone(),
                    name: format!("User {}", id),
                    email: format!("user{}@example.com", id),
                })?)
            })
        })),
        constraints: None,
    });

    // JSON POST endpoint
    router.add_route(Route {
        method: HttpMethod::POST,
        path: "/api/users".to_string(),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                let payload: CreateUserRequest = req.json()?;

                Ok(HttpResponse::created().with_json(&CreateUserResponse {
                    id: 12345,
                    name: payload.name,
                    email: payload
                        .email
                        .unwrap_or_else(|| "default@example.com".to_string()),
                    created: true,
                })?)
            })
        })),
        constraints: None,
    });

    // Health check
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/health".to_string(),
        handler: from_legacy_handler(Arc::new(|_req: HttpRequest| {
            Box::pin(async {
                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "status": "healthy",
                    "framework": "armature"
                }))?)
            })
        })),
        constraints: None,
    });

    // Complex data endpoint for large payload benchmarks
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/data".to_string(),
        handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
            Box::pin(async move {
                let size = req.query_param("size").unwrap_or("medium");
                let count = match size {
                    "small" => 10,
                    "large" => 100,
                    "xlarge" => 500,
                    _ => 50, // medium
                };

                let products = generate_products(count);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();

                Ok(HttpResponse::ok().with_json(&DataResponse {
                    data: products,
                    meta: DataMeta {
                        total: count,
                        page: 1,
                        per_page: count,
                        timestamp: now,
                    },
                })?)
            })
        })),
        constraints: None,
    });

    let container = Container::new();
    let app = Application::new(container, router);

    println!("Endpoints:");
    println!("  GET  http://localhost:3000/            Plaintext");
    println!("  GET  http://localhost:3000/json        JSON response");
    println!("  GET  http://localhost:3000/users/:id   Path parameter");
    println!("  POST http://localhost:3000/api/users   JSON body parsing");
    println!("  GET  http://localhost:3000/health      Health check");
    println!("  GET  http://localhost:3000/data        Complex data (small/medium/large/xlarge)");
    println!();
    println!("Benchmark commands:");
    println!("  oha -z 10s -c 50 http://localhost:3000/");
    println!("  oha -z 10s -c 50 http://localhost:3000/json");
    println!("  oha -z 10s -c 50 http://localhost:3000/data?size=medium");
    println!("  wrk -t4 -c50 -d10s http://localhost:3000/");
    println!();
    println!("Server running on http://localhost:3000");
    println!("Press Ctrl+C to stop\n");

    app.listen(3000).await?;

    Ok(())
}
