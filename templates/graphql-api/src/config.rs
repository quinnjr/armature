//! Application configuration

use std::env;

/// Application configuration
#[derive(Clone)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    /// Used for the Playground's embedded link to the GraphQL endpoint.
    /// Does **not** move the endpoint itself: `#[controller("/graphql")]`
    /// in `main.rs` fixes the actual route at compile time (macro attributes
    /// can't read runtime config), so changing this without also editing
    /// that attribute leaves the Playground pointing at a route that isn't
    /// mounted there.
    pub graphql_path: String,
    /// Gates the `/playground` controller: when `false` it returns 404
    /// instead of serving GraphiQL.
    pub graphql_playground: bool,
    /// Enforced via `SchemaBuilder::limit_depth` when the schema is built.
    pub graphql_depth_limit: usize,
    /// Enforced via `SchemaBuilder::limit_complexity` when the schema is built.
    pub graphql_complexity_limit: usize,
    pub jwt_secret: String,
}

impl AppConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        Self {
            host: env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            port: env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(3000),
            graphql_path: env::var("GRAPHQL_PATH").unwrap_or_else(|_| "/graphql".to_string()),
            graphql_playground: env::var("GRAPHQL_PLAYGROUND")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(true),
            graphql_depth_limit: env::var("GRAPHQL_DEPTH_LIMIT")
                .ok()
                .and_then(|d| d.parse().ok())
                .unwrap_or(10),
            graphql_complexity_limit: env::var("GRAPHQL_COMPLEXITY_LIMIT")
                .ok()
                .and_then(|c| c.parse().ok())
                .unwrap_or(100),
            jwt_secret: env::var("JWT_SECRET")
                .unwrap_or_else(|_| "default-secret-change-me".to_string()),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self::from_env()
    }
}
