//! Armature API - Minimal Template
//!
//! A simple REST API demonstrating basic Armature features.
//!
//! Run with: cargo run
//! Test with: curl http://localhost:3000/health

use armature::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};

// =============================================================================
// Models
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateUser {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }
}

// =============================================================================
// Service
// =============================================================================

/// In-memory user store.
///
/// Handlers are free functions (see below), so the service is shared across
/// requests via a process-wide `OnceLock` rather than constructor injection.
/// This is the same pattern used by `examples/crud_api.rs` in the main repo.
pub struct UserService {
    users: RwLock<Vec<User>>,
    next_id: AtomicU64,
}

impl UserService {
    pub fn new() -> Self {
        Self {
            users: RwLock::new(vec![
                User {
                    id: 1,
                    name: "Alice".to_string(),
                    email: "alice@example.com".to_string(),
                },
                User {
                    id: 2,
                    name: "Bob".to_string(),
                    email: "bob@example.com".to_string(),
                },
            ]),
            next_id: AtomicU64::new(3),
        }
    }

    pub fn get_all(&self) -> Vec<User> {
        self.users.read().unwrap().clone()
    }

    pub fn get_by_id(&self, id: u64) -> Option<User> {
        self.users.read().unwrap().iter().find(|u| u.id == id).cloned()
    }

    pub fn create(&self, create: CreateUser) -> User {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let user = User {
            id,
            name: create.name,
            email: create.email,
        };
        self.users.write().unwrap().push(user.clone());
        user
    }

    pub fn delete(&self, id: u64) -> bool {
        let mut users = self.users.write().unwrap();
        let initial_len = users.len();
        users.retain(|u| u.id != id);
        users.len() != initial_len
    }
}

// NOTE: This OnceLock-based singleton service pattern is duplicated with slight
// variations across sibling templates (`api-full`, `microservice`); a shared
// helper could consolidate these in a future pass.
static USER_SERVICE: OnceLock<UserService> = OnceLock::new();

fn user_service() -> &'static UserService {
    USER_SERVICE.get().expect("UserService not initialized")
}

// =============================================================================
// Controllers
// =============================================================================

#[controller("/health")]
#[derive(Default, Clone)]
struct HealthController;

#[routes]
impl HealthController {
    #[get("")]
    async fn check() -> Result<HttpResponse, Error> {
        HttpResponse::json(&serde_json::json!({
            "status": "healthy",
            "timestamp": current_timestamp(),
        }))
    }
}

fn current_timestamp() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[controller("/api/users")]
#[derive(Default, Clone)]
struct UserController;

#[routes]
impl UserController {
    /// GET /api/users - List all users
    #[get("")]
    async fn list() -> Result<HttpResponse, Error> {
        let users = user_service().get_all();
        HttpResponse::json(&ApiResponse::success(users))
    }

    /// GET /api/users/:id - Get a user by ID
    #[get("/:id")]
    async fn get(req: HttpRequest) -> Result<HttpResponse, Error> {
        let id: u64 = req
            .param("id")
            .and_then(|id| id.parse().ok())
            .ok_or_else(|| Error::bad_request("Invalid user ID"))?;

        match user_service().get_by_id(id) {
            Some(user) => HttpResponse::json(&ApiResponse::success(user)),
            None => Err(Error::not_found("User not found")),
        }
    }

    /// POST /api/users - Create a new user
    #[post("")]
    async fn create(req: HttpRequest) -> Result<HttpResponse, Error> {
        // Do not leak raw serde_json parse error text to clients; `tracing` isn't
        // imported in this file, so the detailed error is simply dropped here.
        let body: CreateUser = req
            .json()
            .map_err(|_| Error::bad_request("Invalid request body"))?;

        let user = user_service().create(body);
        HttpResponse::created().with_json(&ApiResponse::success(user))
    }

    /// DELETE /api/users/:id - Delete a user
    #[delete("/:id")]
    async fn remove(req: HttpRequest) -> Result<HttpResponse, Error> {
        let id: u64 = req
            .param("id")
            .and_then(|id| id.parse().ok())
            .ok_or_else(|| Error::bad_request("Invalid user ID"))?;

        if user_service().delete(id) {
            Ok(HttpResponse::no_content())
        } else {
            Err(Error::not_found("User not found"))
        }
    }
}

// =============================================================================
// Module
// =============================================================================

#[module(
    controllers: [HealthController, UserController]
)]
#[derive(Default)]
struct AppModule;

// =============================================================================
// Main
// =============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Armature API (Minimal Template)");

    USER_SERVICE
        .set(UserService::new())
        .unwrap_or_else(|_| panic!("UserService already initialized"));

    println!("📡 Server running at http://localhost:3000");
    println!();
    println!("Available endpoints:");
    println!("  GET    /health         - Health check");
    println!("  GET    /api/users      - List all users");
    println!("  GET    /api/users/:id  - Get user by ID");
    println!("  POST   /api/users      - Create user");
    println!("  DELETE /api/users/:id  - Delete user");
    println!();
    println!("Try: curl http://localhost:3000/api/users");

    let app = Application::create::<AppModule>().await;
    app.listen(3000).await?;

    Ok(())
}
