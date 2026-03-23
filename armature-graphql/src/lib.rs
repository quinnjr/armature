// GraphQL support for Armature framework

pub mod config;
pub mod decorators;
pub mod federation;
pub mod resolver;
pub mod schema_builder;
pub mod schema_docs;

pub use async_graphql;
pub use async_graphql::{
    Context, EmptyMutation, EmptySubscription, Enum, Error, ID, InputObject, MergedObject,
    MergedSubscription, Object, Result, Schema, SimpleObject, Subscription, Union,
};

pub use config::*;
pub use decorators::*;
pub use resolver::*;
pub use schema_builder::*;
pub use schema_docs::*;

use armature_core::Error as ArmatureError;
use armature_log::info;
use std::sync::Arc;

/// GraphQL schema wrapper
pub struct GraphQLSchema<Query, Mutation, Subscription> {
    schema: Arc<Schema<Query, Mutation, Subscription>>,
}

impl<Query, Mutation, Subscription> GraphQLSchema<Query, Mutation, Subscription>
where
    Query: async_graphql::ObjectType + 'static,
    Mutation: async_graphql::ObjectType + 'static,
    Subscription: async_graphql::SubscriptionType + 'static,
{
    pub fn new(schema: Schema<Query, Mutation, Subscription>) -> Self {
        info!("Initializing GraphQL schema");
        Self {
            schema: Arc::new(schema),
        }
    }

    pub fn schema(&self) -> Arc<Schema<Query, Mutation, Subscription>> {
        self.schema.clone()
    }
}

impl<Query, Mutation, Subscription> Clone for GraphQLSchema<Query, Mutation, Subscription> {
    fn clone(&self) -> Self {
        Self {
            schema: self.schema.clone(),
        }
    }
}

// Provider is automatically implemented via blanket impl
// when Query, Mutation, Subscription all satisfy Send + Sync + 'static

/// GraphQL request handling
pub struct GraphQLRequest {
    pub query: String,
    pub variables: Option<serde_json::Value>,
    pub operation_name: Option<String>,
}

impl GraphQLRequest {
    pub fn new(query: String) -> Self {
        Self {
            query,
            variables: None,
            operation_name: None,
        }
    }

    pub fn with_variables(mut self, variables: serde_json::Value) -> Self {
        self.variables = Some(variables);
        self
    }

    pub fn with_operation(mut self, operation_name: String) -> Self {
        self.operation_name = Some(operation_name);
        self
    }
}

/// GraphQL response
pub struct GraphQLResponse {
    pub data: Option<serde_json::Value>,
    pub errors: Vec<String>,
}

impl GraphQLResponse {
    pub fn success(data: serde_json::Value) -> Self {
        Self {
            data: Some(data),
            errors: Vec::new(),
        }
    }

    pub fn error(message: String) -> Self {
        Self {
            data: None,
            errors: vec![message],
        }
    }

    pub fn to_json(&self) -> Result<String, ArmatureError> {
        serde_json::to_string(self).map_err(|e| ArmatureError::Serialization(e.to_string()))
    }
}

impl serde::Serialize for GraphQLResponse {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;

        if let Some(ref data) = self.data {
            map.serialize_entry("data", data)?;
        } else {
            map.serialize_entry("data", &None::<()>)?;
        }

        if !self.errors.is_empty() {
            map.serialize_entry("errors", &self.errors)?;
        }

        map.end()
    }
}

/// Helper to build GraphQL schemas with DI integration
pub struct SchemaBuilder<Query, Mutation, Subscription> {
    query: Option<Query>,
    mutation: Option<Mutation>,
    subscription: Option<Subscription>,
}

impl<Query, Mutation, Subscription> SchemaBuilder<Query, Mutation, Subscription>
where
    Query: async_graphql::ObjectType + 'static,
    Mutation: async_graphql::ObjectType + 'static,
    Subscription: async_graphql::SubscriptionType + 'static,
{
    pub fn new() -> Self {
        Self {
            query: None,
            mutation: None,
            subscription: None,
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

    pub fn build(self) -> Result<Schema<Query, Mutation, Subscription>, ArmatureError> {
        let query = self
            .query
            .ok_or_else(|| ArmatureError::Internal("Query root is required".to_string()))?;
        let mutation = self
            .mutation
            .ok_or_else(|| ArmatureError::Internal("Mutation root is required".to_string()))?;
        let subscription = self
            .subscription
            .ok_or_else(|| ArmatureError::Internal("Subscription root is required".to_string()))?;

        Ok(Schema::build(query, mutation, subscription).finish())
    }
}

impl<Query, Mutation, Subscription> Default for SchemaBuilder<Query, Mutation, Subscription>
where
    Query: async_graphql::ObjectType + 'static,
    Mutation: async_graphql::ObjectType + 'static,
    Subscription: async_graphql::SubscriptionType + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

/// GraphQL playground HTML
///
/// # Example
///
/// ```
/// use armature_graphql::graphql_playground_html;
///
/// let html = graphql_playground_html("/graphql");
/// assert!(html.contains("GraphQL Playground"));
/// ```
pub fn graphql_playground_html(endpoint: &str) -> String {
    format!(
        r#"
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>GraphQL Playground</title>
    <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/graphql-playground-react/build/static/css/index.css" />
    <script src="https://cdn.jsdelivr.net/npm/graphql-playground-react/build/static/js/middleware.js"></script>
</head>
<body>
    <div id="root"></div>
    <script>
        window.addEventListener('load', function (event) {{
            GraphQLPlayground.init(document.getElementById('root'), {{
                endpoint: '{}'
            }})
        }})
    </script>
</body>
</html>
"#,
        endpoint
    )
}

/// GraphiQL HTML (lighter alternative)
pub fn graphiql_html(endpoint: &str) -> String {
    format!(
        r#"
<!DOCTYPE html>
<html>
<head>
    <title>GraphiQL</title>
    <style>
        body {{
            height: 100vh;
            margin: 0;
            width: 100%;
            overflow: hidden;
        }}
        #graphiql {{
            height: 100vh;
        }}
    </style>
    <script
        crossorigin
        src="https://unpkg.com/react@18/umd/react.production.min.js"
    ></script>
    <script
        crossorigin
        src="https://unpkg.com/react-dom@18/umd/react-dom.production.min.js"
    ></script>
    <link rel="stylesheet" href="https://unpkg.com/graphiql/graphiql.min.css" />
</head>
<body>
    <div id="graphiql">Loading...</div>
    <script
        src="https://unpkg.com/graphiql/graphiql.min.js"
        type="application/javascript"
    ></script>
    <script>
        const fetcher = GraphiQL.createFetcher({{
            url: '{}',
        }});

        ReactDOM.render(
            React.createElement(GraphiQL, {{ fetcher: fetcher }}),
            document.getElementById('graphiql'),
        );
    </script>
</body>
</html>
"#,
        endpoint
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graphql_request_builder() {
        let req = GraphQLRequest::new("query { hello }".to_string())
            .with_variables(serde_json::json!({"name": "World"}))
            .with_operation("HelloQuery".to_string());

        assert_eq!(req.query, "query { hello }");
        assert!(req.variables.is_some());
        assert_eq!(req.operation_name, Some("HelloQuery".to_string()));
    }

    #[test]
    fn test_graphql_response_serialization() {
        let response = GraphQLResponse::success(serde_json::json!({
            "hello": "world"
        }));

        let json = response.to_json().unwrap();
        assert!(json.contains("hello"));
        assert!(json.contains("world"));
    }
}
