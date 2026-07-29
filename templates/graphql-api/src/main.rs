//! Armature GraphQL API Template
//!
//! A production-ready GraphQL API with queries, mutations, and subscriptions.
//!
//! Run with: cargo run
//! GraphQL Playground: http://localhost:3000/playground

mod config;
mod schema;
mod services;

use std::sync::OnceLock;

use armature::prelude::*;
use armature::armature_graphql::graphiql_html;
use async_graphql::{EmptySubscription, Schema};
use serde::Deserialize;
use tracing::info;

use crate::config::AppConfig;
use crate::schema::{MutationRoot, QueryRoot};
use crate::services::{BookService, UserService};

// =============================================================================
// GraphQL Controller
// =============================================================================

/// Build the GraphQL schema, enforcing the depth/complexity limits from
/// [`AppConfig`] (`GRAPHQL_DEPTH_LIMIT` / `GRAPHQL_COMPLEXITY_LIMIT`).
fn build_schema(config: &AppConfig) -> Schema<QueryRoot, MutationRoot, EmptySubscription> {
    let user_service = UserService::new();
    let book_service = BookService::new();

    let query = QueryRoot::new(user_service.clone(), book_service.clone());
    let mutation = MutationRoot::new(user_service, book_service);
    Schema::build(query, mutation, EmptySubscription)
        .limit_depth(config.graphql_depth_limit)
        .limit_complexity(config.graphql_complexity_limit)
        .finish()
}

// The config and schema (and the `UserService`/`BookService` state graph the
// schema closes over) are built once at startup and shared across requests
// via these process-wide singletons, instead of being re-parsed from the
// environment and rebuilt from scratch on every single request. This also
// fixes a correctness bug: rebuilding `UserService`/`BookService` per request
// silently reset their in-memory stores on every call.
static APP_CONFIG: OnceLock<AppConfig> = OnceLock::new();
static GRAPHQL_SCHEMA: OnceLock<Schema<QueryRoot, MutationRoot, EmptySubscription>> =
    OnceLock::new();

fn init_app_state(config: AppConfig) {
    let schema = build_schema(&config);
    APP_CONFIG
        .set(config)
        .unwrap_or_else(|_| panic!("APP_CONFIG already initialized"));
    GRAPHQL_SCHEMA
        .set(schema)
        .unwrap_or_else(|_| panic!("GRAPHQL_SCHEMA already initialized"));
}

fn app_config() -> &'static AppConfig {
    APP_CONFIG.get().expect("AppConfig not initialized")
}

fn graphql_schema() -> &'static Schema<QueryRoot, MutationRoot, EmptySubscription> {
    GRAPHQL_SCHEMA.get().expect("Schema not initialized")
}

#[controller("/graphql")]
#[derive(Default)]
struct GraphQLController;

#[routes]
impl GraphQLController {
    #[post("")]
    async fn execute(req: HttpRequest) -> Result<HttpResponse, Error> {
        let schema = graphql_schema();

        // Parse GraphQL request
        #[derive(Deserialize)]
        struct GraphQLRequest {
            query: String,
            #[serde(default)]
            variables: Option<serde_json::Value>,
            #[serde(default, rename = "operationName")]
            operation_name: Option<String>,
        }

        let gql_req: GraphQLRequest = req.json()?;

        // Build async-graphql request
        let mut request = async_graphql::Request::new(gql_req.query);
        if let Some(vars) = gql_req.variables {
            request = request.variables(async_graphql::Variables::from_json(vars));
        }
        if let Some(op_name) = gql_req.operation_name {
            request = request.operation_name(op_name);
        }

        // Execute query
        let response = schema.execute(request).await;

        // Convert to JSON response
        let json =
            serde_json::to_value(&response).map_err(|e| Error::Serialization(e.to_string()))?;

        HttpResponse::ok().with_json(&json)
    }

    #[get("/schema")]
    async fn get_schema() -> Result<HttpResponse, Error> {
        let sdl = graphql_schema().sdl();

        Ok(HttpResponse::ok()
            .with_header("Content-Type".to_string(), "text/plain".to_string())
            .with_body(sdl.into_bytes()))
    }
}

// =============================================================================
// Playground Controller
// =============================================================================

#[controller("/playground")]
#[derive(Default)]
struct PlaygroundController;

#[routes]
impl PlaygroundController {
    #[get("")]
    async fn playground() -> Result<HttpResponse, Error> {
        let config = app_config();
        if !config.graphql_playground {
            return Err(Error::NotFound(
                "GraphQL Playground is disabled (set GRAPHQL_PLAYGROUND=true to enable)"
                    .to_string(),
            ));
        }

        // The embedded link honors the configured `GRAPHQL_PATH`, but the
        // GraphQL endpoint itself is mounted via `#[controller("/graphql")]`
        // above, whose path is a compile-time macro argument and cannot be
        // driven by runtime config. If `GRAPHQL_PATH` is customized, update
        // that attribute too (see the startup warning in `main()`).
        let html = graphiql_html(&config.graphql_path);
        Ok(HttpResponse::ok()
            .with_header("Content-Type".to_string(), "text/html".to_string())
            .with_body(html.into_bytes()))
    }
}

// =============================================================================
// Health Controller
// =============================================================================

#[controller("/health")]
#[derive(Default)]
struct HealthController;

#[routes]
impl HealthController {
    #[get("")]
    async fn check() -> Result<HttpResponse, Error> {
        HttpResponse::ok().with_json(&serde_json::json!({
            "status": "healthy",
            "service": "graphql-api"
        }))
    }

    #[get("/live")]
    async fn liveness() -> Result<HttpResponse, Error> {
        HttpResponse::ok().with_json(&serde_json::json!({ "status": "alive" }))
    }

    #[get("/ready")]
    async fn readiness() -> Result<HttpResponse, Error> {
        HttpResponse::ok().with_json(&serde_json::json!({ "status": "ready" }))
    }
}

// =============================================================================
// Application Module
// =============================================================================

#[module(
    providers: [UserService, BookService],
    controllers: [GraphQLController, PlaygroundController, HealthController]
)]
#[derive(Default)]
struct AppModule;

// =============================================================================
// Main
// =============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenvy::dotenv().ok();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_target(true)
        .init();

    info!("🚀 Starting Armature GraphQL API");

    // Load configuration
    let config = AppConfig::from_env();

    info!(host = %config.host, port = %config.port, "Configuration loaded");

    // `GRAPHQL_PATH` drives the Playground's embedded link, but the actual
    // GraphQL endpoint is mounted via `#[controller("/graphql")]`, whose
    // path is a compile-time macro argument and cannot be moved at runtime.
    if config.graphql_path != "/graphql" {
        tracing::warn!(
            configured_graphql_path = %config.graphql_path,
            "GRAPHQL_PATH is set but has no effect on routing: the GraphQL \
             endpoint is fixed at \"/graphql\" by #[controller(\"/graphql\")] \
             in main.rs. Edit that attribute (and the /graphql/schema, \
             /playground routes that reference it) to actually move the endpoint."
        );
    }

    let port = config.port;

    // Build the config/schema/service graph once and share it across all
    // requests via a process-wide `OnceLock`, instead of re-parsing the
    // environment and rebuilding everything from scratch per request.
    init_app_state(config);

    // Create application
    let app = Application::create::<AppModule>().await;

    println!();
    println!("🎯 GraphQL API ready!");
    println!();
    println!("Endpoints:");
    println!("  POST /graphql        - GraphQL endpoint");
    println!("  GET  /graphql/schema - GraphQL SDL");
    println!("  GET  /playground     - GraphiQL Playground");
    println!("  GET  /health         - Health check");
    println!("  GET  /health/live    - Liveness probe");
    println!("  GET  /health/ready   - Readiness probe");
    println!();
    println!("Example queries:");
    println!();
    println!("  # List all users");
    println!("  query {{ users {{ id name email role }} }}");
    println!();
    println!("  # Get a specific book with author");
    println!("  query {{ book(id: \"1\") {{ id title authorId }} }}");
    println!();
    println!("  # Create a user");
    println!("  mutation {{ createUser(name: \"Alice\", email: \"alice@example.com\") {{ id name }} }}");
    println!();
    println!("  # Search books");
    println!("  query {{ searchBooks(query: \"Rust\") {{ id title }} }}");
    println!();

    if let Err(e) = app.listen(port).await {
        eprintln!("Server error: {}", e);
    }

    Ok(())
}
