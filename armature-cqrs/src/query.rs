//! Query handling for CQRS

use async_trait::async_trait;
use dashmap::DashMap;
use std::any::{Any, TypeId};
use std::sync::Arc;
use thiserror::Error;

/// Query trait
///
/// Queries represent read operations in CQRS.
pub trait Query: Send + Sync + 'static {
    /// Query result type
    type Result: Send;
}

/// Query handler trait
#[async_trait]
pub trait QueryHandler<Q: Query>: Send + Sync {
    /// Handle the query
    async fn handle(&self, query: Q) -> Result<Q::Result, QueryError>;
}

/// Query error
#[derive(Debug, Error)]
pub enum QueryError {
    #[error("Query execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Handler not found for query")]
    HandlerNotFound,

    #[error("Data not found: {0}")]
    NotFound(String),

    #[error("Invalid query parameters: {0}")]
    InvalidParameters(String),
}

/// Type-erased query handler
#[async_trait]
trait DynQueryHandler: Send + Sync {
    async fn handle_dyn(
        &self,
        query: Box<dyn Any + Send>,
    ) -> Result<Box<dyn Any + Send>, QueryError>;
}

/// Wrapper for typed query handlers
struct TypedQueryHandler<Q: Query, H: QueryHandler<Q>> {
    handler: H,
    _phantom: std::marker::PhantomData<Q>,
}

impl<Q: Query, H: QueryHandler<Q>> TypedQueryHandler<Q, H> {
    fn new(handler: H) -> Self {
        Self {
            handler,
            _phantom: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<Q: Query, H: QueryHandler<Q>> DynQueryHandler for TypedQueryHandler<Q, H> {
    async fn handle_dyn(
        &self,
        query: Box<dyn Any + Send>,
    ) -> Result<Box<dyn Any + Send>, QueryError> {
        match query.downcast::<Q>() {
            Ok(qry) => {
                let result = self.handler.handle(*qry).await?;
                Ok(Box::new(result))
            }
            Err(_) => Err(QueryError::ExecutionFailed("Type mismatch".to_string())),
        }
    }
}

/// Query bus
pub struct QueryBus {
    handlers: DashMap<TypeId, Arc<dyn DynQueryHandler>>,
}

impl QueryBus {
    /// Create new query bus
    pub fn new() -> Self {
        Self {
            handlers: DashMap::new(),
        }
    }

    /// Register a query handler
    pub fn register<Q, H>(&self, handler: H)
    where
        Q: Query,
        H: QueryHandler<Q> + 'static,
    {
        let type_id = TypeId::of::<Q>();
        let handler = Arc::new(TypedQueryHandler::new(handler));
        self.handlers.insert(type_id, handler);
    }

    /// Execute a query
    pub async fn execute<Q>(&self, query: Q) -> Result<Q::Result, QueryError>
    where
        Q: Query,
    {
        let type_id = TypeId::of::<Q>();

        // Clone the handler out and drop the map guard before awaiting. A
        // `Ref` from `DashMap::get` holds that shard's read lock, so holding it
        // across the handler future would block any concurrent `register` that
        // lands on the same shard for the whole duration of the handler — and a
        // handler that re-entrantly calls `register` would deadlock on itself.
        let handler = {
            let entry = self
                .handlers
                .get(&type_id)
                .ok_or(QueryError::HandlerNotFound)?;
            Arc::clone(entry.value())
        };

        let boxed_query: Box<dyn Any + Send> = Box::new(query);
        let result = handler.handle_dyn(boxed_query).await?;

        match result.downcast::<Q::Result>() {
            Ok(result) => Ok(*result),
            Err(_) => Err(QueryError::ExecutionFailed(
                "Result type mismatch".to_string(),
            )),
        }
    }
}

impl Default for QueryBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    struct GetUserQuery {
        user_id: String,
    }

    impl Query for GetUserQuery {
        type Result = User;
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct User {
        id: String,
        email: String,
    }

    struct GetUserHandler;

    #[async_trait]
    impl QueryHandler<GetUserQuery> for GetUserHandler {
        async fn handle(&self, query: GetUserQuery) -> Result<User, QueryError> {
            Ok(User {
                id: query.user_id,
                email: "alice@example.com".to_string(),
            })
        }
    }

    #[tokio::test]
    async fn test_query_bus() {
        let bus = QueryBus::new();
        bus.register::<GetUserQuery, _>(GetUserHandler);

        let query = GetUserQuery {
            user_id: "user-123".to_string(),
        };

        let result = bus.execute(query).await.unwrap();
        assert_eq!(result.id, "user-123");
        assert_eq!(result.email, "alice@example.com");
    }

    #[tokio::test]
    async fn test_query_bus_handler_not_found() {
        let bus = QueryBus::new();

        let query = GetUserQuery {
            user_id: "user-456".to_string(),
        };

        let err = bus.execute(query).await.unwrap_err();
        assert!(matches!(err, QueryError::HandlerNotFound));
    }

    #[derive(Serialize, Deserialize)]
    struct CountUsersQuery;

    impl Query for CountUsersQuery {
        type Result = usize;
    }

    struct CountUsersHandler;

    #[async_trait]
    impl QueryHandler<CountUsersQuery> for CountUsersHandler {
        async fn handle(&self, _query: CountUsersQuery) -> Result<usize, QueryError> {
            Ok(7)
        }
    }

    #[tokio::test]
    async fn test_query_bus_independent_types_do_not_collide() {
        let bus = QueryBus::new();
        bus.register::<GetUserQuery, _>(GetUserHandler);
        bus.register::<CountUsersQuery, _>(CountUsersHandler);

        let user = bus
            .execute(GetUserQuery {
                user_id: "user-789".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(user.id, "user-789");

        let count = bus.execute(CountUsersQuery).await.unwrap();
        assert_eq!(count, 7);
    }

    struct ReentrantRegisterHandler {
        bus: Arc<std::sync::OnceLock<Arc<QueryBus>>>,
    }

    #[async_trait]
    impl QueryHandler<CountUsersQuery> for ReentrantRegisterHandler {
        async fn handle(&self, _query: CountUsersQuery) -> Result<usize, QueryError> {
            // Registering under the *same* query type guarantees the same
            // DashMap shard as the entry `execute` looked up. If `execute` were
            // still holding that shard's read guard, this write would block
            // forever.
            let bus = self.bus.get().expect("bus wired before execute").clone();
            bus.register::<CountUsersQuery, _>(CountUsersHandler);
            Ok(3)
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_handler_can_register_while_running() {
        let cell = Arc::new(std::sync::OnceLock::new());
        let bus = Arc::new(QueryBus::new());
        // The cell was just created, so this always succeeds; the error variant
        // carries the un-stored `Arc` and is not `Debug`.
        let _ = cell.set(bus.clone());

        bus.register::<CountUsersQuery, _>(ReentrantRegisterHandler { bus: cell });

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            bus.execute(CountUsersQuery),
        )
        .await
        .expect("execute must not hold a map guard across the handler await");

        assert_eq!(result.unwrap(), 3);
    }
}
