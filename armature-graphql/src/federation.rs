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
//! - **SDL Bundling**: Concatenate multiple subgraphs' SDL into a single
//!   bundle via [`compose_supergraph`] — a convenience for eyeballing/bundling
//!   SDL text, not real Apollo Federation composition (no type merging, no
//!   `@join`/`@link` generation, no conflict resolution). For production
//!   composition, use Apollo's official tooling (e.g. `rover supergraph compose`).
//!
//! ## Example: Creating a Subgraph
//!
//! `SubgraphSchemaBuilder` only builds an `async-graphql` [`Schema`] with
//! federation directives enabled — it does not run an HTTP server. Serve the
//! resulting schema yourself (e.g. with `async-graphql-axum`, added as a
//! dependency in your own binary) the same way you would serve any other
//! `async-graphql` schema.
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
//! let schema = SubgraphSchemaBuilder::new()
//!     .query(Query)
//!     .mutation(Mutation)
//!     .subscription(EmptySubscription)
//!     .enable_federation()
//!     .build()?;
//!
//! // `schema.schema()` returns the underlying `async_graphql::Schema`; wire
//! // it into your own HTTP router (e.g. `async_graphql_axum::GraphQL`, added
//! // as a dependency in your own binary) to actually serve it. This crate
//! // does not include a server/runtime.
//! ```
//!
//! ## Example: Querying subgraphs from a gateway
//!
//! `FederationGateway` (behind the `federation` feature) is a query-forwarding
//! HTTP *client* for talking to already-running subgraphs — it does not host
//! a gateway server and has no `listen()` method. Use it to fetch subgraph
//! SDLs, forward queries to a specific subgraph, or resolve entities:
//!
//! ```rust,ignore
//! use armature_graphql::federation::*;
//!
//! let gateway = FederationGateway::builder()
//!     .subgraph("users", "http://localhost:4001/graphql")
//!     .subgraph("products", "http://localhost:4002/graphql")
//!     .subgraph("orders", "http://localhost:4003/graphql")
//!     .build()?;
//!
//! // Forward a query to a specific subgraph
//! let result = gateway
//!     .execute_subgraph_query("users", "{ user(id: \"1\") { name } }", None)
//!     .await?;
//!
//! // Fetch every subgraph's SDL (e.g. for offline composition tooling)
//! let sdls = gateway.fetch_subgraph_sdls().await?;
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
    config: Option<crate::config::GraphQLConfig>,
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
            config: None,
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

    /// Attach a [`crate::config::GraphQLConfig`] whose security/behavior knobs
    /// (introspection, max depth, max complexity, validation) will be applied
    /// to the built subgraph schema. Without this, the schema is built
    /// unrestricted.
    pub fn config(mut self, config: crate::config::GraphQLConfig) -> Self {
        self.config = Some(config);
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

        // `self.enable_tracing()` and an attached `GraphQLConfig.enable_tracing`
        // (applied below via `config.configure()`) both register `ApolloTracing`.
        // Register it at most once here, regardless of how many of the two
        // flags are set, and skip the config-driven registration for tracing
        // specifically so it isn't added twice.
        let config_wants_tracing = self
            .config
            .as_ref()
            .map(|config| config.enable_tracing)
            .unwrap_or(false);
        if self.enable_tracing || config_wants_tracing {
            builder = builder.extension(ApolloTracing);
        }

        // Enable federation directives
        if self.enable_federation {
            builder = builder.enable_federation();
        }

        if let Some(config) = &self.config {
            let mut config = config.clone();
            config.enable_tracing = false; // already handled above; avoid double-registration
            builder = config.configure(builder);
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
        // Configured via the builder; not yet consumed by the gateway runtime.
        #[allow(dead_code)]
        enable_introspection: bool,
        #[allow(dead_code)]
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

        /// Resolve entities from a subgraph.
        ///
        /// `fields` is the GraphQL selection set (e.g. `"id name email"`, or
        /// nested selections like `"id address { city }"`) to request for
        /// each resolved entity. `__typename` is always included
        /// automatically. All `representations` must share the same
        /// `__typename` — the `_entities` field returns a `_Entity` union,
        /// so selecting concrete fields (rather than just `__typename`)
        /// requires an inline fragment (`... on <Type> { ... }`) on that one
        /// concrete type; batch calls per-type if you need to resolve
        /// entities of different types.
        ///
        /// Passing an empty `fields` string resolves only `__typename` for
        /// each entity (the previous, more limited behavior).
        pub async fn resolve_entities(
            &self,
            subgraph: &str,
            representations: Vec<EntityReference>,
            fields: &str,
        ) -> Result<Vec<serde_json::Value>, FederationError> {
            let config = self
                .subgraphs
                .get(subgraph)
                .ok_or_else(|| FederationError::SubgraphNotFound(subgraph.to_string()))?;

            if representations.is_empty() {
                return Ok(Vec::new());
            }

            let typename = &representations[0].typename;

            // Enforce the single-typename batch constraint this code (and the
            // doc comment above) already assumes: the query narrows to
            // `representations[0]`'s typename via a single inline fragment, so
            // a batch mixing typenames would silently send later entries under
            // the wrong fragment. Reject rather than mis-resolve.
            if let Some(mismatch) = representations.iter().find(|r| r.typename != *typename) {
                return Err(FederationError::EntityResolutionError(format!(
                    "all representations must share the same __typename; \
                     found both '{}' and '{}' in one batch",
                    typename, mismatch.typename
                )));
            }

            // `typename` is attacker-influenceable in a gateway (it is echoed
            // from inbound `representations`) and is spliced raw into the
            // query string, so validate it against the GraphQL Name grammar
            // before building the query to prevent GraphQL injection.
            if !is_valid_graphql_name(typename) {
                return Err(FederationError::EntityResolutionError(format!(
                    "invalid entity __typename {typename:?}: must match the GraphQL \
                     Name grammar /^[_A-Za-z][_0-9A-Za-z]*$/"
                )));
            }

            let query = build_entities_query(typename, fields);

            let variables = serde_json::json!({
                "representations": representations
            });

            let result = execute_query(&self.client, config, &query, Some(variables)).await?;

            if let Some(entities) = result["data"]["_entities"].as_array() {
                return Ok(entities.clone());
            }

            // No `_entities` array in `data`. Before collapsing to a generic
            // error, inspect the GraphQL `errors` array (a subgraph reporting
            // a resolution failure returns `{"errors":[...]}` with null/absent
            // data) and surface the real server messages rather than swallow
            // them.
            if let Some(errors) = result["errors"].as_array() {
                let messages: Vec<String> = errors
                    .iter()
                    .filter_map(|e| e["message"].as_str().map(str::to_string))
                    .collect();
                if !messages.is_empty() {
                    return Err(FederationError::EntityResolutionError(format!(
                        "subgraph returned errors: {}",
                        messages.join("; ")
                    )));
                }
            }

            Err(FederationError::EntityResolutionError(
                "Invalid response".into(),
            ))
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
        // Apply this subgraph's own `timeout_secs` to the individual
        // request, overriding the gateway client's global default timeout
        // for this call. `SubgraphConfig::with_timeout` previously set
        // `timeout_secs` but nothing ever read it back — the client-wide
        // timeout set once at gateway-build time was always used instead.
        let mut request = client
            .post(&config.url)
            .timeout(Duration::from_secs(config.timeout_secs));

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

/// Returns `true` if `name` matches the GraphQL `Name` grammar
/// (`/^[_A-Za-z][_0-9A-Za-z]*$/`): a leading `_` or ASCII letter followed by
/// any number of `_`, ASCII letters, or ASCII digits.
///
/// Hand-rolled (no `regex` dependency) and used to reject
/// attacker-influenceable `__typename` values before they are spliced into a
/// subgraph query, preventing GraphQL injection.
#[cfg_attr(not(feature = "federation"), allow(dead_code))]
fn is_valid_graphql_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// Build the `_entities` query for [`FederationGateway::resolve_entities`].
///
/// Uses an inline fragment on the concrete entity `typename` so the
/// requested `fields` (a raw GraphQL selection set) can actually be
/// selected — the `_entities` field returns a `_Entity` union, and only
/// `__typename` is selectable without narrowing to a concrete type via
/// `... on <Type> { ... }`.
///
/// # Trust assumptions
///
/// - `typename` MUST already be a valid GraphQL `Name` (see
///   [`is_valid_graphql_name`]). Callers (`resolve_entities`) validate it
///   before calling; it is spliced raw into the query, so an unvalidated
///   value would allow GraphQL injection.
/// - `fields` is a raw GraphQL selection set and is spliced verbatim into the
///   query. It is treated as **trusted, developer-supplied** input (the
///   selection the application wants for each entity), NOT as data echoed
///   from an inbound federation request. Do not forward attacker-controlled
///   strings as `fields`; validating a full selection set is out of scope
///   here (it is far more complex than a single Name).
#[cfg_attr(not(feature = "federation"), allow(dead_code))]
fn build_entities_query(typename: &str, fields: &str) -> String {
    let selection = if fields.trim().is_empty() {
        "__typename".to_string()
    } else {
        format!("__typename\n                        {}", fields.trim())
    };

    format!(
        r#"
                query($representations: [_Any!]!) {{
                    _entities(representations: $representations) {{
                        ... on {typename} {{
                        {selection}
                        }}
                    }}
                }}
            "#,
        typename = typename,
        selection = selection
    )
}

#[cfg(feature = "federation")]
pub use gateway::*;

// =============================================================================
// Federation Directives
// =============================================================================

/// Optional, purely conventional marker trait for entities that can be
/// resolved across subgraph boundaries.
///
/// Implementing this trait documents intent, but it is not currently
/// consumed anywhere by this crate's own code: no macro generates
/// implementations of it, and no gateway or entity-resolution logic in this
/// module calls `typename()` or `resolve()` on it. The actually-used
/// mechanism for entity resolution is async-graphql's native
/// `#[graphql(entity)]` attribute applied directly to a method on an
/// `#[Object]`/`#[ComplexObject]` impl — see this module's top-level doc
/// example (`find_by_id`) for how that is wired up in practice.
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
// Subgraph SDL Bundling Helpers
//
// NOTE: Despite the names below (kept for API compatibility), nothing in
// this module performs real Apollo supergraph composition. There is no
// type merging, no `@join`/`@link` directive generation, and no conflict
// resolution — `compose_supergraph` only concatenates each subgraph's raw
// SDL text behind comment separators and flags subgraphs missing `@key`.
// For a real, spec-compliant supergraph, use Apollo's official composition
// tooling (e.g. `rover supergraph compose`).
// =============================================================================

/// Result of [`compose_supergraph`] — a **concatenated bundle** of subgraph
/// SDLs, not a composed Apollo supergraph.
#[derive(Debug, Clone)]
pub struct ComposedSchema {
    /// The concatenated subgraph SDL bundle (raw SDL text from each
    /// subgraph joined with comment separators). This is NOT a valid,
    /// type-merged Apollo supergraph SDL — no `@join`/`@link` directives
    /// are generated and no conflicts are resolved.
    pub sdl: String,
    /// Hints/warnings produced while scanning the input SDLs (e.g. a
    /// subgraph with no `@key` directives)
    pub hints: Vec<String>,
    /// Subgraphs included in the bundle
    pub subgraphs: Vec<String>,
}

impl ComposedSchema {
    /// Check if bundling produced any hints
    pub fn has_hints(&self) -> bool {
        !self.hints.is_empty()
    }

    /// Get the number of subgraphs
    pub fn subgraph_count(&self) -> usize {
        self.subgraphs.len()
    }
}

/// Concatenate multiple subgraph SDLs into a single bundled SDL document.
///
/// **This does not perform real Apollo Federation composition.** It does no
/// type merging, generates no `@join`/`@link` directives, and resolves no
/// conflicts between subgraphs — it simply joins each subgraph's raw SDL
/// text behind a comment header, and emits a hint for any subgraph whose
/// SDL has no `@key` directive. The result is useful for eyeballing/bundling
/// SDL text, but it is **not** a valid supergraph SDL that a real Apollo
/// Gateway/Router could load. For production composition, use Apollo's
/// official tooling (e.g. `rover supergraph compose`).
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
///
/// Not currently populated or used by [`FederationGateway`]/
/// [`FederationGatewayBuilder`] (behind the `federation` feature) or
/// anywhere else in this crate — nothing ever constructs, updates, or
/// returns a `ServiceInfo`. If constructed via [`ServiceInfo::new`],
/// `healthy` and `sdl` will always be their default values (`false` and
/// `None`), since there is no other constructor or mutator. This is a
/// disconnected DTO today: a future extension point, not live
/// functionality.
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

    // -------------------------------------------------------------------
    // Regression test: `_entities` query selection set
    //
    // Previously `resolve_entities` always sent a hardcoded
    // `... on _Entity { __typename }` selection, so callers could never
    // get back anything but the typename regardless of what fields they
    // actually needed. `build_entities_query` now narrows to the
    // representations' concrete `__typename` via an inline fragment and
    // includes the caller-requested field selection.
    // -------------------------------------------------------------------

    #[test]
    fn test_build_entities_query_selects_requested_fields() {
        let query = build_entities_query("User", "id name email");

        assert!(query.contains("... on User"));
        assert!(query.contains("__typename"));
        assert!(query.contains("id name email"));
        // The old hardcoded query only ever fragmented on the union type
        // itself, which cannot select concrete fields.
        assert!(!query.contains("_Entity"));
    }

    #[test]
    fn test_build_entities_query_defaults_to_typename_only() {
        let query = build_entities_query("Product", "");

        assert!(query.contains("... on Product"));
        assert!(query.contains("__typename"));
    }

    // -------------------------------------------------------------------
    // Security regression: GraphQL Name validation for `__typename`.
    //
    // `__typename` is echoed from attacker-influenceable `representations`
    // in a federation gateway and is spliced raw into the subgraph query,
    // so it must be validated against the GraphQL Name grammar before use.
    // -------------------------------------------------------------------

    #[test]
    fn test_is_valid_graphql_name_accepts_valid_names() {
        assert!(is_valid_graphql_name("User"));
        assert!(is_valid_graphql_name("_Entity"));
        assert!(is_valid_graphql_name("Order2"));
        assert!(is_valid_graphql_name("a_b_c123"));
    }

    #[test]
    fn test_is_valid_graphql_name_rejects_injection_and_malformed() {
        // Classic GraphQL-injection payload that would rewrite the query.
        assert!(!is_valid_graphql_name(
            "User { id } } } query x { adminSecrets"
        ));
        // Braces, spaces, and other punctuation are all invalid Name chars.
        assert!(!is_valid_graphql_name("User { id }"));
        assert!(!is_valid_graphql_name("User Name"));
        assert!(!is_valid_graphql_name("User{"));
        assert!(!is_valid_graphql_name("User}"));
        // Empty and digit-leading names are invalid.
        assert!(!is_valid_graphql_name(""));
        assert!(!is_valid_graphql_name("2User"));
        // Non-ASCII letters are outside the GraphQL Name grammar.
        assert!(!is_valid_graphql_name("Usér"));
    }

    // -------------------------------------------------------------------
    // Regression test: no test previously asserted that a
    // federation-enabled subgraph SDL actually contains federation
    // directives, or that `is_federated()` reflects the schema's real
    // state.
    // -------------------------------------------------------------------

    #[derive(async_graphql::SimpleObject)]
    struct FedUser {
        id: async_graphql::ID,
        name: String,
    }

    struct FedQuery;

    #[async_graphql::Object]
    impl FedQuery {
        // The `@key` directive on `FedUser` is derived from this entity
        // resolver's `id` argument — async-graphql has no field-level
        // `#[graphql(key)]` attribute for `SimpleObject`.
        #[graphql(entity)]
        async fn find_user_by_id(&self, id: async_graphql::ID) -> FedUser {
            FedUser {
                id,
                name: "test".to_string(),
            }
        }

        async fn user(&self) -> FedUser {
            FedUser {
                id: "1".into(),
                name: "test".to_string(),
            }
        }
    }

    #[test]
    fn test_federated_subgraph_sdl_contains_federation_directives() {
        let schema = SubgraphSchemaBuilder::new()
            .query(FedQuery)
            .mutation(async_graphql::EmptyMutation)
            .subscription(async_graphql::EmptySubscription)
            .enable_federation()
            .build()
            .unwrap();

        assert!(
            schema.is_federated(),
            "is_federated() should reflect enable_federation()"
        );

        let sdl = schema.sdl();
        assert!(
            sdl.contains("@key"),
            "federation-enabled SDL should contain a @key directive: {sdl}"
        );
        assert!(
            sdl.contains("@link") && sdl.contains("specs.apollo.dev/federation"),
            "federation-enabled SDL should declare the Apollo Federation @link: {sdl}"
        );
    }

    #[test]
    fn test_non_federated_subgraph_is_federated_false() {
        let schema = SubgraphSchemaBuilder::new()
            .query(FedQuery)
            .mutation(async_graphql::EmptyMutation)
            .subscription(async_graphql::EmptySubscription)
            .build()
            .unwrap();

        assert!(
            !schema.is_federated(),
            "is_federated() should be false when enable_federation() was not called"
        );
    }

    // -------------------------------------------------------------------
    // Wiring test: SubgraphSchemaBuilder::config() actually reaches
    // the built schema (introspection is disabled when configured).
    // -------------------------------------------------------------------

    #[test]
    fn test_subgraph_schema_builder_applies_graphql_config() {
        use crate::config::GraphQLConfig;

        struct PlainQuery;

        #[async_graphql::Object]
        impl PlainQuery {
            async fn hello(&self) -> &str {
                "hi"
            }
        }

        let schema = SubgraphSchemaBuilder::new()
            .query(PlainQuery)
            .mutation(async_graphql::EmptyMutation)
            .subscription(async_graphql::EmptySubscription)
            .config(GraphQLConfig::production("/graphql"))
            .build()
            .unwrap();

        let response =
            tokio_test::block_on(schema.schema().execute("{ __schema { types { name } } }"));

        let data = response.data.into_json().unwrap();
        assert!(
            data["__schema"].is_null(),
            "GraphQLConfig::production() passed via .config() should disable introspection \
             on the built subgraph schema, but got: {:?}",
            data
        );
    }
}

#[cfg(all(test, feature = "federation"))]
mod gateway_tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    // -------------------------------------------------------------------
    // Regression test: `SubgraphConfig::with_timeout` was recorded on
    // the config but `execute_query` never applied it — only the gateway
    // builder's one-time, global client timeout was ever in effect. This
    // spins up a subgraph that accepts a connection and then never
    // responds, and asserts that a short per-subgraph `timeout_secs`
    // fires well before the (much longer) global client timeout.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn test_execute_query_applies_per_subgraph_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let addr = listener.local_addr().unwrap();

        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut buf = [0u8; 1024];
                // Read whatever the client sent, then hold the connection
                // open without responding so the client must rely on its
                // own timeout to give up.
                let _ = stream.read(&mut buf);
                std::thread::sleep(Duration::from_secs(30));
            }
        });

        let gateway = FederationGateway::builder()
            .subgraph_with_config(
                SubgraphConfig::new("slow", format!("http://{}", addr)).with_timeout(1),
            )
            // Global client timeout is deliberately much longer than the
            // per-subgraph override, so a passing test proves the
            // per-subgraph timeout is what actually fired.
            .timeout(60)
            .build()
            .expect("build gateway");

        let start = Instant::now();
        let result = gateway
            .execute_subgraph_query("slow", "{ __typename }", None)
            .await;
        let elapsed = start.elapsed();

        assert!(
            result.is_err(),
            "request to a hung subgraph should fail once its timeout fires"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "per-subgraph timeout_secs=1 should fire well before the 60s global client \
             timeout, but the request took {:?}",
            elapsed
        );
    }

    // -------------------------------------------------------------------
    // Security regression: `resolve_entities` must reject a malicious
    // `__typename` (GraphQL injection) BEFORE splicing it into the query
    // and BEFORE making any network call. We point the subgraph at an
    // unreachable address; a passing test proves the request failed at
    // validation, not at the network layer.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn test_resolve_entities_rejects_malicious_typename() {
        let gateway = FederationGateway::builder()
            // 127.0.0.1:0 is not a listening address; if we ever reached the
            // network we'd get a NetworkError instead of the validation error.
            .subgraph("users", "http://127.0.0.1:0/graphql")
            .build()
            .expect("build gateway");

        let malicious = EntityReference::new("User { id } } } query x { adminSecrets")
            .with_key("id", serde_json::json!("1"));

        let err = gateway
            .resolve_entities("users", vec![malicious], "id name")
            .await
            .expect_err("malicious __typename must be rejected");

        match err {
            FederationError::EntityResolutionError(msg) => {
                assert!(
                    msg.contains("invalid entity __typename"),
                    "expected a Name-validation error, got: {msg}"
                );
            }
            other => panic!("expected EntityResolutionError, got: {other:?}"),
        }
    }

    // -------------------------------------------------------------------
    // Security regression: a batch mixing `__typename` values must be
    // rejected, since the query narrows to representations[0]'s type via a
    // single inline fragment and would otherwise silently mis-resolve the
    // rest.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn test_resolve_entities_rejects_mixed_typename_batch() {
        let gateway = FederationGateway::builder()
            .subgraph("users", "http://127.0.0.1:0/graphql")
            .build()
            .expect("build gateway");

        let reps = vec![
            EntityReference::new("User").with_key("id", serde_json::json!("1")),
            EntityReference::new("Product").with_key("id", serde_json::json!("2")),
        ];

        let err = gateway
            .resolve_entities("users", reps, "id")
            .await
            .expect_err("mixed-typename batch must be rejected");

        match err {
            FederationError::EntityResolutionError(msg) => {
                assert!(
                    msg.contains("same __typename"),
                    "expected a batch-uniformity error, got: {msg}"
                );
            }
            other => panic!("expected EntityResolutionError, got: {other:?}"),
        }
    }

    // -------------------------------------------------------------------
    // Happy path: a valid, uniform batch passes validation and resolves
    // against a real (canned) subgraph response. Proves the security
    // checks did not break normal operation.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn test_resolve_entities_valid_typename_still_resolves() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let addr = listener.local_addr().unwrap();

        std::thread::spawn(move || {
            if let Some(mut stream) = listener.incoming().flatten().next() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let json =
                    r#"{"data":{"_entities":[{"__typename":"User","id":"1","name":"Ada"}]}}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    json.len(),
                    json
                );
                use std::io::Write;
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        let gateway = FederationGateway::builder()
            .subgraph("users", format!("http://{}/graphql", addr))
            .build()
            .expect("build gateway");

        let reps = vec![EntityReference::new("User").with_key("id", serde_json::json!("1"))];

        let entities = gateway
            .resolve_entities("users", reps, "id name")
            .await
            .expect("valid typename should resolve");

        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0]["__typename"], "User");
        assert_eq!(entities[0]["name"], "Ada");
    }

    // -------------------------------------------------------------------
    // Error path: when the subgraph responds with a GraphQL `errors`
    // array and no `_entities`, the real server messages must be surfaced
    // in the `EntityResolutionError` rather than collapsed to the generic
    // "Invalid response".
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn test_resolve_entities_surfaces_subgraph_errors() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let addr = listener.local_addr().unwrap();

        std::thread::spawn(move || {
            if let Some(mut stream) = listener.incoming().flatten().next() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let json =
                    r#"{"data":null,"errors":[{"message":"entity not found"},{"message":"boom"}]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    json.len(),
                    json
                );
                use std::io::Write;
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        let gateway = FederationGateway::builder()
            .subgraph("users", format!("http://{}/graphql", addr))
            .build()
            .expect("build gateway");

        let reps = vec![EntityReference::new("User").with_key("id", serde_json::json!("1"))];

        let err = gateway
            .resolve_entities("users", reps, "id name")
            .await
            .expect_err("subgraph error response should surface as an error");

        match err {
            FederationError::EntityResolutionError(msg) => {
                assert!(
                    msg.contains("entity not found") && msg.contains("boom"),
                    "expected subgraph error messages to be surfaced, got: {msg}"
                );
            }
            other => panic!("expected EntityResolutionError, got: {other:?}"),
        }
    }
}
