//! Apollo Federation v2 Support
//!
//! This module provides GraphQL Federation capabilities for building
//! microservices-based GraphQL architectures.
//!
//! ## Architecture
//!
//! ```text
//!                     ┌─────────────────┐
//!                     │   API Gateway   │
//!                     │   (Supergraph)  │
//!                     └────────┬────────┘
//!                              │
//!          ┌───────────────────┼───────────────────┐
//!          │                   │                   │
//!    ┌─────┴─────┐      ┌─────┴─────┐      ┌─────┴─────┐
//!    │  Users    │      │  Products │      │  Orders   │
//!    │ Subgraph  │      │  Subgraph │      │ Subgraph  │
//!    └───────────┘      └───────────┘      └───────────┘
//! ```
//!
//! ## Features
//!
//! - **Subgraph Creation**: Define federated subgraphs with entity resolvers
//! - **Entity Resolution**: Automatic `_entities` query implementation
//! - **Federation Directives**: `@key`, `@external`, `@requires`, `@provides`, `@shareable`
//! - **Gateway Composition**: Compose multiple subgraphs into a supergraph
//!
//! ## Example: Creating a Subgraph
//!
//! ```rust,ignore
//! use armature_graphql::federation::*;
//! use async_graphql::*;
//!
//! // Define an entity with @key directive
//! #[derive(SimpleObject)]
//! #[graphql(complex)]
//! struct User {
//!     #[graphql(key)]
//!     id: ID,
//!     name: String,
//!     email: String,
//! }
//!
//! // Implement entity reference resolver
//! #[ComplexObject]
//! impl User {
//!     // Federation _entities resolver
//!     #[graphql(entity)]
//!     async fn find_by_id(id: ID) -> Option<Self> {
//!         // Resolve user by ID
//!         Some(User { id, name: "John".into(), email: "john@example.com".into() })
//!     }
//! }
//!
//! // Create the subgraph schema
//! let schema = SubgraphSchema::builder()
//!     .query(Query)
//!     .mutation(Mutation)
//!     .enable_federation()
//!     .build()?;
//!
//! // Start the subgraph server
//! SubgraphServer::new(schema)
//!     .listen(4001)
//!     .await?;
//! ```
//!
//! ## Example: Creating a Gateway
//!
//! ```rust,ignore
//! use armature_graphql::federation::*;
//!
//! let gateway = FederationGateway::builder()
//!     .subgraph("users", "http://localhost:4001/graphql")
//!     .subgraph("products", "http://localhost:4002/graphql")
//!     .subgraph("orders", "http://localhost:4003/graphql")
//!     .enable_introspection()
//!     .build()
//!     .await?;
//!
//! gateway.listen(4000).await?;
//! ```

use async_graphql::{ObjectType, SDLExportOptions, Schema, SubscriptionType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

// Re-export federation-related items from async-graphql
pub use async_graphql::extensions::ApolloTracing;

/// Federation-specific errors
#[derive(Debug, Error)]
pub enum FederationError {
    #[error("Subgraph '{0}' not found")]
    SubgraphNotFound(String),

    #[error("Failed to compose supergraph: {0}")]
    CompositionError(String),

    #[error("Entity resolution failed: {0}")]
    EntityResolutionError(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Schema introspection failed: {0}")]
    IntrospectionError(String),

    #[error("Invalid federation directive: {0}")]
    InvalidDirective(String),
}

// =============================================================================
// Subgraph Definition
// =============================================================================

/// Configuration for a federated subgraph
#[derive(Debug, Clone)]
pub struct SubgraphConfig {
    /// Unique name for this subgraph
    pub name: String,
    /// URL where this subgraph is hosted
    pub url: String,
    /// Whether to enable federation v2 features
    pub federation_v2: bool,
    /// Custom headers to include in requests
    pub headers: HashMap<String, String>,
    /// Timeout for requests (in seconds)
    pub timeout_secs: u64,
    /// Enable Apollo tracing
    pub enable_tracing: bool,
}

impl SubgraphConfig {
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            federation_v2: true,
            headers: HashMap::new(),
            timeout_secs: 30,
            enable_tracing: false,
        }
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    pub fn with_tracing(mut self, enabled: bool) -> Self {
        self.enable_tracing = enabled;
        self
    }
}

// =============================================================================
// Subgraph Schema Builder
// =============================================================================

/// Builder for federated subgraph schemas
///
/// # Example
///
/// ```rust,ignore
/// let schema = SubgraphSchemaBuilder::new()
///     .query(QueryRoot)
///     .mutation(MutationRoot)
///     .data(database_pool)
///     .enable_federation()
///     .build()?;
/// ```
pub struct SubgraphSchemaBuilder<Query, Mutation, Subscription> {
    query: Option<Query>,
    mutation: Option<Mutation>,
    subscription: Option<Subscription>,
    enable_federation: bool,
    enable_tracing: bool,
}

impl<Query, Mutation, Subscription> SubgraphSchemaBuilder<Query, Mutation, Subscription>
where
    Query: ObjectType + 'static,
    Mutation: ObjectType + 'static,
    Subscription: SubscriptionType + 'static,
{
    pub fn new() -> Self {
        Self {
            query: None,
            mutation: None,
            subscription: None,
            enable_federation: false,
            enable_tracing: false,
        }
    }

    pub fn query(mut self, query: Query) -> Self {
        self.query = Some(query);
        self
    }

    pub fn mutation(mut self, mutation: Mutation) -> Self {
        self.mutation = Some(mutation);
        self
    }

    pub fn subscription(mut self, subscription: Subscription) -> Self {
        self.subscription = Some(subscription);
        self
    }

    /// Enable Apollo Federation v2 support
    pub fn enable_federation(mut self) -> Self {
        self.enable_federation = true;
        self
    }

    /// Enable Apollo Tracing extension
    pub fn enable_tracing(mut self) -> Self {
        self.enable_tracing = true;
        self
    }

    /// Build the subgraph schema
    pub fn build(self) -> Result<SubgraphSchema<Query, Mutation, Subscription>, FederationError> {
        let query = self
            .query
            .ok_or_else(|| FederationError::CompositionError("Query root is required".into()))?;
        let mutation = self
            .mutation
            .ok_or_else(|| FederationError::CompositionError("Mutation root is required".into()))?;
        let subscription = self.subscription.ok_or_else(|| {
            FederationError::CompositionError("Subscription root is required".into())
        })?;

        let mut builder = Schema::build(query, mutation, subscription);

        if self.enable_tracing {
            builder = builder.extension(ApolloTracing);
        }

        // Enable federation directives
        if self.enable_federation {
            builder = builder.enable_federation();
        }

        let schema = builder.finish();

        Ok(SubgraphSchema {
            schema: Arc::new(schema),
            federation_enabled: self.enable_federation,
        })
    }
}

impl<Query, Mutation, Subscription> Default for SubgraphSchemaBuilder<Query, Mutation, Subscription>
where
    Query: ObjectType + 'static,
    Mutation: ObjectType + 'static,
    Subscription: SubscriptionType + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

/// A federated subgraph schema
pub struct SubgraphSchema<Query, Mutation, Subscription> {
    schema: Arc<Schema<Query, Mutation, Subscription>>,
    federation_enabled: bool,
}

impl<Query, Mutation, Subscription> SubgraphSchema<Query, Mutation, Subscription>
where
    Query: ObjectType + 'static,
    Mutation: ObjectType + 'static,
    Subscription: SubscriptionType + 'static,
{
    /// Get the SDL (Schema Definition Language) for this subgraph
    pub fn sdl(&self) -> String {
        if self.federation_enabled {
            // Export SDL with federation directives
            self.schema
                .sdl_with_options(SDLExportOptions::new().federation())
        } else {
            self.schema.sdl()
        }
    }

    /// Get a reference to the inner schema
    pub fn schema(&self) -> &Schema<Query, Mutation, Subscription> {
        &self.schema
    }

    /// Get an Arc reference to the schema
    pub fn schema_arc(&self) -> Arc<Schema<Query, Mutation, Subscription>> {
        Arc::clone(&self.schema)
    }

    /// Check if federation is enabled
    pub fn is_federated(&self) -> bool {
        self.federation_enabled
    }
}

impl<Query, Mutation, Subscription> Clone for SubgraphSchema<Query, Mutation, Subscription> {
    fn clone(&self) -> Self {
        Self {
            schema: Arc::clone(&self.schema),
            federation_enabled: self.federation_enabled,
        }
    }
}

// =============================================================================
// Entity Types
// =============================================================================

/// Represents a federated entity reference
///
/// Used for resolving entities across subgraph boundaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityReference {
    /// The typename of the entity
    #[serde(rename = "__typename")]
    pub typename: String,
    /// Key fields for entity resolution
    #[serde(flatten)]
    pub key_fields: HashMap<String, serde_json::Value>,
}

impl EntityReference {
    pub fn new(typename: impl Into<String>) -> Self {
        Self {
            typename: typename.into(),
            key_fields: HashMap::new(),
        }
    }

    pub fn with_key(mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        self.key_fields.insert(key.into(), value.into());
        self
    }

    /// Get a key field as a specific type
    pub fn get_key<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.key_fields
            .get(key)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }
}

// =============================================================================
// Federation Gateway (feature-gated)
// =============================================================================

#[cfg(feature = "federation")]
mod gateway {
    use super::*;
    use futures::future::join_all;
    use reqwest::Client;
    use std::time::Duration;

    /// Federation gateway for composing multiple subgraphs
    ///
    /// The gateway acts as the single entry point for clients,
    /// routing requests to appropriate subgraphs and composing results.
    pub struct FederationGateway {
        subgraphs: HashMap<String, SubgraphConfig>,
        client: Client,
        enable_introspection: bool,
        enable_playground: bool,
    }

    impl FederationGateway {
        /// Create a new gateway builder
        pub fn builder() -> FederationGatewayBuilder {
            FederationGatewayBuilder::new()
        }

        /// Get all registered subgraphs
        pub fn subgraphs(&self) -> &HashMap<String, SubgraphConfig> {
            &self.subgraphs
        }

        /// Check if a subgraph exists
        pub fn has_subgraph(&self, name: &str) -> bool {
            self.subgraphs.contains_key(name)
        }

        /// Get a subgraph configuration
        pub fn get_subgraph(&self, name: &str) -> Option<&SubgraphConfig> {
            self.subgraphs.get(name)
        }

        /// Fetch SDL from all subgraphs
        pub async fn fetch_subgraph_sdls(
            &self,
        ) -> Result<HashMap<String, String>, FederationError> {
            let futures: Vec<_> = self
                .subgraphs
                .iter()
                .map(|(name, config)| {
                    let client = self.client.clone();
                    let name = name.clone();
                    let url = config.url.clone();
                    async move {
                        let sdl = fetch_sdl(&client, &url).await?;
                        Ok::<_, FederationError>((name, sdl))
                    }
                })
                .collect();

            let results = join_all(futures).await;
            let mut sdls = HashMap::new();

            for result in results {
                let (name, sdl) = result?;
                sdls.insert(name, sdl);
            }

            Ok(sdls)
        }

        /// Execute a query against a specific subgraph
        pub async fn execute_subgraph_query(
            &self,
            subgraph: &str,
            query: &str,
            variables: Option<serde_json::Value>,
        ) -> Result<serde_json::Value, FederationError> {
            let config = self
                .subgraphs
                .get(subgraph)
                .ok_or_else(|| FederationError::SubgraphNotFound(subgraph.to_string()))?;

            execute_query(&self.client, config, query, variables).await
        }

        /// Resolve entities from a subgraph
        pub async fn resolve_entities(
            &self,
            subgraph: &str,
            representations: Vec<EntityReference>,
        ) -> Result<Vec<serde_json::Value>, FederationError> {
            let config = self
                .subgraphs
                .get(subgraph)
                .ok_or_else(|| FederationError::SubgraphNotFound(subgraph.to_string()))?;

            let query = r#"
                query($representations: [_Any!]!) {
                    _entities(representations: $representations) {
                        ... on _Entity {
                            __typename
                        }
                    }
                }
            "#;

            let variables = serde_json::json!({
                "representations": representations
            });

            let result = execute_query(&self.client, config, query, Some(variables)).await?;

            result["data"]["_entities"]
                .as_array()
                .cloned()
                .ok_or_else(|| FederationError::EntityResolutionError("Invalid response".into()))
        }
    }

    /// Builder for FederationGateway
    pub struct FederationGatewayBuilder {
        subgraphs: HashMap<String, SubgraphConfig>,
        enable_introspection: bool,
        enable_playground: bool,
        timeout_secs: u64,
    }

    impl FederationGatewayBuilder {
        pub fn new() -> Self {
            Self {
                subgraphs: HashMap::new(),
                enable_introspection: true,
                enable_playground: true,
                timeout_secs: 30,
            }
        }

        /// Add a subgraph
        pub fn subgraph(mut self, name: impl Into<String>, url: impl Into<String>) -> Self {
            let name = name.into();
            let config = SubgraphConfig::new(name.clone(), url);
            self.subgraphs.insert(name, config);
            self
        }

        /// Add a subgraph with custom configuration
        pub fn subgraph_with_config(mut self, config: SubgraphConfig) -> Self {
            self.subgraphs.insert(config.name.clone(), config);
            self
        }

        /// Enable or disable introspection
        pub fn enable_introspection(mut self, enabled: bool) -> Self {
            self.enable_introspection = enabled;
            self
        }

        /// Enable or disable the GraphQL playground
        pub fn enable_playground(mut self, enabled: bool) -> Self {
            self.enable_playground = enabled;
            self
        }

        /// Set default timeout for subgraph requests
        pub fn timeout(mut self, secs: u64) -> Self {
            self.timeout_secs = secs;
            self
        }

        /// Build the gateway
        pub fn build(self) -> Result<FederationGateway, FederationError> {
            if self.subgraphs.is_empty() {
                return Err(FederationError::CompositionError(
                    "At least one subgraph is required".into(),
                ));
            }

            let client = Client::builder()
                .timeout(Duration::from_secs(self.timeout_secs))
                .build()
                .map_err(|e| FederationError::NetworkError(e.to_string()))?;

            Ok(FederationGateway {
                subgraphs: self.subgraphs,
                client,
                enable_introspection: self.enable_introspection,
                enable_playground: self.enable_playground,
            })
        }
    }

    impl Default for FederationGatewayBuilder {
        fn default() -> Self {
            Self::new()
        }
    }

    /// Fetch SDL from a subgraph
    async fn fetch_sdl(client: &Client, url: &str) -> Result<String, FederationError> {
        let query = r#"{ _service { sdl } }"#;

        let response = client
            .post(url)
            .json(&serde_json::json!({ "query": query }))
            .send()
            .await
            .map_err(|e| FederationError::NetworkError(e.to_string()))?;

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| FederationError::IntrospectionError(e.to_string()))?;

        body["data"]["_service"]["sdl"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| FederationError::IntrospectionError("SDL not found in response".into()))
    }

    /// Execute a GraphQL query against a subgraph
    async fn execute_query(
        client: &Client,
        config: &SubgraphConfig,
        query: &str,
        variables: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, FederationError> {
        let mut request = client.post(&config.url);

        // Add custom headers
        for (key, value) in &config.headers {
            request = request.header(key, value);
        }

        let body = if let Some(vars) = variables {
            serde_json::json!({ "query": query, "variables": vars })
        } else {
            serde_json::json!({ "query": query })
        };

        let response = request
            .json(&body)
            .send()
            .await
            .map_err(|e| FederationError::NetworkError(e.to_string()))?;

        response
            .json()
            .await
            .map_err(|e| FederationError::NetworkError(e.to_string()))
    }
}

#[cfg(feature = "federation")]
pub use gateway::*;

// =============================================================================
// Federation Directives
// =============================================================================

/// Marker trait for entities that can be resolved across subgraph boundaries
///
/// Entities are the core building blocks of Apollo Federation.
/// They can be referenced and extended by other subgraphs.
pub trait FederatedEntity: Sized {
    /// The typename as it appears in the schema
    fn typename() -> &'static str;

    /// Resolve this entity from a reference
    ///
    /// This is called when another subgraph references this entity.
    fn resolve(reference: EntityReference) -> Option<Self>;
}

/// Helper for creating entity key representations
#[derive(Debug, Clone)]
pub struct EntityKey {
    fields: Vec<String>,
}

impl EntityKey {
    /// Create a new entity key with the given fields
    pub fn new(fields: &[&str]) -> Self {
        Self {
            fields: fields.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Get the key fields
    pub fn fields(&self) -> &[String] {
        &self.fields
    }

    /// Generate the @key directive string
    pub fn directive(&self) -> String {
        format!("@key(fields: \"{}\")", self.fields.join(" "))
    }
}

// =============================================================================
// Supergraph Composition Helpers
// =============================================================================

/// Result of composing multiple subgraph schemas
#[derive(Debug, Clone)]
pub struct ComposedSchema {
    /// The composed supergraph SDL
    pub sdl: String,
    /// Hints/warnings from composition
    pub hints: Vec<String>,
    /// Subgraphs included in composition
    pub subgraphs: Vec<String>,
}

impl ComposedSchema {
    /// Check if composition produced any hints
    pub fn has_hints(&self) -> bool {
        !self.hints.is_empty()
    }

    /// Get the number of subgraphs
    pub fn subgraph_count(&self) -> usize {
        self.subgraphs.len()
    }
}

/// Compose multiple subgraph SDLs into a supergraph
///
/// This is a simplified composition - for production use, consider
/// using Apollo's official composition tools (rover, etc.)
pub fn compose_supergraph(
    subgraphs: HashMap<String, String>,
) -> Result<ComposedSchema, FederationError> {
    if subgraphs.is_empty() {
        return Err(FederationError::CompositionError(
            "No subgraphs provided".into(),
        ));
    }

    let mut hints = Vec::new();
    let mut combined_sdl = String::new();

    // Add federation boilerplate
    combined_sdl.push_str("# Supergraph composed from federated subgraphs\n");
    combined_sdl.push_str("# Generated by Armature GraphQL Federation\n\n");

    // Add each subgraph's SDL (simplified composition)
    for (name, sdl) in &subgraphs {
        combined_sdl.push_str(&format!("# --- Subgraph: {} ---\n", name));
        combined_sdl.push_str(sdl);
        combined_sdl.push_str("\n\n");

        // Check for common issues
        if !sdl.contains("@key") {
            hints.push(format!(
                "Subgraph '{}' has no @key directives - entities may not be resolvable",
                name
            ));
        }
    }

    Ok(ComposedSchema {
        sdl: combined_sdl,
        hints,
        subgraphs: subgraphs.keys().cloned().collect(),
    })
}

// =============================================================================
// Federation Service Info
// =============================================================================

/// Information about a federated service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    /// The service name
    pub name: String,
    /// The service URL
    pub url: String,
    /// The service SDL
    pub sdl: Option<String>,
    /// Health status
    pub healthy: bool,
    /// Federation version
    pub federation_version: String,
}

impl ServiceInfo {
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            sdl: None,
            healthy: false,
            federation_version: "2".to_string(),
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subgraph_config() {
        let config = SubgraphConfig::new("users", "http://localhost:4001/graphql")
            .with_header("Authorization", "Bearer token")
            .with_timeout(60)
            .with_tracing(true);

        assert_eq!(config.name, "users");
        assert_eq!(config.url, "http://localhost:4001/graphql");
        assert!(config.headers.contains_key("Authorization"));
        assert_eq!(config.timeout_secs, 60);
        assert!(config.enable_tracing);
    }

    #[test]
    fn test_entity_reference() {
        let reference = EntityReference::new("User")
            .with_key("id", serde_json::json!("123"))
            .with_key("email", serde_json::json!("test@example.com"));

        assert_eq!(reference.typename, "User");
        assert_eq!(reference.get_key::<String>("id"), Some("123".to_string()));
    }

    #[test]
    fn test_entity_key() {
        let key = EntityKey::new(&["id", "email"]);
        assert_eq!(key.directive(), "@key(fields: \"id email\")");
    }

    #[test]
    fn test_compose_empty_subgraphs() {
        let result = compose_supergraph(HashMap::new());
        assert!(result.is_err());
    }

    #[test]
    fn test_compose_subgraphs() {
        let mut subgraphs = HashMap::new();
        subgraphs.insert(
            "users".to_string(),
            "type User @key(fields: \"id\") { id: ID! name: String! }".to_string(),
        );
        subgraphs.insert(
            "products".to_string(),
            "type Product { id: ID! name: String! }".to_string(),
        );

        let result = compose_supergraph(subgraphs).unwrap();
        assert_eq!(result.subgraph_count(), 2);
        assert!(result.sdl.contains("User"));
        assert!(result.sdl.contains("Product"));
        // Should have a hint about products missing @key
        assert!(result.has_hints());
    }
}
