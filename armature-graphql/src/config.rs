/// GraphQL server configuration
#[derive(Debug, Clone)]
pub struct GraphQLConfig {
    /// GraphQL endpoint path
    pub endpoint: String,

    /// Enable GraphQL Playground (interactive GraphQL IDE)
    pub enable_playground: bool,

    /// Playground endpoint path (if enabled)
    pub playground_endpoint: String,

    /// Enable GraphiQL (lighter alternative to Playground)
    pub enable_graphiql: bool,

    /// GraphiQL endpoint path (if enabled)
    pub graphiql_endpoint: String,

    /// Enable schema documentation endpoint
    pub enable_schema_docs: bool,

    /// Schema documentation endpoint path
    pub schema_docs_endpoint: String,

    /// Enable introspection queries (required for playgrounds and docs)
    pub enable_introspection: bool,

    /// Maximum query depth (0 = unlimited)
    pub max_depth: usize,

    /// Maximum query complexity (0 = unlimited)
    pub max_complexity: usize,

    /// Enable query validation
    pub enable_validation: bool,

    /// Enable Apollo Tracing
    pub enable_tracing: bool,
}

impl GraphQLConfig {
    /// Create a new GraphQL configuration with defaults
    ///
    /// # Example
    ///
    /// ```
    /// use armature_graphql::GraphQLConfig;
    ///
    /// let config = GraphQLConfig::new("/graphql");
    /// assert_eq!(config.endpoint, "/graphql");
    /// assert!(config.enable_playground); // Enabled by default in development
    /// ```
    pub fn new(endpoint: impl Into<String>) -> Self {
        let endpoint = endpoint.into();
        Self {
            playground_endpoint: format!("{}/playground", endpoint),
            graphiql_endpoint: format!("{}/graphiql", endpoint),
            schema_docs_endpoint: format!("{}/schema", endpoint),
            endpoint,
            enable_playground: true,
            enable_graphiql: false,
            enable_schema_docs: true,
            enable_introspection: true,
            max_depth: 0,
            max_complexity: 0,
            enable_validation: true,
            enable_tracing: false,
        }
    }

    /// Create a production configuration (playgrounds disabled)
    ///
    /// # Example
    ///
    /// ```
    /// use armature_graphql::GraphQLConfig;
    ///
    /// let config = GraphQLConfig::production("/graphql");
    /// assert!(!config.enable_playground);
    /// assert!(!config.enable_graphiql);
    /// assert!(!config.enable_introspection); // Disabled for security
    /// assert_eq!(config.max_depth, 15); // DoS protection enabled by default
    /// assert_eq!(config.max_complexity, 1000); // DoS protection enabled by default
    /// ```
    pub fn production(endpoint: impl Into<String>) -> Self {
        let mut config = Self::new(endpoint);
        config.enable_playground = false;
        config.enable_graphiql = false;
        config.enable_introspection = false;
        config.enable_schema_docs = false; // Can be enabled separately if needed
        // `new()` defaults max_depth/max_complexity to 0 (unlimited), which
        // leaves the server open to deeply nested or combinatorially
        // expensive queries. Production deployments need real limits, not
        // just introspection disabled; these are conservative starting
        // points (see `configure()`/`limit_depth`/`limit_complexity`) that
        // callers can override with `.with_max_depth()`/`.with_max_complexity()`.
        config.max_depth = 15;
        config.max_complexity = 1000;
        config
    }

    /// Create a development configuration (all features enabled)
    ///
    /// # Example
    ///
    /// ```
    /// use armature_graphql::GraphQLConfig;
    ///
    /// let config = GraphQLConfig::development("/graphql");
    /// assert!(config.enable_playground);
    /// assert!(config.enable_graphiql);
    /// assert!(config.enable_schema_docs);
    /// assert!(config.enable_introspection);
    /// ```
    pub fn development(endpoint: impl Into<String>) -> Self {
        let mut config = Self::new(endpoint);
        config.enable_playground = true;
        config.enable_graphiql = true;
        config.enable_schema_docs = true;
        config.enable_introspection = true;
        config.enable_tracing = true;
        config
    }

    /// Enable or disable GraphQL Playground
    pub fn with_playground(mut self, enable: bool) -> Self {
        self.enable_playground = enable;
        self
    }

    /// Set custom playground endpoint
    pub fn with_playground_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.playground_endpoint = endpoint.into();
        self
    }

    /// Enable or disable GraphiQL
    pub fn with_graphiql(mut self, enable: bool) -> Self {
        self.enable_graphiql = enable;
        self
    }

    /// Set custom GraphiQL endpoint
    pub fn with_graphiql_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.graphiql_endpoint = endpoint.into();
        self
    }

    /// Enable or disable schema documentation endpoint
    pub fn with_schema_docs(mut self, enable: bool) -> Self {
        self.enable_schema_docs = enable;
        self
    }

    /// Set custom schema documentation endpoint
    pub fn with_schema_docs_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.schema_docs_endpoint = endpoint.into();
        self
    }

    /// Enable or disable introspection queries
    pub fn with_introspection(mut self, enable: bool) -> Self {
        self.enable_introspection = enable;
        self
    }

    /// Set maximum query depth
    pub fn with_max_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth;
        self
    }

    /// Set maximum query complexity
    pub fn with_max_complexity(mut self, complexity: usize) -> Self {
        self.max_complexity = complexity;
        self
    }

    /// Enable or disable query validation
    pub fn with_validation(mut self, enable: bool) -> Self {
        self.enable_validation = enable;
        self
    }

    /// Enable or disable Apollo Tracing
    pub fn with_tracing(mut self, enable: bool) -> Self {
        self.enable_tracing = enable;
        self
    }

    /// Apply this configuration's security/behavior knobs onto an
    /// `async-graphql` [`SchemaBuilder`](async_graphql::SchemaBuilder).
    ///
    /// This is the only place these knobs take effect — building a schema
    /// via a bare `Schema::build(...).finish()` silently ignores every
    /// field on [`GraphQLConfig`]. In particular, [`GraphQLConfig::production`]
    /// documents introspection as "Disabled for security", but that only
    /// becomes true once the config is threaded through this method.
    ///
    /// Applies, in order:
    /// - `.limit_depth(max_depth)` when `max_depth != 0`
    /// - `.limit_complexity(max_complexity)` when `max_complexity != 0`
    /// - `.disable_introspection()` when `!enable_introspection`
    /// - `.validation_mode(ValidationMode::Fast)` and `.disable_suggestions()`
    ///   when `!enable_validation` (async-graphql has no way to fully turn
    ///   validation off; this is the closest approximation it exposes —
    ///   faster/looser validation and no field-name suggestions, which also
    ///   avoids leaking schema shape hints in error messages)
    /// - `.extension(ApolloTracing)` when `enable_tracing`
    ///
    /// # Example
    ///
    /// ```
    /// use armature_graphql::GraphQLConfig;
    /// use async_graphql::{EmptyMutation, EmptySubscription, Object, Schema};
    ///
    /// struct Query;
    ///
    /// #[Object]
    /// impl Query {
    ///     async fn hello(&self) -> &str {
    ///         "hi"
    ///     }
    /// }
    ///
    /// let config = GraphQLConfig::production("/graphql");
    /// let schema = config
    ///     .configure(Schema::build(Query, EmptyMutation, EmptySubscription))
    ///     .finish();
    /// ```
    pub fn configure<Q, M, S>(
        &self,
        mut builder: async_graphql::SchemaBuilder<Q, M, S>,
    ) -> async_graphql::SchemaBuilder<Q, M, S> {
        if self.max_depth > 0 {
            builder = builder.limit_depth(self.max_depth);
        }

        if self.max_complexity > 0 {
            builder = builder.limit_complexity(self.max_complexity);
        }

        if !self.enable_introspection {
            builder = builder.disable_introspection();
        }

        if !self.enable_validation {
            builder = builder
                .validation_mode(async_graphql::ValidationMode::Fast)
                .disable_suggestions();
        }

        if self.enable_tracing {
            builder = builder.extension(async_graphql::extensions::ApolloTracing);
        }

        builder
    }
}

impl Default for GraphQLConfig {
    fn default() -> Self {
        Self::new("/graphql")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = GraphQLConfig::new("/graphql");
        assert_eq!(config.endpoint, "/graphql");
        assert!(config.enable_playground);
        assert!(config.enable_introspection);
        assert!(config.enable_schema_docs);
    }

    #[test]
    fn test_production_config() {
        let config = GraphQLConfig::production("/api/graphql");
        assert_eq!(config.endpoint, "/api/graphql");
        assert!(!config.enable_playground);
        assert!(!config.enable_graphiql);
        assert!(!config.enable_introspection);
        assert!(!config.enable_schema_docs);
    }

    #[test]
    fn test_production_config_enforces_dos_protection() {
        // `production()` disables introspection but must also set non-zero
        // max_depth/max_complexity, otherwise the DoS-protection half of
        // the "production" preset is inert (0 == unlimited, see `configure()`).
        let config = GraphQLConfig::production("/graphql");
        assert!(config.max_depth > 0, "production() must cap query depth");
        assert!(
            config.max_complexity > 0,
            "production() must cap query complexity"
        );
    }

    #[test]
    fn test_development_config() {
        let config = GraphQLConfig::development("/dev/graphql");
        assert_eq!(config.endpoint, "/dev/graphql");
        assert!(config.enable_playground);
        assert!(config.enable_graphiql);
        assert!(config.enable_schema_docs);
        assert!(config.enable_introspection);
        assert!(config.enable_tracing);
    }

    #[test]
    fn test_builder_pattern() {
        let config = GraphQLConfig::new("/graphql")
            .with_playground(false)
            .with_graphiql(true)
            .with_schema_docs(true)
            .with_max_depth(10)
            .with_max_complexity(100);

        assert!(!config.enable_playground);
        assert!(config.enable_graphiql);
        assert!(config.enable_schema_docs);
        assert_eq!(config.max_depth, 10);
        assert_eq!(config.max_complexity, 100);
    }

    #[test]
    fn test_custom_endpoints() {
        let config = GraphQLConfig::new("/api")
            .with_playground_endpoint("/api/play")
            .with_graphiql_endpoint("/api/iql")
            .with_schema_docs_endpoint("/api/docs");

        assert_eq!(config.playground_endpoint, "/api/play");
        assert_eq!(config.graphiql_endpoint, "/api/iql");
        assert_eq!(config.schema_docs_endpoint, "/api/docs");
    }

    // -------------------------------------------------------------------
    // GraphQLConfig::configure — regression tests.
    //
    // Prior to this fix, `configure` did not exist and nothing in the
    // crate ever called `.disable_introspection()`/`.limit_depth()` on
    // the async-graphql `SchemaBuilder`. A schema built from
    // `GraphQLConfig::production()` was therefore fully introspectable
    // and depth-unlimited despite the config's own docs claiming
    // otherwise. These tests fail against that prior behavior.
    // -------------------------------------------------------------------

    struct IntrospectionTestQuery;

    #[async_graphql::Object]
    impl IntrospectionTestQuery {
        async fn hello(&self) -> &str {
            "hi"
        }
    }

    #[test]
    fn test_configure_disables_introspection_for_production() {
        use async_graphql::{EmptyMutation, EmptySubscription, Schema};

        let config = GraphQLConfig::production("/graphql");
        let schema = config
            .configure(Schema::build(
                IntrospectionTestQuery,
                EmptyMutation,
                EmptySubscription,
            ))
            .finish();

        let response = tokio_test::block_on(schema.execute("{ __schema { types { name } } }"));

        // With introspection disabled, async-graphql resolves `__schema` to
        // `null` rather than erroring — so the regression check is that the
        // introspection data itself is absent, not that the query errors.
        let data = response.data.into_json().unwrap();
        assert!(
            data["__schema"].is_null(),
            "introspection query should return no schema data when GraphQLConfig::production() \
             is applied via configure(), but it returned: {:?}",
            data
        );
    }

    #[test]
    fn test_configure_allows_introspection_by_default() {
        use async_graphql::{EmptyMutation, EmptySubscription, Schema};

        let config = GraphQLConfig::new("/graphql");
        let schema = config
            .configure(Schema::build(
                IntrospectionTestQuery,
                EmptyMutation,
                EmptySubscription,
            ))
            .finish();

        let response = tokio_test::block_on(schema.execute("{ __schema { types { name } } }"));

        assert!(
            response.errors.is_empty(),
            "default config should still allow introspection: {:?}",
            response.errors
        );

        let data = response.data.into_json().unwrap();
        assert!(
            !data["__schema"].is_null(),
            "default config should return real introspection data"
        );
    }

    struct Nested3;

    #[async_graphql::Object]
    impl Nested3 {
        async fn value(&self) -> i32 {
            3
        }
    }

    struct Nested2;

    #[async_graphql::Object]
    impl Nested2 {
        async fn nested3(&self) -> Nested3 {
            Nested3
        }
    }

    struct DepthTestQuery;

    #[async_graphql::Object]
    impl DepthTestQuery {
        async fn nested2(&self) -> Nested2 {
            Nested2
        }
    }

    #[test]
    fn test_configure_enforces_max_depth() {
        use async_graphql::{EmptyMutation, EmptySubscription, Schema};

        let config = GraphQLConfig::new("/graphql").with_max_depth(1);
        let schema = config
            .configure(Schema::build(
                DepthTestQuery,
                EmptyMutation,
                EmptySubscription,
            ))
            .finish();

        // Query depth here is 3: nested2 -> nested3 -> value
        let response = tokio_test::block_on(schema.execute("{ nested2 { nested3 { value } } }"));

        assert!(
            !response.errors.is_empty(),
            "query exceeding max_depth should be rejected when configured via configure(), \
             but it succeeded: {:?}",
            response.data
        );
    }

    #[test]
    fn test_configure_is_noop_when_limits_unset() {
        use async_graphql::{EmptyMutation, EmptySubscription, Schema};

        let config = GraphQLConfig::new("/graphql"); // max_depth: 0 (unlimited)
        let schema = config
            .configure(Schema::build(
                DepthTestQuery,
                EmptyMutation,
                EmptySubscription,
            ))
            .finish();

        let response = tokio_test::block_on(schema.execute("{ nested2 { nested3 { value } } }"));

        assert!(
            response.errors.is_empty(),
            "unlimited depth config should not reject a depth-3 query: {:?}",
            response.errors
        );
    }
}
