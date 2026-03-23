//! Code generation templates for Armature CLI.

use handlebars::Handlebars;
use serde::Serialize;

/// Template registry for code generation.
pub struct TemplateRegistry {
    hbs: Handlebars<'static>,
}

impl TemplateRegistry {
    /// Create a new template registry with all templates registered.
    pub fn new() -> Self {
        let mut hbs = Handlebars::new();
        hbs.set_strict_mode(true);

        // Register all templates
        hbs.register_template_string("controller", CONTROLLER_TEMPLATE)
            .expect("Failed to register controller template");
        hbs.register_template_string("controller_crud", CONTROLLER_CRUD_TEMPLATE)
            .expect("Failed to register controller CRUD template");
        hbs.register_template_string("controller_test", CONTROLLER_TEST_TEMPLATE)
            .expect("Failed to register controller test template");
        hbs.register_template_string("module", MODULE_TEMPLATE)
            .expect("Failed to register module template");
        hbs.register_template_string("middleware", MIDDLEWARE_TEMPLATE)
            .expect("Failed to register middleware template");
        hbs.register_template_string("middleware_test", MIDDLEWARE_TEST_TEMPLATE)
            .expect("Failed to register middleware test template");
        hbs.register_template_string("guard", GUARD_TEMPLATE)
            .expect("Failed to register guard template");
        hbs.register_template_string("guard_test", GUARD_TEST_TEMPLATE)
            .expect("Failed to register guard test template");
        hbs.register_template_string("service", SERVICE_TEMPLATE)
            .expect("Failed to register service template");
        hbs.register_template_string("service_test", SERVICE_TEST_TEMPLATE)
            .expect("Failed to register service test template");
        hbs.register_template_string("main_minimal", MAIN_MINIMAL_TEMPLATE)
            .expect("Failed to register main minimal template");
        hbs.register_template_string("cargo_toml", CARGO_TOML_TEMPLATE)
            .expect("Failed to register Cargo.toml template");
        hbs.register_template_string("env_example", ENV_EXAMPLE_TEMPLATE)
            .expect("Failed to register .env.example template");
        hbs.register_template_string("readme", README_TEMPLATE)
            .expect("Failed to register README template");

        // Additional templates
        hbs.register_template_string("repository", REPOSITORY_TEMPLATE)
            .expect("Failed to register repository template");
        hbs.register_template_string("repository_test", REPOSITORY_TEST_TEMPLATE)
            .expect("Failed to register repository test template");
        hbs.register_template_string("dto", DTO_TEMPLATE)
            .expect("Failed to register DTO template");
        hbs.register_template_string("websocket", WEBSOCKET_TEMPLATE)
            .expect("Failed to register WebSocket template");
        hbs.register_template_string("websocket_test", WEBSOCKET_TEST_TEMPLATE)
            .expect("Failed to register WebSocket test template");
        hbs.register_template_string("graphql_resolver", GRAPHQL_RESOLVER_TEMPLATE)
            .expect("Failed to register GraphQL resolver template");
        hbs.register_template_string("graphql_resolver_test", GRAPHQL_RESOLVER_TEST_TEMPLATE)
            .expect("Failed to register GraphQL resolver test template");
        hbs.register_template_string("job", JOB_TEMPLATE)
            .expect("Failed to register job template");
        hbs.register_template_string("job_test", JOB_TEST_TEMPLATE)
            .expect("Failed to register job test template");
        hbs.register_template_string("event_handler", EVENT_HANDLER_TEMPLATE)
            .expect("Failed to register event handler template");
        hbs.register_template_string("event_handler_test", EVENT_HANDLER_TEST_TEMPLATE)
            .expect("Failed to register event handler test template");
        hbs.register_template_string("interceptor", INTERCEPTOR_TEMPLATE)
            .expect("Failed to register interceptor template");
        hbs.register_template_string("interceptor_test", INTERCEPTOR_TEST_TEMPLATE)
            .expect("Failed to register interceptor test template");
        hbs.register_template_string("pipe", PIPE_TEMPLATE)
            .expect("Failed to register pipe template");
        hbs.register_template_string("pipe_test", PIPE_TEST_TEMPLATE)
            .expect("Failed to register pipe test template");
        hbs.register_template_string("exception_filter", EXCEPTION_FILTER_TEMPLATE)
            .expect("Failed to register exception filter template");
        hbs.register_template_string("exception_filter_test", EXCEPTION_FILTER_TEST_TEMPLATE)
            .expect("Failed to register exception filter test template");
        hbs.register_template_string("config", CONFIG_TEMPLATE)
            .expect("Failed to register config template");
        hbs.register_template_string("entity", ENTITY_TEMPLATE)
            .expect("Failed to register entity template");
        hbs.register_template_string("entity_prax", ENTITY_PRAX_TEMPLATE)
            .expect("Failed to register Prax entity template");
        hbs.register_template_string("prax_schema", PRAX_SCHEMA_TEMPLATE)
            .expect("Failed to register Prax schema template");
        hbs.register_template_string("prax_repository", PRAX_REPOSITORY_TEMPLATE)
            .expect("Failed to register Prax repository template");
        hbs.register_template_string("prax_repository_test", PRAX_REPOSITORY_TEST_TEMPLATE)
            .expect("Failed to register Prax repository test template");
        hbs.register_template_string("prax_module", PRAX_MODULE_TEMPLATE)
            .expect("Failed to register Prax module template");
        hbs.register_template_string("rhai_handler", RHAI_HANDLER_TEMPLATE)
            .expect("Failed to register Rhai handler template");
        hbs.register_template_string("health_controller", HEALTH_CONTROLLER_TEMPLATE)
            .expect("Failed to register health controller template");
        hbs.register_template_string("dockerfile", DOCKERFILE_TEMPLATE)
            .expect("Failed to register Dockerfile template");
        hbs.register_template_string("docker_compose", DOCKER_COMPOSE_TEMPLATE)
            .expect("Failed to register docker-compose template");
        hbs.register_template_string("github_actions", GITHUB_ACTIONS_TEMPLATE)
            .expect("Failed to register GitHub Actions template");
        hbs.register_template_string("integration_test", INTEGRATION_TEST_TEMPLATE)
            .expect("Failed to register integration test template");
        hbs.register_template_string("scheduler", SCHEDULER_TEMPLATE)
            .expect("Failed to register scheduler template");
        hbs.register_template_string("scheduler_test", SCHEDULER_TEST_TEMPLATE)
            .expect("Failed to register scheduler test template");
        hbs.register_template_string("cache_service", CACHE_SERVICE_TEMPLATE)
            .expect("Failed to register cache service template");
        hbs.register_template_string("cache_service_test", CACHE_SERVICE_TEST_TEMPLATE)
            .expect("Failed to register cache service test template");
        hbs.register_template_string("api_client", API_CLIENT_TEMPLATE)
            .expect("Failed to register API client template");

        Self { hbs }
    }

    /// Render a template with the given data.
    pub fn render<T: Serialize>(&self, template: &str, data: &T) -> Result<String, String> {
        self.hbs.render(template, data).map_err(|e| e.to_string())
    }
}

impl Default for TemplateRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// CONTROLLER TEMPLATES
// =============================================================================

const CONTROLLER_TEMPLATE: &str = r#"//! {{name_pascal}} controller.

use armature::prelude::*;

/// {{name_pascal}} controller handles {{name_snake}} related endpoints.
#[controller("/{{base_path}}")]
#[derive(Default)]
pub struct {{name_pascal}}Controller;

impl {{name_pascal}}Controller {
    /// Get all {{name_snake}}s.
    #[get("/")]
    pub async fn index(&self, _req: HttpRequest) -> Result<HttpResponse, Error> {
        HttpResponse::ok().with_json(&serde_json::json!({
            "message": "List all {{name_snake}}s"
        }))
    }

    /// Get a single {{name_snake}} by ID.
    #[get("/:id")]
    pub async fn show(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        let id = req.params.get("id").unwrap_or(&"0".to_string()).clone();
        HttpResponse::ok().with_json(&serde_json::json!({
            "message": format!("Get {{name_snake}} with id: {}", id)
        }))
    }
}
"#;

const CONTROLLER_CRUD_TEMPLATE: &str = r#"//! {{name_pascal}} controller with CRUD operations.

use armature::prelude::*;
use serde::{Deserialize, Serialize};

/// {{name_pascal}} data transfer object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct {{name_pascal}}Dto {
    pub id: Option<u64>,
    pub name: String,
    // Add more fields as needed
}

/// Create {{name_pascal}} request.
#[derive(Debug, Deserialize)]
pub struct Create{{name_pascal}}Request {
    pub name: String,
}

/// Update {{name_pascal}} request.
#[derive(Debug, Deserialize)]
pub struct Update{{name_pascal}}Request {
    pub name: Option<String>,
}

/// {{name_pascal}} controller handles {{name_snake}} CRUD operations.
#[controller("/{{base_path}}")]
#[derive(Default)]
pub struct {{name_pascal}}Controller;

impl {{name_pascal}}Controller {
    /// List all {{name_snake}}s.
    ///
    /// GET /{{base_path}}
    #[get("/")]
    pub async fn index(&self, _req: HttpRequest) -> Result<HttpResponse, Error> {
        // TODO: Implement listing logic
        let items: Vec<{{name_pascal}}Dto> = vec![];
        HttpResponse::ok().with_json(&items)
    }

    /// Get a single {{name_snake}} by ID.
    ///
    /// GET /{{base_path}}/:id
    #[get("/:id")]
    pub async fn show(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        let id = req.params.get("id")
            .ok_or_else(|| Error::BadRequest("Missing id parameter".to_string()))?;

        // TODO: Implement fetch logic
        let item = {{name_pascal}}Dto {
            id: Some(id.parse().unwrap_or(0)),
            name: "Example".to_string(),
        };

        HttpResponse::ok().with_json(&item)
    }

    /// Create a new {{name_snake}}.
    ///
    /// POST /{{base_path}}
    #[post("/")]
    pub async fn create(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        let body: Create{{name_pascal}}Request = serde_json::from_slice(&req.body)
            .map_err(|e| Error::BadRequest(format!("Invalid request body: {}", e)))?;

        // TODO: Implement create logic
        let item = {{name_pascal}}Dto {
            id: Some(1),
            name: body.name,
        };

        HttpResponse::created().with_json(&item)
    }

    /// Update an existing {{name_snake}}.
    ///
    /// PUT /{{base_path}}/:id
    #[put("/:id")]
    pub async fn update(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        let id = req.params.get("id")
            .ok_or_else(|| Error::BadRequest("Missing id parameter".to_string()))?;

        let body: Update{{name_pascal}}Request = serde_json::from_slice(&req.body)
            .map_err(|e| Error::BadRequest(format!("Invalid request body: {}", e)))?;

        // TODO: Implement update logic
        let item = {{name_pascal}}Dto {
            id: Some(id.parse().unwrap_or(0)),
            name: body.name.unwrap_or_else(|| "Updated".to_string()),
        };

        HttpResponse::ok().with_json(&item)
    }

    /// Delete a {{name_snake}}.
    ///
    /// DELETE /{{base_path}}/:id
    #[delete("/:id")]
    pub async fn destroy(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        let id = req.params.get("id")
            .ok_or_else(|| Error::BadRequest("Missing id parameter".to_string()))?;

        // TODO: Implement delete logic
        let _ = id;

        HttpResponse::no_content()
    }
}
"#;

const CONTROLLER_TEST_TEMPLATE: &str = r#"//! Tests for {{name_pascal}}Controller.

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_index() {
        let controller = {{name_pascal}}Controller::default();
        let req = HttpRequest::default();

        let response = controller.index(req).await;
        assert!(response.is_ok());
    }

    #[tokio::test]
    async fn test_show() {
        let controller = {{name_pascal}}Controller::default();
        let mut req = HttpRequest::default();
        req.params.insert("id".to_string(), "1".to_string());

        let response = controller.show(req).await;
        assert!(response.is_ok());
    }
}
"#;

// =============================================================================
// MODULE TEMPLATES
// =============================================================================

const MODULE_TEMPLATE: &str = r#"//! {{name_pascal}} module.

use armature::prelude::*;

{{#each controllers}}
mod {{this}};
pub use {{this}}::{{this_pascal}}Controller;
{{/each}}

{{#each providers}}
mod {{this}};
pub use {{this}}::{{this_pascal}}Service;
{{/each}}

/// {{name_pascal}} module bundles related controllers and providers.
#[module(
    controllers: [{{controller_list}}],
    providers: [{{provider_list}}]
)]
#[derive(Default)]
pub struct {{name_pascal}}Module;
"#;

// =============================================================================
// MIDDLEWARE TEMPLATES
// =============================================================================

const MIDDLEWARE_TEMPLATE: &str = r#"//! {{name_pascal}} middleware.

use armature::prelude::*;
use async_trait::async_trait;

/// {{name_pascal}} middleware.
///
/// # Example
///
/// ```rust
/// use armature::prelude::*;
///
/// let middleware = {{name_pascal}}Middleware::new();
/// ```
pub struct {{name_pascal}}Middleware {
    // Add configuration fields here
}

impl {{name_pascal}}Middleware {
    /// Create a new {{name_pascal}}Middleware instance.
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for {{name_pascal}}Middleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware for {{name_pascal}}Middleware {
    async fn handle(&self, req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        // Pre-processing: Add logic before the request is handled
        tracing::debug!("{{name_pascal}}Middleware: Processing request to {}", req.path);

        // Call the next middleware/handler
        let response = next(req).await?;

        // Post-processing: Add logic after the response is generated
        tracing::debug!("{{name_pascal}}Middleware: Response status {}", response.status);

        Ok(response)
    }
}
"#;

const MIDDLEWARE_TEST_TEMPLATE: &str = r#"//! Tests for {{name_pascal}}Middleware.

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_middleware_passes_through() {
        let middleware = {{name_pascal}}Middleware::new();
        let req = HttpRequest::default();

        let next: Next = Box::new(|_req| {
            Box::pin(async { Ok(HttpResponse::ok()) })
        });

        let response = middleware.handle(req, next).await;
        assert!(response.is_ok());
        assert_eq!(response.unwrap().status, 200);
    }
}
"#;

// =============================================================================
// GUARD TEMPLATES
// =============================================================================

const GUARD_TEMPLATE: &str = r#"//! {{name_pascal}} guard.

use armature::prelude::*;
use async_trait::async_trait;

/// {{name_pascal}} guard for route protection.
///
/// # Example
///
/// ```rust
/// use armature::prelude::*;
///
/// let guard = {{name_pascal}}Guard::new();
/// ```
pub struct {{name_pascal}}Guard {
    // Add configuration fields here
}

impl {{name_pascal}}Guard {
    /// Create a new {{name_pascal}}Guard instance.
    pub fn new() -> Self {
        Self {}
    }

    /// Check if the request is authorized.
    fn is_authorized(&self, req: &HttpRequest) -> bool {
        // TODO: Implement authorization logic
        // Example: Check for a valid API key or JWT token
        req.headers.contains_key("authorization")
    }
}

impl Default for {{name_pascal}}Guard {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Guard for {{name_pascal}}Guard {
    async fn can_activate(&self, req: &HttpRequest) -> Result<bool, Error> {
        if self.is_authorized(req) {
            Ok(true)
        } else {
            Err(Error::Unauthorized("Access denied".to_string()))
        }
    }
}
"#;

const GUARD_TEST_TEMPLATE: &str = r#"//! Tests for {{name_pascal}}Guard.

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_guard_denies_unauthorized() {
        let guard = {{name_pascal}}Guard::new();
        let req = HttpRequest::default();

        let result = guard.can_activate(&req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_guard_allows_authorized() {
        let guard = {{name_pascal}}Guard::new();
        let mut req = HttpRequest::default();
        req.headers.insert("authorization".to_string(), "Bearer token".to_string());

        let result = guard.can_activate(&req).await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }
}
"#;

// =============================================================================
// SERVICE TEMPLATES
// =============================================================================

const SERVICE_TEMPLATE: &str = r#"//! {{name_pascal}} service.

use armature::prelude::*;
use std::sync::Arc;

/// {{name_pascal}} service provides business logic for {{name_snake}} operations.
///
/// # Example
///
/// ```rust
/// use armature::prelude::*;
///
/// let service = {{name_pascal}}Service::new();
/// ```
#[derive(Clone)]
#[injectable]
pub struct {{name_pascal}}Service {
    // Add dependencies here
}

impl {{name_pascal}}Service {
    /// Create a new {{name_pascal}}Service instance.
    pub fn new() -> Self {
        Self {}
    }

    /// Example method - replace with your business logic.
    pub async fn find_all(&self) -> Result<Vec<String>, Error> {
        // TODO: Implement business logic
        Ok(vec![])
    }

    /// Example method - replace with your business logic.
    pub async fn find_by_id(&self, id: u64) -> Result<Option<String>, Error> {
        // TODO: Implement business logic
        let _ = id;
        Ok(None)
    }
}

impl Default for {{name_pascal}}Service {
    fn default() -> Self {
        Self::new()
    }
}
"#;

const SERVICE_TEST_TEMPLATE: &str = r#"//! Tests for {{name_pascal}}Service.

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_find_all() {
        let service = {{name_pascal}}Service::new();
        let result = service.find_all().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_find_by_id() {
        let service = {{name_pascal}}Service::new();
        let result = service.find_by_id(1).await;
        assert!(result.is_ok());
    }
}
"#;

// =============================================================================
// PROJECT TEMPLATES
// =============================================================================

const MAIN_MINIMAL_TEMPLATE: &str = r#"//! {{name_pascal}} - Built with Armature Framework

use armature::prelude::*;

mod controllers;

use controllers::health::HealthController;

/// Application module.
#[module(controllers: [HealthController])]
#[derive(Default)]
struct AppModule;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Create and run the application
    let app = Application::create::<AppModule>().await;

    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse()
        .unwrap_or(3000);

    println!("🚀 Server running on http://127.0.0.1:{}", port);

    app.listen(port).await?;

    Ok(())
}
"#;

const CARGO_TOML_TEMPLATE: &str = r#"[package]
name = "{{name_kebab}}"
version = "0.1.0"
edition = "2021"
authors = ["Your Name <your.email@example.com>"]
description = "{{description}}"

[dependencies]
armature = "0.1"
tokio = { version = "1.0", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tracing = "0.1"
tracing-subscriber = "0.3"
async-trait = "0.1"
thiserror = "2.0"

[dev-dependencies]
tokio-test = "0.4"
"#;

const ENV_EXAMPLE_TEMPLATE: &str = r#"# {{name_pascal}} Environment Configuration

# Server
PORT=3000
HOST=127.0.0.1

# Logging
RUST_LOG=info

# Database (if needed)
# DATABASE_URL=postgres://user:password@localhost:5432/{{name_snake}}

# Redis (if needed)
# REDIS_URL=redis://localhost:6379

# JWT (if needed)
# JWT_SECRET=your-secret-key-here
# JWT_EXPIRATION=3600
"#;

const README_TEMPLATE: &str = r#"# {{name_pascal}}

{{description}}

Built with [Armature](https://github.com/pegasusheavy/armature) - A modern Rust web framework.

## Getting Started

### Prerequisites

- Rust 1.75 or later
- Cargo

### Installation

1. Clone the repository
2. Copy `.env.example` to `.env` and configure
3. Run the development server:

```bash
cargo run
```

Or with the Armature CLI:

```bash
armature dev
```

### Development

Generate new code:

```bash
# Generate a controller
armature generate controller users

# Generate a service
armature generate service users

# Generate a complete resource
armature generate resource products --crud
```

### Building for Production

```bash
cargo build --release
```

## Project Structure

```
{{name_kebab}}/
├── src/
│   ├── main.rs           # Application entry point
│   ├── controllers/      # Route handlers
│   ├── services/         # Business logic
│   ├── middleware/       # Request/response middleware
│   └── guards/           # Route guards
├── Cargo.toml
├── .env.example
└── README.md
```

## License

[Your License]
"#;

// =============================================================================
// REPOSITORY TEMPLATES
// =============================================================================

const REPOSITORY_TEMPLATE: &str = r#"//! {{name_pascal}} repository.

use armature::prelude::*;
use std::sync::Arc;

/// {{name_pascal}} entity type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct {{name_pascal}} {
    pub id: u64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    // Add more fields as needed
}

/// Repository trait for {{name_pascal}} operations.
#[async_trait::async_trait]
pub trait {{name_pascal}}Repository: Send + Sync {
    /// Find all {{name_snake}}s.
    async fn find_all(&self) -> Result<Vec<{{name_pascal}}>, Error>;

    /// Find a {{name_snake}} by ID.
    async fn find_by_id(&self, id: u64) -> Result<Option<{{name_pascal}}>, Error>;

    /// Create a new {{name_snake}}.
    async fn create(&self, entity: {{name_pascal}}) -> Result<{{name_pascal}}, Error>;

    /// Update an existing {{name_snake}}.
    async fn update(&self, entity: {{name_pascal}}) -> Result<{{name_pascal}}, Error>;

    /// Delete a {{name_snake}} by ID.
    async fn delete(&self, id: u64) -> Result<bool, Error>;
}

/// In-memory implementation of {{name_pascal}}Repository.
///
/// Replace with database implementation (Diesel, SeaORM, etc.)
#[derive(Clone)]
#[injectable]
pub struct {{name_pascal}}RepositoryImpl {
    // Add database connection pool here
}

impl {{name_pascal}}RepositoryImpl {
    /// Create a new repository instance.
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for {{name_pascal}}RepositoryImpl {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl {{name_pascal}}Repository for {{name_pascal}}RepositoryImpl {
    async fn find_all(&self) -> Result<Vec<{{name_pascal}}>, Error> {
        // TODO: Implement database query
        Ok(vec![])
    }

    async fn find_by_id(&self, id: u64) -> Result<Option<{{name_pascal}}>, Error> {
        // TODO: Implement database query
        let _ = id;
        Ok(None)
    }

    async fn create(&self, entity: {{name_pascal}}) -> Result<{{name_pascal}}, Error> {
        // TODO: Implement database insert
        Ok(entity)
    }

    async fn update(&self, entity: {{name_pascal}}) -> Result<{{name_pascal}}, Error> {
        // TODO: Implement database update
        Ok(entity)
    }

    async fn delete(&self, id: u64) -> Result<bool, Error> {
        // TODO: Implement database delete
        let _ = id;
        Ok(true)
    }
}
"#;

const REPOSITORY_TEST_TEMPLATE: &str = r#"//! Tests for {{name_pascal}}Repository.

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_find_all() {
        let repo = {{name_pascal}}RepositoryImpl::new();
        let result = repo.find_all().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_find_by_id() {
        let repo = {{name_pascal}}RepositoryImpl::new();
        let result = repo.find_by_id(1).await;
        assert!(result.is_ok());
    }
}
"#;

// =============================================================================
// DTO TEMPLATES
// =============================================================================

const DTO_TEMPLATE: &str = r#"//! {{name_pascal}} Data Transfer Objects.

use serde::{Deserialize, Serialize};
use validator::Validate;

/// {{name_pascal}} response DTO.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct {{name_pascal}}Response {
    pub id: u64,
    pub created_at: String,
    pub updated_at: String,
    // Add more fields as needed
}

/// Create {{name_pascal}} request DTO.
#[derive(Debug, Clone, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct Create{{name_pascal}}Request {
    #[validate(length(min = 1, max = 255, message = "Name must be between 1 and 255 characters"))]
    pub name: String,
    // Add more fields as needed
}

/// Update {{name_pascal}} request DTO.
#[derive(Debug, Clone, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct Update{{name_pascal}}Request {
    #[validate(length(min = 1, max = 255, message = "Name must be between 1 and 255 characters"))]
    pub name: Option<String>,
    // Add more fields as needed
}

/// Query parameters for listing {{name_snake}}s.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct {{name_pascal}}Query {
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub sort_by: Option<String>,
    pub sort_order: Option<SortOrder>,
    pub search: Option<String>,
}

/// Sort order for queries.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    Asc,
    Desc,
}

impl Default for {{name_pascal}}Query {
    fn default() -> Self {
        Self {
            page: Some(1),
            limit: Some(20),
            sort_by: None,
            sort_order: Some(SortOrder::Desc),
            search: None,
        }
    }
}

/// Paginated response wrapper.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Paginated<T> {
    pub data: Vec<T>,
    pub page: u32,
    pub limit: u32,
    pub total: u64,
    pub total_pages: u32,
}

impl<T> Paginated<T> {
    pub fn new(data: Vec<T>, page: u32, limit: u32, total: u64) -> Self {
        let total_pages = ((total as f64) / (limit as f64)).ceil() as u32;
        Self {
            data,
            page,
            limit,
            total,
            total_pages,
        }
    }
}
"#;

// =============================================================================
// WEBSOCKET TEMPLATES
// =============================================================================

const WEBSOCKET_TEMPLATE: &str = r#"//! {{name_pascal}} WebSocket handler.

use armature::prelude::*;
use armature_websocket::{WebSocket, Message, WebSocketHandler};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::broadcast;

/// {{name_pascal}} WebSocket handler.
///
/// Handles real-time {{name_snake}} events.
pub struct {{name_pascal}}WebSocket {
    /// Broadcast channel for messages.
    tx: broadcast::Sender<String>,
}

impl {{name_pascal}}WebSocket {
    /// Create a new WebSocket handler.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(100);
        Self { tx }
    }

    /// Broadcast a message to all connected clients.
    pub fn broadcast(&self, message: &str) {
        let _ = self.tx.send(message.to_string());
    }
}

impl Default for {{name_pascal}}WebSocket {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WebSocketHandler for {{name_pascal}}WebSocket {
    async fn on_connect(&self, ws: &WebSocket) -> Result<(), Error> {
        tracing::info!("{{name_pascal}}WebSocket: Client connected: {}", ws.id());
        Ok(())
    }

    async fn on_message(&self, ws: &WebSocket, msg: Message) -> Result<(), Error> {
        match msg {
            Message::Text(text) => {
                tracing::debug!("{{name_pascal}}WebSocket: Received text: {}", text);

                // Echo back to sender
                ws.send(Message::Text(format!("Echo: {}", text))).await?;

                // Broadcast to all clients
                self.broadcast(&text);
            }
            Message::Binary(data) => {
                tracing::debug!("{{name_pascal}}WebSocket: Received binary: {} bytes", data.len());
                ws.send(Message::Binary(data)).await?;
            }
            Message::Ping(data) => {
                ws.send(Message::Pong(data)).await?;
            }
            Message::Pong(_) => {}
            Message::Close(_) => {
                tracing::info!("{{name_pascal}}WebSocket: Client closing connection");
            }
        }
        Ok(())
    }

    async fn on_disconnect(&self, ws: &WebSocket) -> Result<(), Error> {
        tracing::info!("{{name_pascal}}WebSocket: Client disconnected: {}", ws.id());
        Ok(())
    }

    async fn on_error(&self, ws: &WebSocket, error: Error) -> Result<(), Error> {
        tracing::error!("{{name_pascal}}WebSocket: Error for client {}: {}", ws.id(), error);
        Ok(())
    }
}

/// Create WebSocket route.
pub fn {{name_snake}}_websocket_route() -> impl Fn(HttpRequest) -> Result<HttpResponse, Error> {
    let handler = Arc::new({{name_pascal}}WebSocket::new());

    move |req: HttpRequest| {
        let handler = handler.clone();
        // WebSocket upgrade logic here
        Ok(HttpResponse::ok())
    }
}
"#;

const WEBSOCKET_TEST_TEMPLATE: &str = r#"//! Tests for {{name_pascal}}WebSocket.

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_websocket_creation() {
        let ws = {{name_pascal}}WebSocket::new();
        // Test broadcast doesn't panic with no receivers
        ws.broadcast("test message");
    }
}
"#;

// =============================================================================
// GRAPHQL RESOLVER TEMPLATES
// =============================================================================

const GRAPHQL_RESOLVER_TEMPLATE: &str = r#"//! {{name_pascal}} GraphQL resolver.

use async_graphql::{Context, Object, InputObject, SimpleObject, Result, ID};
use std::sync::Arc;

/// {{name_pascal}} GraphQL type.
#[derive(Debug, Clone, SimpleObject)]
pub struct {{name_pascal}} {
    pub id: ID,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Input for creating a {{name_snake}}.
#[derive(Debug, InputObject)]
pub struct Create{{name_pascal}}Input {
    pub name: String,
}

/// Input for updating a {{name_snake}}.
#[derive(Debug, InputObject)]
pub struct Update{{name_pascal}}Input {
    pub name: Option<String>,
}

/// {{name_pascal}} query resolver.
#[derive(Default)]
pub struct {{name_pascal}}Query;

#[Object]
impl {{name_pascal}}Query {
    /// Get all {{name_snake}}s.
    async fn {{name_snake}}s(&self, ctx: &Context<'_>) -> Result<Vec<{{name_pascal}}>> {
        // TODO: Implement query logic
        Ok(vec![])
    }

    /// Get a {{name_snake}} by ID.
    async fn {{name_snake}}(&self, ctx: &Context<'_>, id: ID) -> Result<Option<{{name_pascal}}>> {
        // TODO: Implement query logic
        Ok(None)
    }
}

/// {{name_pascal}} mutation resolver.
#[derive(Default)]
pub struct {{name_pascal}}Mutation;

#[Object]
impl {{name_pascal}}Mutation {
    /// Create a new {{name_snake}}.
    async fn create_{{name_snake}}(
        &self,
        ctx: &Context<'_>,
        input: Create{{name_pascal}}Input,
    ) -> Result<{{name_pascal}}> {
        // TODO: Implement create logic
        Ok({{name_pascal}} {
            id: ID::from("1"),
            name: input.name,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    /// Update an existing {{name_snake}}.
    async fn update_{{name_snake}}(
        &self,
        ctx: &Context<'_>,
        id: ID,
        input: Update{{name_pascal}}Input,
    ) -> Result<{{name_pascal}}> {
        // TODO: Implement update logic
        Ok({{name_pascal}} {
            id,
            name: input.name.unwrap_or_default(),
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    /// Delete a {{name_snake}}.
    async fn delete_{{name_snake}}(&self, ctx: &Context<'_>, id: ID) -> Result<bool> {
        // TODO: Implement delete logic
        Ok(true)
    }
}

/// {{name_pascal}} subscription resolver.
#[derive(Default)]
pub struct {{name_pascal}}Subscription;

// Uncomment to add subscriptions
// #[Subscription]
// impl {{name_pascal}}Subscription {
//     /// Subscribe to {{name_snake}} updates.
//     async fn {{name_snake}}_updated(&self) -> impl Stream<Item = {{name_pascal}}> {
//         // TODO: Implement subscription logic
//     }
// }
"#;

const GRAPHQL_RESOLVER_TEST_TEMPLATE: &str = r##"//! Tests for {{name_pascal}} GraphQL resolver.

use super::*;
use async_graphql::{EmptySubscription, Schema};

#[cfg(test)]
mod tests {
    use super::*;

    type TestSchema = Schema<{{name_pascal}}Query, {{name_pascal}}Mutation, EmptySubscription>;

    fn create_schema() -> TestSchema {
        Schema::build(
            {{name_pascal}}Query::default(),
            {{name_pascal}}Mutation::default(),
            EmptySubscription,
        )
        .finish()
    }

    #[tokio::test]
    async fn test_query_{{name_snake}}s() {
        let schema = create_schema();
        let query = r#"{ {{name_snake}}s { id name } }"#;
        let result = schema.execute(query).await;
        assert!(result.errors.is_empty());
    }

    #[tokio::test]
    async fn test_mutation_create() {
        let schema = create_schema();
        let query = r#"mutation { create{{name_pascal}}(input: { name: "Test" }) { id name } }"#;
        let result = schema.execute(query).await;
        assert!(result.errors.is_empty());
    }
}
"##;

// =============================================================================
// JOB/WORKER TEMPLATES
// =============================================================================

const JOB_TEMPLATE: &str = r#"//! {{name_pascal}} background job.

use armature::prelude::*;
use armature_queue::{Job, JobContext, JobResult, Processor};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// {{name_pascal}} job payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct {{name_pascal}}Payload {
    pub id: u64,
    // Add more fields as needed
}

/// {{name_pascal}} job processor.
#[derive(Clone)]
pub struct {{name_pascal}}Job;

impl {{name_pascal}}Job {
    /// Create a new job processor.
    pub fn new() -> Self {
        Self
    }

    /// Queue name for this job.
    pub const QUEUE: &'static str = "{{name_snake}}";
}

impl Default for {{name_pascal}}Job {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Processor for {{name_pascal}}Job {
    type Payload = {{name_pascal}}Payload;

    fn queue(&self) -> &str {
        Self::QUEUE
    }

    async fn process(&self, ctx: JobContext, payload: Self::Payload) -> JobResult {
        tracing::info!(
            "Processing {{name_pascal}}Job: id={}, attempt={}",
            payload.id,
            ctx.attempt
        );

        // TODO: Implement job logic
        // Example: Send email, process data, etc.

        // Simulate work
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        tracing::info!("{{name_pascal}}Job completed: id={}", payload.id);

        JobResult::Success
    }

    fn max_retries(&self) -> u32 {
        3
    }

    fn retry_delay(&self, attempt: u32) -> std::time::Duration {
        // Exponential backoff
        std::time::Duration::from_secs(2u64.pow(attempt))
    }

    async fn on_failure(&self, ctx: JobContext, payload: Self::Payload, error: &str) {
        tracing::error!(
            "{{name_pascal}}Job failed: id={}, error={}",
            payload.id,
            error
        );
        // TODO: Handle failure (send alert, log to dead letter queue, etc.)
    }
}

/// Helper function to enqueue a {{name_snake}} job.
pub async fn enqueue_{{name_snake}}_job(
    queue: &armature_queue::Queue,
    payload: {{name_pascal}}Payload,
) -> Result<String, Error> {
    queue
        .enqueue({{name_pascal}}Job::QUEUE, payload)
        .await
        .map_err(|e| Error::Internal(e.to_string()))
}
"#;

const JOB_TEST_TEMPLATE: &str = r#"//! Tests for {{name_pascal}}Job.

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_job_processing() {
        let job = {{name_pascal}}Job::new();
        let payload = {{name_pascal}}Payload { id: 1 };
        let ctx = JobContext { attempt: 1, job_id: "test".to_string() };

        let result = job.process(ctx, payload).await;
        assert!(matches!(result, JobResult::Success));
    }

    #[test]
    fn test_retry_delay() {
        let job = {{name_pascal}}Job::new();
        assert_eq!(job.retry_delay(1), std::time::Duration::from_secs(2));
        assert_eq!(job.retry_delay(2), std::time::Duration::from_secs(4));
        assert_eq!(job.retry_delay(3), std::time::Duration::from_secs(8));
    }
}
"#;

// =============================================================================
// EVENT HANDLER TEMPLATES
// =============================================================================

const EVENT_HANDLER_TEMPLATE: &str = r#"//! {{name_pascal}} event handler.

use armature::prelude::*;
use armature_events::{Event, EventHandler, EventBus};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// {{name_pascal}} event types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum {{name_pascal}}Event {
    Created { id: u64 },
    Updated { id: u64 },
    Deleted { id: u64 },
}

impl Event for {{name_pascal}}Event {
    fn event_type(&self) -> &str {
        match self {
            Self::Created { .. } => "{{name_snake}}.created",
            Self::Updated { .. } => "{{name_snake}}.updated",
            Self::Deleted { .. } => "{{name_snake}}.deleted",
        }
    }
}

/// Handler for {{name_pascal}} created events.
#[derive(Clone)]
pub struct On{{name_pascal}}Created;

#[async_trait]
impl EventHandler<{{name_pascal}}Event> for On{{name_pascal}}Created {
    async fn handle(&self, event: {{name_pascal}}Event) -> Result<(), Error> {
        if let {{name_pascal}}Event::Created { id } = event {
            tracing::info!("{{name_pascal}} created: id={}", id);
            // TODO: Implement handler logic
            // Example: Send notification, update cache, etc.
        }
        Ok(())
    }
}

/// Handler for {{name_pascal}} updated events.
#[derive(Clone)]
pub struct On{{name_pascal}}Updated;

#[async_trait]
impl EventHandler<{{name_pascal}}Event> for On{{name_pascal}}Updated {
    async fn handle(&self, event: {{name_pascal}}Event) -> Result<(), Error> {
        if let {{name_pascal}}Event::Updated { id } = event {
            tracing::info!("{{name_pascal}} updated: id={}", id);
            // TODO: Implement handler logic
        }
        Ok(())
    }
}

/// Handler for {{name_pascal}} deleted events.
#[derive(Clone)]
pub struct On{{name_pascal}}Deleted;

#[async_trait]
impl EventHandler<{{name_pascal}}Event> for On{{name_pascal}}Deleted {
    async fn handle(&self, event: {{name_pascal}}Event) -> Result<(), Error> {
        if let {{name_pascal}}Event::Deleted { id } = event {
            tracing::info!("{{name_pascal}} deleted: id={}", id);
            // TODO: Implement handler logic
        }
        Ok(())
    }
}

/// Register all {{name_snake}} event handlers.
pub fn register_{{name_snake}}_handlers(bus: &EventBus) {
    bus.subscribe(Arc::new(On{{name_pascal}}Created));
    bus.subscribe(Arc::new(On{{name_pascal}}Updated));
    bus.subscribe(Arc::new(On{{name_pascal}}Deleted));
}
"#;

const EVENT_HANDLER_TEST_TEMPLATE: &str = r#"//! Tests for {{name_pascal}} event handlers.

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_on_created_handler() {
        let handler = On{{name_pascal}}Created;
        let event = {{name_pascal}}Event::Created { id: 1 };
        let result = handler.handle(event).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_on_updated_handler() {
        let handler = On{{name_pascal}}Updated;
        let event = {{name_pascal}}Event::Updated { id: 1 };
        let result = handler.handle(event).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_on_deleted_handler() {
        let handler = On{{name_pascal}}Deleted;
        let event = {{name_pascal}}Event::Deleted { id: 1 };
        let result = handler.handle(event).await;
        assert!(result.is_ok());
    }
}
"#;

// =============================================================================
// INTERCEPTOR TEMPLATES
// =============================================================================

const INTERCEPTOR_TEMPLATE: &str = r#"//! {{name_pascal}} interceptor.

use armature::prelude::*;
use async_trait::async_trait;
use std::time::Instant;

/// {{name_pascal}} interceptor for request/response transformation.
///
/// # Example
///
/// ```rust
/// use armature::prelude::*;
///
/// let interceptor = {{name_pascal}}Interceptor::new();
/// ```
pub struct {{name_pascal}}Interceptor {
    // Add configuration fields here
}

impl {{name_pascal}}Interceptor {
    /// Create a new interceptor instance.
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for {{name_pascal}}Interceptor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Interceptor for {{name_pascal}}Interceptor {
    /// Called before the handler executes.
    async fn before(&self, req: &mut HttpRequest) -> Result<(), Error> {
        // Store request start time for timing
        req.extensions.insert(Instant::now());

        tracing::debug!("{{name_pascal}}Interceptor: Before handler for {}", req.path);

        // TODO: Add pre-processing logic
        // Example: Transform request body, add headers, etc.

        Ok(())
    }

    /// Called after the handler executes.
    async fn after(&self, req: &HttpRequest, res: &mut HttpResponse) -> Result<(), Error> {
        // Calculate request duration
        if let Some(start) = req.extensions.get::<Instant>() {
            let duration = start.elapsed();
            res.headers.insert(
                "X-Response-Time".to_string(),
                format!("{}ms", duration.as_millis()),
            );
        }

        tracing::debug!(
            "{{name_pascal}}Interceptor: After handler for {}, status={}",
            req.path,
            res.status
        );

        // TODO: Add post-processing logic
        // Example: Transform response body, add headers, etc.

        Ok(())
    }
}
"#;

const INTERCEPTOR_TEST_TEMPLATE: &str = r#"//! Tests for {{name_pascal}}Interceptor.

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_interceptor_before() {
        let interceptor = {{name_pascal}}Interceptor::new();
        let mut req = HttpRequest::default();

        let result = interceptor.before(&mut req).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_interceptor_after() {
        let interceptor = {{name_pascal}}Interceptor::new();
        let req = HttpRequest::default();
        let mut res = HttpResponse::ok();

        let result = interceptor.after(&req, &mut res).await;
        assert!(result.is_ok());
    }
}
"#;

// =============================================================================
// PIPE (VALIDATOR) TEMPLATES
// =============================================================================

const PIPE_TEMPLATE: &str = r#"//! {{name_pascal}} validation pipe.

use armature::prelude::*;
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use validator::Validate;

/// {{name_pascal}} validation pipe.
///
/// Validates incoming request data against defined rules.
pub struct {{name_pascal}}Pipe<T> {
    _marker: std::marker::PhantomData<T>,
}

impl<T> {{name_pascal}}Pipe<T> {
    /// Create a new validation pipe.
    pub fn new() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T> Default for {{name_pascal}}Pipe<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl<T> Pipe<T> for {{name_pascal}}Pipe<T>
where
    T: DeserializeOwned + Validate + Send + Sync,
{
    async fn transform(&self, req: &HttpRequest) -> Result<T, Error> {
        // Parse request body
        let value: T = serde_json::from_slice(&req.body)
            .map_err(|e| Error::BadRequest(format!("Invalid JSON: {}", e)))?;

        // Validate
        value.validate().map_err(|e| {
            let errors: Vec<String> = e
                .field_errors()
                .iter()
                .flat_map(|(field, errors)| {
                    errors.iter().map(move |err| {
                        format!(
                            "{}: {}",
                            field,
                            err.message.as_ref().map(|m| m.to_string()).unwrap_or_default()
                        )
                    })
                })
                .collect();
            Error::Validation(errors.join(", "))
        })?;

        Ok(value)
    }
}

/// Custom validation rules for {{name_pascal}}.
pub mod rules {
    use validator::ValidationError;

    /// Validate that a string is not empty or whitespace only.
    pub fn not_blank(value: &str) -> Result<(), ValidationError> {
        if value.trim().is_empty() {
            return Err(ValidationError::new("blank"));
        }
        Ok(())
    }

    /// Validate that a string is a valid slug (lowercase, hyphens, no spaces).
    pub fn is_slug(value: &str) -> Result<(), ValidationError> {
        if !value.chars().all(|c| c.is_ascii_lowercase() || c == '-' || c.is_ascii_digit()) {
            return Err(ValidationError::new("invalid_slug"));
        }
        Ok(())
    }
}
"#;

const PIPE_TEST_TEMPLATE: &str = r##"//! Tests for {{name_pascal}}Pipe.

use super::*;
use serde::Deserialize;
use validator::Validate;

#[derive(Debug, Deserialize, Validate)]
struct TestInput {
    #[validate(length(min = 1))]
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_valid_input() {
        let pipe = {{name_pascal}}Pipe::<TestInput>::new();
        let mut req = HttpRequest::default();
        req.body = r#"{"name": "test"}"#.as_bytes().to_vec();

        let result = pipe.transform(&req).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_invalid_input() {
        let pipe = {{name_pascal}}Pipe::<TestInput>::new();
        let mut req = HttpRequest::default();
        req.body = r#"{"name": ""}"#.as_bytes().to_vec();

        let result = pipe.transform(&req).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_not_blank_rule() {
        assert!(rules::not_blank("test").is_ok());
        assert!(rules::not_blank("").is_err());
        assert!(rules::not_blank("  ").is_err());
    }

    #[test]
    fn test_is_slug_rule() {
        assert!(rules::is_slug("hello-world").is_ok());
        assert!(rules::is_slug("Hello-World").is_err());
        assert!(rules::is_slug("hello world").is_err());
    }
}
"##;

// =============================================================================
// EXCEPTION FILTER TEMPLATES
// =============================================================================

const EXCEPTION_FILTER_TEMPLATE: &str = r#"//! {{name_pascal}} exception filter.

use armature::prelude::*;
use async_trait::async_trait;
use serde::Serialize;

/// Standard error response format.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse {
    pub status_code: u16,
    pub error: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    pub timestamp: String,
    pub path: String,
}

/// {{name_pascal}} exception filter.
///
/// Catches exceptions and transforms them into standardized error responses.
pub struct {{name_pascal}}ExceptionFilter;

impl {{name_pascal}}ExceptionFilter {
    /// Create a new exception filter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for {{name_pascal}}ExceptionFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExceptionFilter for {{name_pascal}}ExceptionFilter {
    async fn catch(&self, error: Error, req: &HttpRequest) -> HttpResponse {
        let (status_code, error_type, message) = match &error {
            Error::NotFound(msg) => (404, "Not Found", msg.clone()),
            Error::BadRequest(msg) => (400, "Bad Request", msg.clone()),
            Error::Unauthorized(msg) => (401, "Unauthorized", msg.clone()),
            Error::Forbidden(msg) => (403, "Forbidden", msg.clone()),
            Error::Validation(msg) => (422, "Validation Error", msg.clone()),
            Error::Conflict(msg) => (409, "Conflict", msg.clone()),
            Error::Internal(msg) => (500, "Internal Server Error", msg.clone()),
            _ => (500, "Internal Server Error", "An unexpected error occurred".to_string()),
        };

        tracing::error!(
            "{{name_pascal}}ExceptionFilter: {} - {} - {}",
            status_code,
            error_type,
            message
        );

        let error_response = ErrorResponse {
            status_code,
            error: error_type.to_string(),
            message,
            details: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
            path: req.path.clone(),
        };

        let body = serde_json::to_vec(&error_response).unwrap_or_default();

        HttpResponse::new(status_code)
            .with_header("Content-Type".to_string(), "application/json".to_string())
            .with_body(body)
    }
}
"#;

const EXCEPTION_FILTER_TEST_TEMPLATE: &str = r#"//! Tests for {{name_pascal}}ExceptionFilter.

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_not_found_error() {
        let filter = {{name_pascal}}ExceptionFilter::new();
        let error = Error::NotFound("Resource not found".to_string());
        let req = HttpRequest::default();

        let response = filter.catch(error, &req).await;
        assert_eq!(response.status, 404);
    }

    #[tokio::test]
    async fn test_bad_request_error() {
        let filter = {{name_pascal}}ExceptionFilter::new();
        let error = Error::BadRequest("Invalid input".to_string());
        let req = HttpRequest::default();

        let response = filter.catch(error, &req).await;
        assert_eq!(response.status, 400);
    }

    #[tokio::test]
    async fn test_internal_error() {
        let filter = {{name_pascal}}ExceptionFilter::new();
        let error = Error::Internal("Something went wrong".to_string());
        let req = HttpRequest::default();

        let response = filter.catch(error, &req).await;
        assert_eq!(response.status, 500);
    }
}
"#;

// =============================================================================
// CONFIG TEMPLATES
// =============================================================================

const CONFIG_TEMPLATE: &str = r#"//! {{name_pascal}} configuration module.

use serde::Deserialize;
use std::env;

/// {{name_pascal}} configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct {{name_pascal}}Config {
    /// Enable debug mode.
    #[serde(default)]
    pub debug: bool,

    /// API key for external services.
    pub api_key: Option<String>,

    /// Base URL for external API.
    pub base_url: Option<String>,

    /// Timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,

    /// Maximum retry attempts.
    #[serde(default = "default_retries")]
    pub max_retries: u32,
}

fn default_timeout() -> u64 {
    30
}

fn default_retries() -> u32 {
    3
}

impl Default for {{name_pascal}}Config {
    fn default() -> Self {
        Self {
            debug: false,
            api_key: None,
            base_url: None,
            timeout_secs: default_timeout(),
            max_retries: default_retries(),
        }
    }
}

impl {{name_pascal}}Config {
    /// Load configuration from environment variables.
    ///
    /// Environment variables:
    /// - `{{name_upper}}_DEBUG`: Enable debug mode (true/false)
    /// - `{{name_upper}}_API_KEY`: API key
    /// - `{{name_upper}}_BASE_URL`: Base URL
    /// - `{{name_upper}}_TIMEOUT`: Timeout in seconds
    /// - `{{name_upper}}_MAX_RETRIES`: Maximum retry attempts
    pub fn from_env() -> Self {
        Self {
            debug: env::var("{{name_upper}}_DEBUG")
                .map(|v| v.parse().unwrap_or(false))
                .unwrap_or(false),
            api_key: env::var("{{name_upper}}_API_KEY").ok(),
            base_url: env::var("{{name_upper}}_BASE_URL").ok(),
            timeout_secs: env::var("{{name_upper}}_TIMEOUT")
                .map(|v| v.parse().unwrap_or(default_timeout()))
                .unwrap_or(default_timeout()),
            max_retries: env::var("{{name_upper}}_MAX_RETRIES")
                .map(|v| v.parse().unwrap_or(default_retries()))
                .unwrap_or(default_retries()),
        }
    }

    /// Create a builder for the configuration.
    pub fn builder() -> {{name_pascal}}ConfigBuilder {
        {{name_pascal}}ConfigBuilder::default()
    }
}

/// Builder for {{name_pascal}}Config.
#[derive(Default)]
pub struct {{name_pascal}}ConfigBuilder {
    config: {{name_pascal}}Config,
}

impl {{name_pascal}}ConfigBuilder {
    /// Enable debug mode.
    pub fn debug(mut self, debug: bool) -> Self {
        self.config.debug = debug;
        self
    }

    /// Set the API key.
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.config.api_key = Some(key.into());
        self
    }

    /// Set the base URL.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.config.base_url = Some(url.into());
        self
    }

    /// Set the timeout in seconds.
    pub fn timeout(mut self, secs: u64) -> Self {
        self.config.timeout_secs = secs;
        self
    }

    /// Set maximum retries.
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.config.max_retries = retries;
        self
    }

    /// Build the configuration.
    pub fn build(self) -> {{name_pascal}}Config {
        self.config
    }
}
"#;

// =============================================================================
// ENTITY TEMPLATES
// =============================================================================

const ENTITY_TEMPLATE: &str = r#"//! {{name_pascal}} database entity.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[cfg(feature = "diesel")]
use diesel::prelude::*;

#[cfg(feature = "sea-orm")]
use sea_orm::entity::prelude::*;

/// {{name_pascal}} entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "diesel", derive(Queryable, Insertable, AsChangeset))]
#[cfg_attr(feature = "diesel", diesel(table_name = {{name_snake}}s))]
pub struct {{name_pascal}} {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// New {{name_pascal}} for insertion.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "diesel", derive(Insertable))]
#[cfg_attr(feature = "diesel", diesel(table_name = {{name_snake}}s))]
pub struct New{{name_pascal}} {
    pub name: String,
    pub description: Option<String>,
    #[serde(default = "default_active")]
    pub active: bool,
}

fn default_active() -> bool {
    true
}

/// {{name_pascal}} update changeset.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(feature = "diesel", derive(AsChangeset))]
#[cfg_attr(feature = "diesel", diesel(table_name = {{name_snake}}s))]
pub struct Update{{name_pascal}} {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub active: Option<bool>,
}

#[cfg(feature = "diesel")]
diesel::table! {
    {{name_snake}}s (id) {
        id -> BigInt,
        name -> Varchar,
        description -> Nullable<Text>,
        active -> Bool,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

// SeaORM entity definition
#[cfg(feature = "sea-orm")]
mod sea_orm_entity {
    use super::*;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
    #[sea_orm(table_name = "{{name_snake}}s")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub name: String,
        pub description: Option<String>,
        pub active: bool,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

#[cfg(feature = "sea-orm")]
pub use sea_orm_entity::*;
"#;

// =============================================================================
// PRAX ORM TEMPLATES
// =============================================================================

const ENTITY_PRAX_TEMPLATE: &str = r#"//! {{name_pascal}} Prax ORM model.
//!
//! This file is auto-generated from the Prax schema.
//! Manual changes may be overwritten when regenerating.

use prax::prelude::*;
use serde::{Deserialize, Serialize};

/// {{name_pascal}} model generated from Prax schema.
#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[prax(table = "{{name_snake}}s")]
pub struct {{name_pascal}} {
    #[prax(id, auto)]
    pub id: i64,

    #[prax(unique)]
    pub name: String,

    pub description: Option<String>,

    #[prax(default = true)]
    pub active: bool,

    #[prax(default = "now()")]
    pub created_at: DateTime<Utc>,

    #[prax(default = "now()", on_update = "now()")]
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a new {{name_pascal}}.
#[derive(Debug, Clone, Deserialize)]
pub struct Create{{name_pascal}}Input {
    pub name: String,
    pub description: Option<String>,
    #[serde(default = "default_active")]
    pub active: bool,
}

fn default_active() -> bool {
    true
}

/// Input for updating an existing {{name_pascal}}.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Update{{name_pascal}}Input {
    pub name: Option<String>,
    pub description: Option<String>,
    pub active: Option<bool>,
}

/// Filter options for querying {{name_pascal}}s.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct {{name_pascal}}Filter {
    pub name_contains: Option<String>,
    pub active: Option<bool>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
}

/// Sort options for {{name_pascal}} queries.
#[derive(Debug, Clone, Default)]
pub struct {{name_pascal}}OrderBy {
    pub field: {{name_pascal}}SortField,
    pub direction: SortDirection,
}

#[derive(Debug, Clone, Default)]
pub enum {{name_pascal}}SortField {
    #[default]
    CreatedAt,
    UpdatedAt,
    Name,
}

#[derive(Debug, Clone, Default)]
pub enum SortDirection {
    #[default]
    Desc,
    Asc,
}
"#;

const PRAX_SCHEMA_TEMPLATE: &str = r#"// Prax Schema Definition
// This file defines your database models using Prax's schema language.
// Run `prax generate` to generate Rust code from this schema.

datasource db {
  provider = "postgresql"
  url      = env("DATABASE_URL")
}

generator client {
  provider = "prax-codegen"
  output   = "./src/generated"
}

model {{name_pascal}} {
  id          Int      @id @default(autoincrement())
  name        String   @unique
  description String?
  active      Boolean  @default(true)
  createdAt   DateTime @default(now()) @map("created_at")
  updatedAt   DateTime @updatedAt @map("updated_at")

  @@map("{{name_snake}}s")
}
"#;

const PRAX_REPOSITORY_TEMPLATE: &str = r#"//! {{name_pascal}} repository using Prax ORM.

use armature::prelude::*;
use prax_armature::PraxClient;
use std::sync::Arc;

use crate::entities::{{name_snake}}::{
    {{name_pascal}}, Create{{name_pascal}}Input, Update{{name_pascal}}Input, {{name_pascal}}Filter,
};

/// Repository for {{name_pascal}} operations using Prax ORM.
#[derive(Clone)]
#[injectable]
pub struct {{name_pascal}}Repository {
    db: Arc<PraxClient>,
}

impl {{name_pascal}}Repository {
    /// Create a new repository instance.
    pub fn new(db: Arc<PraxClient>) -> Self {
        Self { db }
    }

    /// Find all {{name_snake}}s with optional filtering.
    pub async fn find_many(&self, filter: Option<{{name_pascal}}Filter>) -> Result<Vec<{{name_pascal}}>, Error> {
        let mut query = self.db.{{name_snake}}().find_many();

        if let Some(f) = filter {
            if let Some(name) = f.name_contains {
                query = query.filter({{name_snake}}::name.contains(name));
            }
            if let Some(active) = f.active {
                query = query.filter({{name_snake}}::active.equals(active));
            }
            if let Some(after) = f.created_after {
                query = query.filter({{name_snake}}::created_at.gte(after));
            }
            if let Some(before) = f.created_before {
                query = query.filter({{name_snake}}::created_at.lte(before));
            }
        }

        query
            .exec()
            .await
            .map_err(|e| Error::Database(e.to_string()))
    }

    /// Find a single {{name_snake}} by ID.
    pub async fn find_by_id(&self, id: i64) -> Result<Option<{{name_pascal}}>, Error> {
        self.db
            .{{name_snake}}()
            .find_unique({{name_snake}}::id.equals(id))
            .exec()
            .await
            .map_err(|e| Error::Database(e.to_string()))
    }

    /// Find a single {{name_snake}} by name.
    pub async fn find_by_name(&self, name: &str) -> Result<Option<{{name_pascal}}>, Error> {
        self.db
            .{{name_snake}}()
            .find_unique({{name_snake}}::name.equals(name.to_string()))
            .exec()
            .await
            .map_err(|e| Error::Database(e.to_string()))
    }

    /// Find the first {{name_snake}} matching the filter.
    pub async fn find_first(&self, filter: {{name_pascal}}Filter) -> Result<Option<{{name_pascal}}>, Error> {
        let mut query = self.db.{{name_snake}}().find_first();

        if let Some(name) = filter.name_contains {
            query = query.filter({{name_snake}}::name.contains(name));
        }
        if let Some(active) = filter.active {
            query = query.filter({{name_snake}}::active.equals(active));
        }

        query
            .exec()
            .await
            .map_err(|e| Error::Database(e.to_string()))
    }

    /// Create a new {{name_snake}}.
    pub async fn create(&self, input: Create{{name_pascal}}Input) -> Result<{{name_pascal}}, Error> {
        self.db
            .{{name_snake}}()
            .create(
                {{name_snake}}::name.set(input.name),
                vec![
                    {{name_snake}}::description.set(input.description),
                    {{name_snake}}::active.set(input.active),
                ],
            )
            .exec()
            .await
            .map_err(|e| Error::Database(e.to_string()))
    }

    /// Create multiple {{name_snake}}s.
    pub async fn create_many(&self, inputs: Vec<Create{{name_pascal}}Input>) -> Result<i64, Error> {
        let data: Vec<_> = inputs
            .into_iter()
            .map(|input| {
                (
                    {{name_snake}}::name.set(input.name),
                    vec![
                        {{name_snake}}::description.set(input.description),
                        {{name_snake}}::active.set(input.active),
                    ],
                )
            })
            .collect();

        self.db
            .{{name_snake}}()
            .create_many(data)
            .exec()
            .await
            .map_err(|e| Error::Database(e.to_string()))
    }

    /// Update an existing {{name_snake}}.
    pub async fn update(&self, id: i64, input: Update{{name_pascal}}Input) -> Result<{{name_pascal}}, Error> {
        let mut updates = vec![];

        if let Some(name) = input.name {
            updates.push({{name_snake}}::name.set(name));
        }
        if let Some(description) = input.description {
            updates.push({{name_snake}}::description.set(Some(description)));
        }
        if let Some(active) = input.active {
            updates.push({{name_snake}}::active.set(active));
        }

        self.db
            .{{name_snake}}()
            .update({{name_snake}}::id.equals(id), updates)
            .exec()
            .await
            .map_err(|e| Error::Database(e.to_string()))
    }

    /// Update many {{name_snake}}s matching a filter.
    pub async fn update_many(
        &self,
        filter: {{name_pascal}}Filter,
        input: Update{{name_pascal}}Input,
    ) -> Result<i64, Error> {
        let mut filters = vec![];
        let mut updates = vec![];

        if let Some(active) = filter.active {
            filters.push({{name_snake}}::active.equals(active));
        }

        if let Some(name) = input.name {
            updates.push({{name_snake}}::name.set(name));
        }
        if let Some(active) = input.active {
            updates.push({{name_snake}}::active.set(active));
        }

        self.db
            .{{name_snake}}()
            .update_many(filters, updates)
            .exec()
            .await
            .map_err(|e| Error::Database(e.to_string()))
    }

    /// Delete a {{name_snake}} by ID.
    pub async fn delete(&self, id: i64) -> Result<{{name_pascal}}, Error> {
        self.db
            .{{name_snake}}()
            .delete({{name_snake}}::id.equals(id))
            .exec()
            .await
            .map_err(|e| Error::Database(e.to_string()))
    }

    /// Delete many {{name_snake}}s matching a filter.
    pub async fn delete_many(&self, filter: {{name_pascal}}Filter) -> Result<i64, Error> {
        let mut filters = vec![];

        if let Some(active) = filter.active {
            filters.push({{name_snake}}::active.equals(active));
        }

        self.db
            .{{name_snake}}()
            .delete_many(filters)
            .exec()
            .await
            .map_err(|e| Error::Database(e.to_string()))
    }

    /// Count {{name_snake}}s with optional filter.
    pub async fn count(&self, filter: Option<{{name_pascal}}Filter>) -> Result<i64, Error> {
        let mut query = self.db.{{name_snake}}().count();

        if let Some(f) = filter {
            if let Some(active) = f.active {
                query = query.filter({{name_snake}}::active.equals(active));
            }
        }

        query
            .exec()
            .await
            .map_err(|e| Error::Database(e.to_string()))
    }

    /// Check if a {{name_snake}} exists by ID.
    pub async fn exists(&self, id: i64) -> Result<bool, Error> {
        let count = self
            .db
            .{{name_snake}}()
            .count()
            .filter({{name_snake}}::id.equals(id))
            .exec()
            .await
            .map_err(|e| Error::Database(e.to_string()))?;

        Ok(count > 0)
    }

    /// Upsert a {{name_snake}} (create if not exists, update if exists).
    pub async fn upsert(
        &self,
        name: String,
        create: Create{{name_pascal}}Input,
        update: Update{{name_pascal}}Input,
    ) -> Result<{{name_pascal}}, Error> {
        let mut updates = vec![];

        if let Some(new_name) = update.name {
            updates.push({{name_snake}}::name.set(new_name));
        }
        if let Some(description) = update.description {
            updates.push({{name_snake}}::description.set(Some(description)));
        }
        if let Some(active) = update.active {
            updates.push({{name_snake}}::active.set(active));
        }

        self.db
            .{{name_snake}}()
            .upsert(
                {{name_snake}}::name.equals(name),
                (
                    {{name_snake}}::name.set(create.name),
                    vec![
                        {{name_snake}}::description.set(create.description),
                        {{name_snake}}::active.set(create.active),
                    ],
                ),
                updates,
            )
            .exec()
            .await
            .map_err(|e| Error::Database(e.to_string()))
    }
}
"#;

const PRAX_REPOSITORY_TEST_TEMPLATE: &str = r#"//! Tests for {{name_pascal}}Repository.

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require a running database and Prax client.
    // Use test containers or a test database for integration testing.

    #[tokio::test]
    #[ignore] // Requires database
    async fn test_create_and_find() {
        // Setup: Create a test Prax client
        let db = setup_test_db().await;
        let repo = {{name_pascal}}Repository::new(db);

        // Create
        let input = Create{{name_pascal}}Input {
            name: "Test {{name_pascal}}".to_string(),
            description: Some("Test description".to_string()),
            active: true,
        };

        let created = repo.create(input).await.expect("Failed to create");
        assert!(created.id > 0);
        assert_eq!(created.name, "Test {{name_pascal}}");

        // Find by ID
        let found = repo.find_by_id(created.id).await.expect("Failed to find");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "Test {{name_pascal}}");

        // Cleanup
        repo.delete(created.id).await.expect("Failed to delete");
    }

    #[tokio::test]
    #[ignore] // Requires database
    async fn test_update() {
        let db = setup_test_db().await;
        let repo = {{name_pascal}}Repository::new(db);

        // Create
        let created = repo
            .create(Create{{name_pascal}}Input {
                name: "Original".to_string(),
                description: None,
                active: true,
            })
            .await
            .expect("Failed to create");

        // Update
        let updated = repo
            .update(
                created.id,
                Update{{name_pascal}}Input {
                    name: Some("Updated".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("Failed to update");

        assert_eq!(updated.name, "Updated");

        // Cleanup
        repo.delete(created.id).await.expect("Failed to delete");
    }

    #[tokio::test]
    #[ignore] // Requires database
    async fn test_find_many_with_filter() {
        let db = setup_test_db().await;
        let repo = {{name_pascal}}Repository::new(db);

        // Create test data
        let _ = repo
            .create(Create{{name_pascal}}Input {
                name: "Active Item".to_string(),
                description: None,
                active: true,
            })
            .await;

        let _ = repo
            .create(Create{{name_pascal}}Input {
                name: "Inactive Item".to_string(),
                description: None,
                active: false,
            })
            .await;

        // Find active only
        let filter = {{name_pascal}}Filter {
            active: Some(true),
            ..Default::default()
        };

        let results = repo.find_many(Some(filter)).await.expect("Failed to find");
        assert!(results.iter().all(|r| r.active));
    }

    async fn setup_test_db() -> Arc<PraxClient> {
        let url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://localhost/test".to_string());

        let client = PraxClientBuilder::new()
            .url(&url)
            .build()
            .await
            .expect("Failed to connect to test database");

        Arc::new(client)
    }
}
"#;

const PRAX_MODULE_TEMPLATE: &str = r#"//! {{name_pascal}} module with Prax ORM integration.

use armature::prelude::*;
use prax_armature::{PraxClient, PraxClientBuilder};
use std::sync::Arc;

mod entities;
mod repositories;
mod services;
mod controllers;

pub use entities::{{name_snake}}::*;
pub use repositories::{{name_snake}}_repository::{{name_pascal}}Repository;
pub use services::{{name_snake}}_service::{{name_pascal}}Service;
pub use controllers::{{name_snake}}_controller::{{name_pascal}}Controller;

/// {{name_pascal}} module with Prax ORM database integration.
#[module(
    controllers: [{{name_pascal}}Controller],
    providers: [{{name_pascal}}Service, {{name_pascal}}Repository, PraxClientProvider]
)]
#[derive(Default)]
pub struct {{name_pascal}}Module;

/// Provider for the Prax ORM client.
#[module_impl]
impl {{name_pascal}}Module {
    /// Create a Prax client singleton.
    #[provider(singleton)]
    async fn prax_client() -> Arc<PraxClient> {
        let database_url = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL must be set");

        let client = PraxClientBuilder::new()
            .url(&database_url)
            .max_connections(10)
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .await
            .expect("Failed to connect to database");

        Arc::new(client)
    }
}

/// Database configuration for the {{name_snake}} module.
pub struct {{name_pascal}}DbConfig {
    pub url: String,
    pub max_connections: u32,
    pub connect_timeout_secs: u64,
}

impl Default for {{name_pascal}}DbConfig {
    fn default() -> Self {
        Self {
            url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgresql://localhost/{{name_snake}}".to_string()),
            max_connections: 10,
            connect_timeout_secs: 10,
        }
    }
}
"#;

// =============================================================================
// RHAI SCRIPT TEMPLATES
// =============================================================================

const RHAI_HANDLER_TEMPLATE: &str = r#"// {{name_pascal}} handler script
// This script handles HTTP requests for {{name_snake}} endpoints

let method = request.method;
let path = request.path;

log_info(`{{name_pascal}}Handler: ${method} ${path}`);

// Route based on method
if method == "GET" {
    // Check for ID parameter
    let id = request.param("id");

    if id != () {
        // Get single item
        response.json(#{
            id: id,
            name: "Example {{name_pascal}}",
            message: "Retrieved {{name_snake}}"
        })
    } else {
        // List all items
        response.json(#{
            items: [],
            total: 0,
            page: 1,
            message: "List all {{name_snake}}s"
        })
    }
} else if method == "POST" {
    // Create new item
    let body = request.json();

    if body.name == () {
        bad_request().json(#{
            error: "Validation Error",
            message: "Name is required"
        })
    } else {
        created().json(#{
            id: 1,
            name: body.name,
            message: "{{name_pascal}} created"
        })
    }
} else if method == "PUT" {
    // Update existing item
    let id = request.param("id");

    if id == () {
        bad_request().json(#{
            error: "Bad Request",
            message: "ID is required"
        })
    } else {
        let body = request.json();
        response.json(#{
            id: id,
            name: body.name,
            message: "{{name_pascal}} updated"
        })
    }
} else if method == "DELETE" {
    // Delete item
    let id = request.param("id");

    if id == () {
        bad_request().json(#{
            error: "Bad Request",
            message: "ID is required"
        })
    } else {
        no_content()
    }
} else {
    method_not_allowed()
}
"#;

// =============================================================================
// HEALTH CONTROLLER TEMPLATE
// =============================================================================

const HEALTH_CONTROLLER_TEMPLATE: &str = r#"//! Health check controller.

use armature::prelude::*;
use serde::Serialize;
use std::time::Instant;

/// Health check response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub uptime_secs: u64,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checks: Option<Vec<HealthCheck>>,
}

/// Individual health check result.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheck {
    pub name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub response_time_ms: u64,
}

/// Health check controller.
#[controller("/health")]
#[derive(Default)]
pub struct HealthController {
    start_time: Option<Instant>,
}

impl HealthController {
    /// Create a new health controller.
    pub fn new() -> Self {
        Self {
            start_time: Some(Instant::now()),
        }
    }
}

impl HealthController {
    /// Basic health check (liveness probe).
    ///
    /// GET /health
    #[get("/")]
    pub async fn health(&self, _req: HttpRequest) -> Result<HttpResponse, Error> {
        let response = HealthResponse {
            status: "ok".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_secs: self.start_time.map(|t| t.elapsed().as_secs()).unwrap_or(0),
            timestamp: chrono::Utc::now().to_rfc3339(),
            checks: None,
        };

        HttpResponse::ok().with_json(&response)
    }

    /// Detailed health check (readiness probe).
    ///
    /// GET /health/ready
    #[get("/ready")]
    pub async fn ready(&self, _req: HttpRequest) -> Result<HttpResponse, Error> {
        let mut checks = Vec::new();
        let mut all_healthy = true;

        // Database check
        let db_check = self.check_database().await;
        if db_check.status != "ok" {
            all_healthy = false;
        }
        checks.push(db_check);

        // Redis check
        let redis_check = self.check_redis().await;
        if redis_check.status != "ok" {
            all_healthy = false;
        }
        checks.push(redis_check);

        let response = HealthResponse {
            status: if all_healthy { "ok" } else { "degraded" }.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_secs: self.start_time.map(|t| t.elapsed().as_secs()).unwrap_or(0),
            timestamp: chrono::Utc::now().to_rfc3339(),
            checks: Some(checks),
        };

        let status = if all_healthy { 200 } else { 503 };

        HttpResponse::new(status).with_json(&response)
    }

    async fn check_database(&self) -> HealthCheck {
        let start = Instant::now();
        // TODO: Implement actual database health check
        HealthCheck {
            name: "database".to_string(),
            status: "ok".to_string(),
            message: None,
            response_time_ms: start.elapsed().as_millis() as u64,
        }
    }

    async fn check_redis(&self) -> HealthCheck {
        let start = Instant::now();
        // TODO: Implement actual Redis health check
        HealthCheck {
            name: "redis".to_string(),
            status: "ok".to_string(),
            message: None,
            response_time_ms: start.elapsed().as_millis() as u64,
        }
    }
}
"#;

// =============================================================================
// DEVOPS TEMPLATES
// =============================================================================

const DOCKERFILE_TEMPLATE: &str = r#"# Build stage
FROM rust:1.75-slim-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Create dummy source for dependency caching
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build dependencies only
RUN cargo build --release && rm -rf src target/release/{{name_kebab}}*

# Copy actual source
COPY src ./src

# Build the application
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -r -s /bin/false appuser

# Copy binary from builder
COPY --from=builder /app/target/release/{{name_kebab}} /app/{{name_kebab}}

# Set ownership
RUN chown -R appuser:appuser /app

USER appuser

# Expose port
EXPOSE 3000

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:3000/health || exit 1

# Run the application
CMD ["/app/{{name_kebab}}"]
"#;

const DOCKER_COMPOSE_TEMPLATE: &str = r#"version: '3.8'

services:
  {{name_kebab}}:
    build:
      context: .
      dockerfile: Dockerfile
    ports:
      - "3000:3000"
    environment:
      - RUST_LOG=info
      - DATABASE_URL=postgres://postgres:postgres@db:5432/{{name_snake}}
      - REDIS_URL=redis://redis:6379
    depends_on:
      db:
        condition: service_healthy
      redis:
        condition: service_healthy
    networks:
      - {{name_snake}}-network
    restart: unless-stopped

  db:
    image: postgres:16-alpine
    environment:
      - POSTGRES_USER=postgres
      - POSTGRES_PASSWORD=postgres
      - POSTGRES_DB={{name_snake}}
    volumes:
      - postgres-data:/var/lib/postgresql/data
    ports:
      - "5432:5432"
    networks:
      - {{name_snake}}-network
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 10s
      timeout: 5s
      retries: 5

  redis:
    image: redis:7-alpine
    ports:
      - "6379:6379"
    volumes:
      - redis-data:/data
    networks:
      - {{name_snake}}-network
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 10s
      timeout: 5s
      retries: 5

networks:
  {{name_snake}}-network:
    driver: bridge

volumes:
  postgres-data:
  redis-data:
"#;

const GITHUB_ACTIONS_TEMPLATE: &str = r#"name: CI

on:
  push:
    branches: [main, develop]
  pull_request:
    branches: [main]

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  test:
    name: Test
    runs-on: ubuntu-latest
    services:
      postgres:
        image: postgres:16
        env:
          POSTGRES_USER: postgres
          POSTGRES_PASSWORD: postgres
          POSTGRES_DB: {{name_snake}}_test
        ports:
          - 5432:5432
        options: >-
          --health-cmd pg_isready
          --health-interval 10s
          --health-timeout 5s
          --health-retries 5

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt

      - name: Cache cargo
        uses: Swatinem/rust-cache@v2

      - name: Check formatting
        run: cargo fmt --all -- --check

      - name: Clippy
        run: cargo clippy --all-targets --all-features -- -D warnings

      - name: Run tests
        run: cargo test --all-features
        env:
          DATABASE_URL: postgres://postgres:postgres@localhost:5432/{{name_snake}}_test

  build:
    name: Build
    runs-on: ubuntu-latest
    needs: test

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Cache cargo
        uses: Swatinem/rust-cache@v2

      - name: Build release
        run: cargo build --release

      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: {{name_kebab}}
          path: target/release/{{name_kebab}}

  docker:
    name: Docker
    runs-on: ubuntu-latest
    needs: test
    if: github.ref == 'refs/heads/main'

    steps:
      - uses: actions/checkout@v4

      - name: Set up Docker Buildx
        uses: docker/setup-buildx-action@v3

      - name: Login to Container Registry
        uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{curly_open}}{{curly_open}} github.actor {{curly_close}}{{curly_close}}
          password: ${{curly_open}}{{curly_open}} secrets.GITHUB_TOKEN {{curly_close}}{{curly_close}}

      - name: Build and push
        uses: docker/build-push-action@v5
        with:
          context: .
          push: true
          tags: ghcr.io/${{curly_open}}{{curly_open}} github.repository {{curly_close}}{{curly_close}}:latest
          cache-from: type=gha
          cache-to: type=gha,mode=max
"#;

// =============================================================================
// TEST TEMPLATES
// =============================================================================

const INTEGRATION_TEST_TEMPLATE: &str = r#"//! Integration tests for {{name_pascal}}.

use armature::prelude::*;
use armature_testing::TestClient;

/// Test client wrapper for {{name_pascal}} API.
pub struct {{name_pascal}}TestClient {
    client: TestClient,
}

impl {{name_pascal}}TestClient {
    /// Create a new test client.
    pub async fn new() -> Self {
        let client = TestClient::new().await;
        Self { client }
    }

    /// GET /{{base_path}}
    pub async fn list(&self) -> Result<Vec<serde_json::Value>, Error> {
        self.client.get("/{{base_path}}").send().await?.json().await
    }

    /// GET /{{base_path}}/:id
    pub async fn get(&self, id: u64) -> Result<serde_json::Value, Error> {
        self.client
            .get(&format!("/{{base_path}}/{}", id))
            .send()
            .await?
            .json()
            .await
    }

    /// POST /{{base_path}}
    pub async fn create(&self, body: serde_json::Value) -> Result<serde_json::Value, Error> {
        self.client
            .post("/{{base_path}}")
            .json(&body)
            .send()
            .await?
            .json()
            .await
    }

    /// PUT /{{base_path}}/:id
    pub async fn update(&self, id: u64, body: serde_json::Value) -> Result<serde_json::Value, Error> {
        self.client
            .put(&format!("/{{base_path}}/{}", id))
            .json(&body)
            .send()
            .await?
            .json()
            .await
    }

    /// DELETE /{{base_path}}/:id
    pub async fn delete(&self, id: u64) -> Result<(), Error> {
        self.client
            .delete(&format!("/{{base_path}}/{}", id))
            .send()
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_list_{{name_snake}}s() {
        let client = {{name_pascal}}TestClient::new().await;
        let result = client.list().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_create_and_get_{{name_snake}}() {
        let client = {{name_pascal}}TestClient::new().await;

        // Create
        let created = client
            .create(serde_json::json!({ "name": "Test" }))
            .await
            .expect("Failed to create");

        let id = created["id"].as_u64().expect("No ID in response");

        // Get
        let fetched = client.get(id).await.expect("Failed to get");
        assert_eq!(fetched["name"], "Test");
    }

    #[tokio::test]
    async fn test_update_{{name_snake}}() {
        let client = {{name_pascal}}TestClient::new().await;

        // Create
        let created = client
            .create(serde_json::json!({ "name": "Original" }))
            .await
            .expect("Failed to create");

        let id = created["id"].as_u64().expect("No ID in response");

        // Update
        let updated = client
            .update(id, serde_json::json!({ "name": "Updated" }))
            .await
            .expect("Failed to update");

        assert_eq!(updated["name"], "Updated");
    }

    #[tokio::test]
    async fn test_delete_{{name_snake}}() {
        let client = {{name_pascal}}TestClient::new().await;

        // Create
        let created = client
            .create(serde_json::json!({ "name": "ToDelete" }))
            .await
            .expect("Failed to create");

        let id = created["id"].as_u64().expect("No ID in response");

        // Delete
        client.delete(id).await.expect("Failed to delete");

        // Verify deleted
        let result = client.get(id).await;
        assert!(result.is_err());
    }
}
"#;

// =============================================================================
// SCHEDULER TEMPLATES
// =============================================================================

const SCHEDULER_TEMPLATE: &str = r#"//! {{name_pascal}} scheduled task.

use armature::prelude::*;
use armature_cron::{Cron, CronSchedule, Task};
use async_trait::async_trait;

/// {{name_pascal}} scheduled task.
///
/// Runs according to the configured cron schedule.
pub struct {{name_pascal}}Task {
    schedule: CronSchedule,
}

impl {{name_pascal}}Task {
    /// Create a new scheduled task.
    ///
    /// # Arguments
    ///
    /// * `cron_expr` - Cron expression (e.g., "0 */5 * * * *" for every 5 minutes)
    pub fn new(cron_expr: &str) -> Result<Self, Error> {
        let schedule = CronSchedule::parse(cron_expr)
            .map_err(|e| Error::Config(format!("Invalid cron expression: {}", e)))?;

        Ok(Self { schedule })
    }

    /// Create a task that runs every minute.
    pub fn every_minute() -> Result<Self, Error> {
        Self::new("0 * * * * *")
    }

    /// Create a task that runs every 5 minutes.
    pub fn every_5_minutes() -> Result<Self, Error> {
        Self::new("0 */5 * * * *")
    }

    /// Create a task that runs every hour.
    pub fn hourly() -> Result<Self, Error> {
        Self::new("0 0 * * * *")
    }

    /// Create a task that runs daily at midnight.
    pub fn daily() -> Result<Self, Error> {
        Self::new("0 0 0 * * *")
    }

    /// Create a task that runs weekly on Sunday at midnight.
    pub fn weekly() -> Result<Self, Error> {
        Self::new("0 0 0 * * 0")
    }
}

#[async_trait]
impl Task for {{name_pascal}}Task {
    fn name(&self) -> &str {
        "{{name_snake}}_task"
    }

    fn schedule(&self) -> &CronSchedule {
        &self.schedule
    }

    async fn run(&self) -> Result<(), Error> {
        tracing::info!("{{name_pascal}}Task: Starting execution");
        let start = std::time::Instant::now();

        // TODO: Implement your scheduled task logic here
        // Examples:
        // - Clean up old records
        // - Send scheduled notifications
        // - Generate reports
        // - Sync data with external services

        let duration = start.elapsed();
        tracing::info!(
            "{{name_pascal}}Task: Completed in {:?}",
            duration
        );

        Ok(())
    }

    async fn on_error(&self, error: Error) {
        tracing::error!("{{name_pascal}}Task failed: {}", error);
        // TODO: Handle error (send alert, retry logic, etc.)
    }
}
"#;

const SCHEDULER_TEST_TEMPLATE: &str = r#"//! Tests for {{name_pascal}}Task.

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cron_parsing() {
        let task = {{name_pascal}}Task::every_minute();
        assert!(task.is_ok());

        let task = {{name_pascal}}Task::every_5_minutes();
        assert!(task.is_ok());

        let task = {{name_pascal}}Task::hourly();
        assert!(task.is_ok());

        let task = {{name_pascal}}Task::daily();
        assert!(task.is_ok());

        let task = {{name_pascal}}Task::weekly();
        assert!(task.is_ok());
    }

    #[tokio::test]
    async fn test_task_execution() {
        let task = {{name_pascal}}Task::every_minute().expect("Failed to create task");
        let result = task.run().await;
        assert!(result.is_ok());
    }
}
"#;

// =============================================================================
// CACHE SERVICE TEMPLATES
// =============================================================================

const CACHE_SERVICE_TEMPLATE: &str = r#"//! {{name_pascal}} cache service.

use armature::prelude::*;
use armature_cache::{Cache, CacheConfig, CacheKey};
use serde::{de::DeserializeOwned, Serialize};
use std::time::Duration;

/// {{name_pascal}} cache service.
///
/// Provides caching functionality for {{name_snake}} data.
#[derive(Clone)]
#[injectable]
pub struct {{name_pascal}}CacheService {
    cache: Cache,
    prefix: String,
    default_ttl: Duration,
}

impl {{name_pascal}}CacheService {
    /// Create a new cache service.
    pub async fn new(config: CacheConfig) -> Result<Self, Error> {
        let cache = Cache::new(config).await?;

        Ok(Self {
            cache,
            prefix: "{{name_snake}}".to_string(),
            default_ttl: Duration::from_secs(300), // 5 minutes
        })
    }

    /// Build a cache key with the service prefix.
    fn key(&self, id: &str) -> String {
        format!("{}:{}", self.prefix, id)
    }

    /// Get a value from cache.
    pub async fn get<T: DeserializeOwned>(&self, id: &str) -> Result<Option<T>, Error> {
        self.cache.get(&self.key(id)).await
    }

    /// Set a value in cache with default TTL.
    pub async fn set<T: Serialize + Send + Sync>(
        &self,
        id: &str,
        value: &T,
    ) -> Result<(), Error> {
        self.cache.set(&self.key(id), value, self.default_ttl).await
    }

    /// Set a value in cache with custom TTL.
    pub async fn set_with_ttl<T: Serialize + Send + Sync>(
        &self,
        id: &str,
        value: &T,
        ttl: Duration,
    ) -> Result<(), Error> {
        self.cache.set(&self.key(id), value, ttl).await
    }

    /// Delete a value from cache.
    pub async fn delete(&self, id: &str) -> Result<(), Error> {
        self.cache.delete(&self.key(id)).await
    }

    /// Check if a key exists in cache.
    pub async fn exists(&self, id: &str) -> Result<bool, Error> {
        self.cache.exists(&self.key(id)).await
    }

    /// Get or compute a value.
    ///
    /// If the value is in cache, return it. Otherwise, compute it using
    /// the provided function and cache the result.
    pub async fn get_or_set<T, F, Fut>(
        &self,
        id: &str,
        compute: F,
    ) -> Result<T, Error>
    where
        T: Serialize + DeserializeOwned + Send + Sync,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, Error>>,
    {
        if let Some(value) = self.get(id).await? {
            return Ok(value);
        }

        let value = compute().await?;
        self.set(id, &value).await?;
        Ok(value)
    }

    /// Invalidate all cache entries with the service prefix.
    pub async fn invalidate_all(&self) -> Result<(), Error> {
        self.cache.delete_pattern(&format!("{}:*", self.prefix)).await
    }
}
"#;

const CACHE_SERVICE_TEST_TEMPLATE: &str = r#"//! Tests for {{name_pascal}}CacheService.

use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TestData {
        id: u64,
        name: String,
    }

    // Note: These tests require a running cache backend (Redis, etc.)
    // In a real project, you'd use test containers or mock the cache

    #[tokio::test]
    #[ignore] // Requires running Redis
    async fn test_set_and_get() {
        let config = CacheConfig::default();
        let cache = {{name_pascal}}CacheService::new(config).await.unwrap();

        let data = TestData {
            id: 1,
            name: "Test".to_string(),
        };

        cache.set("test-1", &data).await.unwrap();
        let retrieved: Option<TestData> = cache.get("test-1").await.unwrap();

        assert_eq!(retrieved, Some(data));
    }

    #[tokio::test]
    #[ignore] // Requires running Redis
    async fn test_delete() {
        let config = CacheConfig::default();
        let cache = {{name_pascal}}CacheService::new(config).await.unwrap();

        let data = TestData {
            id: 1,
            name: "Test".to_string(),
        };

        cache.set("test-delete", &data).await.unwrap();
        cache.delete("test-delete").await.unwrap();

        let retrieved: Option<TestData> = cache.get("test-delete").await.unwrap();
        assert!(retrieved.is_none());
    }
}
"#;

// =============================================================================
// API CLIENT TEMPLATES
// =============================================================================

const API_CLIENT_TEMPLATE: &str = r#"//! {{name_pascal}} API client.

use armature::prelude::*;
use armature_http_client::{HttpClient, HttpClientConfig};
use serde::{de::DeserializeOwned, Serialize};
use std::time::Duration;

/// {{name_pascal}} API client.
///
/// Client for interacting with the {{name_pascal}} external API.
#[derive(Clone)]
pub struct {{name_pascal}}Client {
    client: HttpClient,
    base_url: String,
    api_key: Option<String>,
}

impl {{name_pascal}}Client {
    /// Create a new API client.
    pub fn new(base_url: impl Into<String>) -> Self {
        let config = HttpClientConfig::default()
            .timeout(Duration::from_secs(30))
            .retry_count(3);

        Self {
            client: HttpClient::new(config),
            base_url: base_url.into(),
            api_key: None,
        }
    }

    /// Create a client with API key authentication.
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Build a full URL for an endpoint.
    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    /// Add authentication headers if configured.
    fn add_auth_headers(&self, headers: &mut Vec<(String, String)>) {
        if let Some(ref api_key) = self.api_key {
            headers.push(("Authorization".to_string(), format!("Bearer {}", api_key)));
        }
    }

    /// GET request.
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        let mut headers = vec![];
        self.add_auth_headers(&mut headers);

        self.client
            .get(&self.url(path))
            .headers(headers)
            .send()
            .await?
            .json()
            .await
    }

    /// POST request with JSON body.
    pub async fn post<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, Error> {
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        self.add_auth_headers(&mut headers);

        self.client
            .post(&self.url(path))
            .headers(headers)
            .json(body)
            .send()
            .await?
            .json()
            .await
    }

    /// PUT request with JSON body.
    pub async fn put<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, Error> {
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        self.add_auth_headers(&mut headers);

        self.client
            .put(&self.url(path))
            .headers(headers)
            .json(body)
            .send()
            .await?
            .json()
            .await
    }

    /// DELETE request.
    pub async fn delete(&self, path: &str) -> Result<(), Error> {
        let mut headers = vec![];
        self.add_auth_headers(&mut headers);

        self.client
            .delete(&self.url(path))
            .headers(headers)
            .send()
            .await?;

        Ok(())
    }

    /// PATCH request with JSON body.
    pub async fn patch<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, Error> {
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        self.add_auth_headers(&mut headers);

        self.client
            .patch(&self.url(path))
            .headers(headers)
            .json(body)
            .send()
            .await?
            .json()
            .await
    }
}

/// Builder for {{name_pascal}}Client.
pub struct {{name_pascal}}ClientBuilder {
    base_url: String,
    api_key: Option<String>,
    timeout: Duration,
    retry_count: u32,
}

impl {{name_pascal}}ClientBuilder {
    /// Create a new builder.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: None,
            timeout: Duration::from_secs(30),
            retry_count: 3,
        }
    }

    /// Set the API key.
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Set the request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the retry count.
    pub fn retry_count(mut self, count: u32) -> Self {
        self.retry_count = count;
        self
    }

    /// Build the client.
    pub fn build(self) -> {{name_pascal}}Client {
        let mut client = {{name_pascal}}Client::new(self.base_url);
        if let Some(api_key) = self.api_key {
            client = client.with_api_key(api_key);
        }
        client
    }
}
"#;

// =============================================================================
// HELPER FUNCTIONS
// =============================================================================

/// Template data for controller generation.
#[derive(Serialize)]
pub struct ControllerData {
    pub name_pascal: String,
    pub name_snake: String,
    pub name_kebab: String,
    pub base_path: String,
}

/// Template data for module generation.
#[derive(Serialize)]
pub struct ModuleData {
    pub name_pascal: String,
    pub name_snake: String,
    pub controllers: Vec<String>,
    pub providers: Vec<String>,
    pub controller_list: String,
    pub provider_list: String,
}

/// Template data for middleware/guard/service generation.
#[derive(Serialize)]
pub struct ComponentData {
    pub name_pascal: String,
    pub name_snake: String,
    pub name_kebab: String,
}

/// Template data for project generation.
#[derive(Serialize)]
pub struct ProjectData {
    pub name_pascal: String,
    pub name_snake: String,
    pub name_kebab: String,
    pub name_upper: String,
    pub description: String,
    pub curly_open: String,
    pub curly_close: String,
}

impl ProjectData {
    /// Create new project data with default curly braces.
    pub fn new(
        name_pascal: String,
        name_snake: String,
        name_kebab: String,
        description: String,
    ) -> Self {
        Self {
            name_upper: name_snake.to_uppercase(),
            name_pascal,
            name_snake,
            name_kebab,
            description,
            curly_open: "{".to_string(),
            curly_close: "}".to_string(),
        }
    }
}

/// Template data for entity generation.
#[derive(Serialize)]
pub struct EntityData {
    pub name_pascal: String,
    pub name_snake: String,
    pub name_kebab: String,
}

/// Template data for Rhai script generation.
#[derive(Serialize)]
pub struct RhaiData {
    pub name_pascal: String,
    pub name_snake: String,
    pub base_path: String,
}

/// Template data for DevOps files (Docker, CI/CD).
#[derive(Serialize)]
pub struct DevOpsData {
    pub name_pascal: String,
    pub name_snake: String,
    pub name_kebab: String,
    pub name_upper: String,
    pub curly_open: String,
    pub curly_close: String,
}

impl DevOpsData {
    /// Create new DevOps data with escaped curly braces for GitHub Actions.
    pub fn new(name_pascal: String, name_snake: String, name_kebab: String) -> Self {
        Self {
            name_upper: name_snake.to_uppercase(),
            name_pascal,
            name_snake,
            name_kebab,
            curly_open: "{".to_string(),
            curly_close: "}".to_string(),
        }
    }
}
