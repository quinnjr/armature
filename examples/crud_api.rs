#![allow(clippy::all)]
//! Complete CRUD API Example
//!
//! This example demonstrates how to build a complete REST API with Armature
//! including all CRUD operations, validation, error handling, and DI.
//!
//! Run with: `cargo run --example crud_api`

#![allow(dead_code)]

use armature::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

// =============================================================================
// Domain Models
// =============================================================================

/// A user in our system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub name: String,
    pub email: String,
    #[serde(default)]
    pub active: bool,
}

/// Request payload for creating a user.
#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub name: String,
    pub email: String,
}

/// Request payload for updating a user.
#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub name: Option<String>,
    pub email: Option<String>,
    pub active: Option<bool>,
}

/// Paginated response wrapper.
#[derive(Debug, Serialize)]
pub struct PagedResponse<T> {
    pub data: Vec<T>,
    pub total: usize,
    pub page: usize,
    pub per_page: usize,
}

// =============================================================================
// Repository (In-Memory Storage)
// =============================================================================

/// In-memory repository for users.
#[derive(Debug, Clone, Default)]
pub struct UserRepository {
    users: Arc<RwLock<HashMap<u64, User>>>,
    next_id: Arc<RwLock<u64>>,
}

impl UserRepository {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed with some initial data.
    pub fn with_seed_data() -> Self {
        let repo = Self::new();
        let users = vec![
            User {
                id: 1,
                name: "Alice".to_string(),
                email: "alice@example.com".to_string(),
                active: true,
            },
            User {
                id: 2,
                name: "Bob".to_string(),
                email: "bob@example.com".to_string(),
                active: true,
            },
            User {
                id: 3,
                name: "Charlie".to_string(),
                email: "charlie@example.com".to_string(),
                active: false,
            },
        ];

        {
            let mut store = repo.users.write().unwrap();
            for user in users {
                store.insert(user.id, user);
            }
            *repo.next_id.write().unwrap() = 4;
        }

        repo
    }

    pub fn find_all(&self, page: usize, per_page: usize) -> PagedResponse<User> {
        let store = self.users.read().unwrap();
        let total = store.len();
        let data: Vec<User> = store
            .values()
            .skip((page - 1) * per_page)
            .take(per_page)
            .cloned()
            .collect();

        PagedResponse {
            data,
            total,
            page,
            per_page,
        }
    }

    pub fn find_by_id(&self, id: u64) -> Option<User> {
        self.users.read().unwrap().get(&id).cloned()
    }

    pub fn create(&self, name: String, email: String) -> User {
        let mut next_id = self.next_id.write().unwrap();
        let id = *next_id;
        *next_id += 1;

        let user = User {
            id,
            name,
            email,
            active: true,
        };

        self.users.write().unwrap().insert(id, user.clone());
        user
    }

    pub fn update(&self, id: u64, update: UpdateUserRequest) -> Option<User> {
        let mut store = self.users.write().unwrap();
        if let Some(user) = store.get_mut(&id) {
            if let Some(name) = update.name {
                user.name = name;
            }
            if let Some(email) = update.email {
                user.email = email;
            }
            if let Some(active) = update.active {
                user.active = active;
            }
            Some(user.clone())
        } else {
            None
        }
    }

    pub fn delete(&self, id: u64) -> bool {
        self.users.write().unwrap().remove(&id).is_some()
    }
}

// =============================================================================
// Service Layer
// =============================================================================

/// User service with business logic.
#[derive(Debug, Clone)]
pub struct UserService {
    repository: UserRepository,
}

impl UserService {
    pub fn new(repository: UserRepository) -> Self {
        Self { repository }
    }

    pub fn list_users(&self, page: usize, per_page: usize) -> PagedResponse<User> {
        let page = page.max(1);
        let per_page = per_page.clamp(1, 100);
        self.repository.find_all(page, per_page)
    }

    pub fn get_user(&self, id: u64) -> std::result::Result<User, String> {
        self.repository
            .find_by_id(id)
            .ok_or_else(|| format!("User with id {} not found", id))
    }

    pub fn create_user(&self, req: CreateUserRequest) -> std::result::Result<User, String> {
        // Validation
        if req.name.trim().is_empty() {
            return Err("Name is required".to_string());
        }
        if req.email.trim().is_empty() {
            return Err("Email is required".to_string());
        }
        if !req.email.contains('@') {
            return Err("Invalid email format".to_string());
        }

        Ok(self.repository.create(req.name, req.email))
    }

    pub fn update_user(
        &self,
        id: u64,
        req: UpdateUserRequest,
    ) -> std::result::Result<User, String> {
        // Validate email if provided
        if let Some(ref email) = req.email {
            if !email.contains('@') {
                return Err("Invalid email format".to_string());
            }
        }

        self.repository
            .update(id, req)
            .ok_or_else(|| format!("User with id {} not found", id))
    }

    pub fn delete_user(&self, id: u64) -> std::result::Result<(), String> {
        if self.repository.delete(id) {
            Ok(())
        } else {
            Err(format!("User with id {} not found", id))
        }
    }
}

// =============================================================================
// Global state for sharing service
// =============================================================================

static USER_SERVICE: std::sync::OnceLock<UserService> = std::sync::OnceLock::new();

fn get_user_service() -> &'static UserService {
    USER_SERVICE.get().expect("UserService not initialized")
}

// =============================================================================
// Controller
// =============================================================================

/// REST API controller for users.
#[controller("/api/users")]
#[derive(Default, Clone)]
struct UserController;

#[routes]
impl UserController {
    /// GET /api/users - List all users with pagination
    #[get("")]
    async fn list_users(req: HttpRequest) -> Result<HttpResponse, Error> {
        let page: usize = req
            .query_param("page")
            .and_then(|p| p.parse().ok())
            .unwrap_or(1);
        let per_page: usize = req
            .query_param("per_page")
            .and_then(|p| p.parse().ok())
            .unwrap_or(10);

        let result = get_user_service().list_users(page, per_page);
        HttpResponse::json(&result)
    }

    /// GET /api/users/:id - Get a user by ID
    #[get("/:id")]
    async fn get_user(req: HttpRequest) -> Result<HttpResponse, Error> {
        let id: u64 = req
            .param("id")
            .and_then(|id| id.parse().ok())
            .ok_or_else(|| Error::bad_request("Invalid user ID"))?;

        match get_user_service().get_user(id) {
            Ok(user) => HttpResponse::json(&user),
            Err(msg) => Err(Error::not_found(msg)),
        }
    }

    /// POST /api/users - Create a new user
    #[post("")]
    async fn create_user(req: HttpRequest) -> Result<HttpResponse, Error> {
        let body: CreateUserRequest = req
            .json()
            .map_err(|e| Error::bad_request(format!("Invalid JSON: {}", e)))?;

        match get_user_service().create_user(body) {
            Ok(user) => {
                let mut response = HttpResponse::created();
                response = response.with_json(&user)?;
                Ok(response)
            }
            Err(msg) => Err(Error::validation(msg)),
        }
    }

    /// PUT /api/users/:id - Update an existing user
    #[put("/:id")]
    async fn update_user(req: HttpRequest) -> Result<HttpResponse, Error> {
        let id: u64 = req
            .param("id")
            .and_then(|id| id.parse().ok())
            .ok_or_else(|| Error::bad_request("Invalid user ID"))?;

        let body: UpdateUserRequest = req
            .json()
            .map_err(|e| Error::bad_request(format!("Invalid JSON: {}", e)))?;

        match get_user_service().update_user(id, body) {
            Ok(user) => HttpResponse::json(&user),
            Err(msg) => Err(Error::not_found(msg)),
        }
    }

    /// DELETE /api/users/:id - Delete a user
    #[delete("/:id")]
    async fn delete_user(req: HttpRequest) -> Result<HttpResponse, Error> {
        let id: u64 = req
            .param("id")
            .and_then(|id| id.parse().ok())
            .ok_or_else(|| Error::bad_request("Invalid user ID"))?;

        match get_user_service().delete_user(id) {
            Ok(()) => Ok(HttpResponse::no_content()),
            Err(msg) => Err(Error::not_found(msg)),
        }
    }
}

// =============================================================================
// Module
// =============================================================================

#[module(
    controllers: [UserController]
)]
#[derive(Default)]
struct AppModule;

// =============================================================================
// Main
// =============================================================================

#[tokio::main]
async fn main() {
    println!("Starting CRUD API example");

    // Create and register services
    let repository = UserRepository::with_seed_data();
    let service = UserService::new(repository);
    USER_SERVICE
        .set(service)
        .expect("Failed to set user service");

    println!("Server running at http://127.0.0.1:3000");
    println!("Try these endpoints:");
    println!("  GET    /api/users          - List all users");
    println!("  GET    /api/users/1        - Get user by ID");
    println!("  POST   /api/users          - Create a user");
    println!("  PUT    /api/users/1        - Update a user");
    println!("  DELETE /api/users/1        - Delete a user");

    let app = Application::create::<AppModule>().await;
    app.listen(3000).await.unwrap();
}
