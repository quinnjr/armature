//! Armature Micro-Framework Benchmark Server
//! Port: 3000
//!
//! Run with: cargo run --release --example micro_benchmark_server

use armature_core::micro::*;
use armature_core::{Error, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};

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

async fn plaintext(_req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok()
        .with_header("Content-Type".to_string(), "text/plain".to_string())
        .with_body(b"Hello, World!".to_vec()))
}

async fn json(_req: HttpRequest) -> Result<HttpResponse, Error> {
    HttpResponse::json(&JsonResponse {
        message: "Hello, World!",
    })
}

async fn get_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    let id = req.param("id").cloned().unwrap_or_default();
    HttpResponse::json(&UserResponse {
        id: id.clone(),
        name: format!("User {}", id),
        email: format!("user{}@example.com", id),
    })
}

async fn create_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    let body: CreateUserRequest = serde_json::from_slice(&req.body)
        .map_err(|e| Error::validation(format!("Invalid JSON: {}", e)))?;

    let response = CreateUserResponse {
        id: 12345,
        name: body.name,
        email: body
            .email
            .unwrap_or_else(|| "default@example.com".to_string()),
        created: true,
    };

    let json = serde_json::to_vec(&response)
        .map_err(|e| Error::internal(format!("JSON serialization error: {}", e)))?;

    Ok(HttpResponse::new(201)
        .with_header("Content-Type".to_string(), "application/json".to_string())
        .with_body(json))
}

async fn health(_req: HttpRequest) -> Result<HttpResponse, Error> {
    HttpResponse::json(&serde_json::json!({
        "status": "healthy",
        "framework": "armature-micro"
    }))
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    println!("🚀 Armature Micro-Framework Benchmark Server on http://localhost:3000");

    App::new()
        .route("/", get(plaintext))
        .route("/json", get(json))
        .route("/users/:id", get(get_user))
        .route("/api/users", post(create_user))
        .route("/health", get(health))
        .run("127.0.0.1:3000")
        .await
}
