//! GraphQL request builders.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use crate::{GraphQLClient, GraphQLError, GraphQLResponse, Result};

/// GraphQL request payload.
#[derive(Debug, Clone, Serialize)]
pub struct GraphQLRequest {
    /// The GraphQL query or mutation.
    pub query: String,
    /// Operation name (for documents with multiple operations).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_name: Option<String>,
    /// Variables for the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variables: Option<Value>,
    /// Extensions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Value>,
}

impl GraphQLRequest {
    /// Create a new request.
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            operation_name: None,
            variables: None,
            extensions: None,
        }
    }

    /// Set the operation name.
    pub fn operation_name(mut self, name: impl Into<String>) -> Self {
        self.operation_name = Some(name.into());
        self
    }

    /// Set variables.
    pub fn variables<T: Serialize>(mut self, variables: T) -> Self {
        self.variables = Some(serde_json::to_value(variables).unwrap_or_default());
        self
    }

    /// Set extensions.
    pub fn extensions(mut self, extensions: Value) -> Self {
        self.extensions = Some(extensions);
        self
    }
}

/// Query builder for GraphQL queries.
pub struct QueryBuilder<'a> {
    client: &'a GraphQLClient,
    request: GraphQLRequest,
    timeout: Option<Duration>,
    headers: Vec<(String, String)>,
    cacheable: bool,
}

impl<'a> QueryBuilder<'a> {
    /// Create a new query builder.
    pub(crate) fn new(client: &'a GraphQLClient, query: impl Into<String>) -> Self {
        Self {
            client,
            request: GraphQLRequest::new(query),
            timeout: None,
            headers: Vec::new(),
            cacheable: true,
        }
    }

    /// Mark whether this request is eligible for response caching.
    ///
    /// Queries are cacheable by default; mutations must never be cached.
    pub(crate) fn with_cacheable(mut self, cacheable: bool) -> Self {
        self.cacheable = cacheable;
        self
    }

    /// Set the operation name.
    pub fn operation_name(mut self, name: impl Into<String>) -> Self {
        self.request.operation_name = Some(name.into());
        self
    }

    /// Set variables.
    pub fn variables<T: Serialize>(mut self, variables: T) -> Self {
        self.request.variables = Some(serde_json::to_value(variables).unwrap_or_default());
        self
    }

    /// Set a single variable.
    pub fn variable(mut self, name: impl Into<String>, value: impl Serialize) -> Self {
        let vars = self
            .request
            .variables
            .get_or_insert_with(|| Value::Object(Default::default()));
        if let Value::Object(map) = vars {
            map.insert(name.into(), serde_json::to_value(value).unwrap_or_default());
        }
        self
    }

    /// Set extensions.
    pub fn extensions(mut self, extensions: Value) -> Self {
        self.request.extensions = Some(extensions);
        self
    }

    /// Set a custom timeout for this request.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Add a header for this request.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Execute the query and return the raw response.
    pub async fn send_raw(self) -> Result<GraphQLResponse<Value>> {
        self.client
            .execute_request_cached(self.request, self.headers, self.timeout, self.cacheable)
            .await
    }

    /// Execute the query and deserialize the response.
    pub async fn send<T: for<'de> Deserialize<'de>>(self) -> Result<T> {
        let response = self.send_raw().await?;
        response.into_result().and_then(|data| {
            serde_json::from_value(data).map_err(|e| GraphQLError::Parse(e.to_string()))
        })
    }
}

/// Mutation builder for GraphQL mutations.
pub struct MutationBuilder<'a> {
    inner: QueryBuilder<'a>,
}

impl<'a> MutationBuilder<'a> {
    /// Create a new mutation builder.
    pub(crate) fn new(client: &'a GraphQLClient, mutation: impl Into<String>) -> Self {
        Self {
            inner: QueryBuilder::new(client, mutation).with_cacheable(false),
        }
    }

    /// Set the operation name.
    pub fn operation_name(mut self, name: impl Into<String>) -> Self {
        self.inner = self.inner.operation_name(name);
        self
    }

    /// Set variables.
    pub fn variables<T: Serialize>(mut self, variables: T) -> Self {
        self.inner = self.inner.variables(variables);
        self
    }

    /// Set a single variable.
    pub fn variable(mut self, name: impl Into<String>, value: impl Serialize) -> Self {
        self.inner = self.inner.variable(name, value);
        self
    }

    /// Set a custom timeout for this request.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.inner = self.inner.timeout(timeout);
        self
    }

    /// Add a header for this request.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.inner = self.inner.header(name, value);
        self
    }

    /// Execute the mutation and return the raw response.
    pub async fn send_raw(self) -> Result<GraphQLResponse<Value>> {
        self.inner.send_raw().await
    }

    /// Execute the mutation and deserialize the response.
    pub async fn send<T: for<'de> Deserialize<'de>>(self) -> Result<T> {
        self.inner.send().await
    }
}

/// Subscription builder for GraphQL subscriptions.
pub struct SubscriptionBuilder<'a> {
    client: &'a GraphQLClient,
    request: GraphQLRequest,
    headers: Vec<(String, String)>,
}

impl<'a> SubscriptionBuilder<'a> {
    /// Create a new subscription builder.
    pub(crate) fn new(client: &'a GraphQLClient, subscription: impl Into<String>) -> Self {
        Self {
            client,
            request: GraphQLRequest::new(subscription),
            headers: Vec::new(),
        }
    }

    /// Set the operation name.
    pub fn operation_name(mut self, name: impl Into<String>) -> Self {
        self.request.operation_name = Some(name.into());
        self
    }

    /// Set variables.
    pub fn variables<T: Serialize>(mut self, variables: T) -> Self {
        self.request.variables = Some(serde_json::to_value(variables).unwrap_or_default());
        self
    }

    /// Add a header for the WebSocket connection.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Start the subscription.
    pub async fn send(self) -> Result<crate::SubscriptionStream<Value>> {
        self.client
            .execute_subscription(self.request, self.headers)
            .await
    }
}
