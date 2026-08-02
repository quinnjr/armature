// GraphQL resolver traits and utilities

use async_graphql::{Context, Result};
use std::any::Any;

/// Trait for GraphQL resolvers with DI support
pub trait Resolver: Send + Sync {
    fn as_any(&self) -> &dyn Any;
}

/// Optional marker trait for manually categorizing a type as a query resolver.
///
/// This is a purely conventional, opt-in trait for callers who want to
/// explicitly tag their resolver types. Nothing in this crate — including
/// the `#[resolver]` macro (`armature-graphql-macros`) — generates
/// implementations of it or consumes it automatically; implementing it has
/// no effect beyond documenting intent.
pub trait QueryResolver: Resolver {
    fn type_name() -> &'static str
    where
        Self: Sized;
}

/// Optional marker trait for manually categorizing a type as a mutation
/// resolver.
///
/// This is a purely conventional, opt-in trait for callers who want to
/// explicitly tag their resolver types. Nothing in this crate — including
/// the `#[resolver]` macro (`armature-graphql-macros`) — generates
/// implementations of it or consumes it automatically; implementing it has
/// no effect beyond documenting intent.
pub trait MutationResolver: Resolver {
    fn type_name() -> &'static str
    where
        Self: Sized;
}

/// Optional marker trait for manually categorizing a type as a subscription
/// resolver.
///
/// This is a purely conventional, opt-in trait for callers who want to
/// explicitly tag their resolver types. Nothing in this crate — including
/// the `#[resolver]` macro (`armature-graphql-macros`) — generates
/// implementations of it or consumes it automatically; implementing it has
/// no effect beyond documenting intent.
pub trait SubscriptionResolver: Resolver {
    fn type_name() -> &'static str
    where
        Self: Sized;
}

/// Context extension for accessing DI services
pub trait ContextExt {
    /// Get a service from the context
    fn get_service<T: Send + Sync + 'static>(&self) -> Result<&T>;
}

impl<'a> ContextExt for Context<'a> {
    fn get_service<T: Send + Sync + 'static>(&self) -> Result<&T> {
        self.data::<T>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_graphql::{EmptyMutation, EmptySubscription, Schema};

    /// A service that is registered in the schema's context data, used to
    /// exercise the found (`Ok`) path of `ContextExt::get_service`.
    struct GreeterService {
        greeting: String,
    }

    /// A service type that is deliberately never registered, used to
    /// exercise the not-found (`Err`) path of `ContextExt::get_service`.
    struct UnregisteredService;

    struct TestQuery;

    #[async_graphql::Object]
    impl TestQuery {
        /// Resolves via a service that *is* present in the schema data.
        async fn greeting(&self, ctx: &Context<'_>) -> Result<String> {
            let service = ctx.get_service::<GreeterService>()?;
            Ok(service.greeting.clone())
        }

        /// Attempts to resolve a service that was never registered; expected
        /// to surface as a GraphQL error rather than panic.
        async fn missing(&self, ctx: &Context<'_>) -> Result<String> {
            let _service = ctx.get_service::<UnregisteredService>()?;
            Ok("unreachable".to_string())
        }
    }

    fn build_schema() -> Schema<TestQuery, EmptyMutation, EmptySubscription> {
        Schema::build(TestQuery, EmptyMutation, EmptySubscription)
            .data(GreeterService {
                greeting: "hello from DI".to_string(),
            })
            .finish()
    }

    #[test]
    fn test_context_ext_get_service_found() {
        let schema = build_schema();

        let response = tokio_test::block_on(schema.execute("{ greeting }"));

        assert!(
            response.errors.is_empty(),
            "expected no errors, got: {:?}",
            response.errors
        );

        let data = response.data.into_json().unwrap();
        assert_eq!(data["greeting"], "hello from DI");
    }

    #[test]
    fn test_context_ext_get_service_not_found() {
        let schema = build_schema();

        let response = tokio_test::block_on(schema.execute("{ missing }"));

        assert!(
            !response.errors.is_empty(),
            "expected an error for an unregistered service, got none"
        );
    }
}
