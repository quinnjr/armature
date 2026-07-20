// Programmatic schema builder

use crate::config::GraphQLConfig;
use async_graphql::Schema;

/// Programmatic schema builder with DI integration
pub struct ProgrammaticSchemaBuilder<Q, M, S> {
    query: Option<Q>,
    mutation: Option<M>,
    subscription: Option<S>,
    services: Vec<Box<dyn std::any::Any + Send + Sync>>,
    config: Option<GraphQLConfig>,
}

impl<Q, M, S> ProgrammaticSchemaBuilder<Q, M, S>
where
    Q: async_graphql::ObjectType + 'static,
    M: async_graphql::ObjectType + 'static,
    S: async_graphql::SubscriptionType + 'static,
{
    /// Create a new schema builder
    pub fn new() -> Self {
        Self {
            query: None,
            mutation: None,
            subscription: None,
            services: Vec::new(),
            config: None,
        }
    }

    /// Set the query root
    pub fn query(mut self, query: Q) -> Self {
        self.query = Some(query);
        self
    }

    /// Set the mutation root
    pub fn mutation(mut self, mutation: M) -> Self {
        self.mutation = Some(mutation);
        self
    }

    /// Set the subscription root
    pub fn subscription(mut self, subscription: S) -> Self {
        self.subscription = Some(subscription);
        self
    }

    /// Add a service to the schema context
    pub fn add_service<T: Send + Sync + 'static>(mut self, service: T) -> Self {
        self.services.push(Box::new(service));
        self
    }

    /// Attach a [`GraphQLConfig`] whose security/behavior knobs (introspection,
    /// max depth, max complexity, validation, tracing) will be applied to the
    /// built schema. Without this, the schema is built unrestricted.
    pub fn config(mut self, config: GraphQLConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Build the schema
    pub fn build(self) -> Schema<Q, M, S> {
        let query = self.query.expect("Query root is required");
        let mutation = self.mutation.expect("Mutation root is required");
        let subscription = self.subscription.expect("Subscription root is required");

        let mut schema_builder = Schema::build(query, mutation, subscription);

        // Add all services to the schema data
        for service in self.services {
            schema_builder = schema_builder.data(service);
        }

        if let Some(config) = &self.config {
            schema_builder = config.configure(schema_builder);
        }

        schema_builder.finish()
    }
}

impl<Q, M, S> Default for ProgrammaticSchemaBuilder<Q, M, S>
where
    Q: async_graphql::ObjectType + 'static,
    M: async_graphql::ObjectType + 'static,
    S: async_graphql::SubscriptionType + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

/// Helper to create an empty query root
#[derive(Default)]
pub struct EmptyQuery;

#[async_graphql::Object]
impl EmptyQuery {
    async fn _empty(&self) -> &str {
        ""
    }
}

/// Helper to create an empty mutation root
#[derive(Default)]
pub struct EmptyMutation;

#[async_graphql::Object]
impl EmptyMutation {
    async fn _empty(&self) -> &str {
        ""
    }
}

/// Helper to create an empty subscription root
#[derive(Default)]
pub struct EmptySubscription;

#[async_graphql::Subscription]
impl EmptySubscription {
    async fn _empty(&self) -> impl async_graphql::futures_util::Stream<Item = &str> {
        async_graphql::futures_util::stream::empty()
    }
}

/// Macro to merge multiple resolver objects into a single root
#[macro_export]
macro_rules! merge_resolvers {
    ($($resolver:ty),+ $(,)?) => {
        {
            use async_graphql::MergedObject;

            #[derive(MergedObject, Default)]
            struct MergedRoot($($resolver),+);

            MergedRoot::default()
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct TestQuery;

    #[async_graphql::Object]
    impl TestQuery {
        async fn test(&self) -> &str {
            "test"
        }
    }

    #[test]
    fn test_schema_builder() {
        let _schema = ProgrammaticSchemaBuilder::new()
            .query(TestQuery)
            .mutation(EmptyMutation)
            .subscription(EmptySubscription)
            .build();
    }

    // Finding 2 regression: `create_merged_schema` used to silently ignore its
    // resolver arguments and return an unconfigured `ProgrammaticSchemaBuilder`
    // that panicked on `.build()`. It has been removed; the crate's real,
    // working merging mechanism is the `merge_resolvers!` macro (compile-time
    // `MergedObject` composition). This test proves that mechanism actually
    // resolves fields from multiple merged resolvers.
    #[derive(Default)]
    struct AlphaQuery;

    #[async_graphql::Object]
    impl AlphaQuery {
        async fn alpha(&self) -> &str {
            "alpha"
        }
    }

    #[derive(Default)]
    struct BetaQuery;

    #[async_graphql::Object]
    impl BetaQuery {
        async fn beta(&self) -> &str {
            "beta"
        }
    }

    #[test]
    fn test_merge_resolvers_macro_resolves_merged_fields() {
        let merged = merge_resolvers!(AlphaQuery, BetaQuery);

        let schema = ProgrammaticSchemaBuilder::new()
            .query(merged)
            .mutation(EmptyMutation)
            .subscription(EmptySubscription)
            .build();

        let response = tokio_test::block_on(schema.execute("{ alpha beta }"));

        assert!(
            response.errors.is_empty(),
            "merged query should resolve both fields without error: {:?}",
            response.errors
        );

        let data = response.data.into_json().unwrap();
        assert_eq!(data["alpha"], "alpha");
        assert_eq!(data["beta"], "beta");
    }
}
