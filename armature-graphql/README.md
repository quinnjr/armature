# armature-graphql

GraphQL support for the Armature framework.

## Features

- **Schema Definition** - Type-safe GraphQL schemas
- **Resolvers** - Async resolver functions
- **Subscriptions** - Real-time updates via WebSocket
- **Playground** - Built-in GraphQL IDE
- **Validation** - Query validation and depth limiting

## Installation

```toml
[dependencies]
armature-graphql = "0.1"
```

## Quick Start

`armature_graphql` re-exports `async_graphql`, so the resolver macros
(`#[async_graphql::Object]`, `Object`, `SimpleObject`, …) come from this crate
directly. Define your resolver roots, then assemble a schema with
`ProgrammaticSchemaBuilder` (DI-aware) or the plain `SchemaBuilder`.

```rust
use armature_graphql::{
    async_graphql, graphql_playground_html, EmptyMutation, EmptySubscription,
    ProgrammaticSchemaBuilder,
};

struct QueryRoot;

#[async_graphql::Object]
impl QueryRoot {
    async fn hello(&self) -> &str {
        "Hello, World!"
    }
}

#[tokio::main]
async fn main() {
    // Build a schema programmatically. `.add_service(..)` injects values into
    // the resolver `Context`; `.config(GraphQLConfig::default())` applies the
    // introspection / depth / complexity limits.
    let schema = ProgrammaticSchemaBuilder::new()
        .query(QueryRoot)
        .mutation(EmptyMutation)
        .subscription(EmptySubscription)
        .build();

    // Execute a query directly against the schema.
    let response = schema.execute("{ hello }").await;
    println!("{}", serde_json::to_string(&response).unwrap());

    // Serve the built-in Playground IDE pointed at your `/graphql` endpoint.
    let _playground = graphql_playground_html("/graphql");
    // (Wire `schema` and `_playground` into your Armature HTTP routes.)
}
```

To wrap a finished schema for sharing across handlers, use
`GraphQLSchema::new(schema)` and clone the `Arc` it holds via `.schema()`.

## Subscriptions

```rust
use armature_graphql::async_graphql;
use async_graphql::futures_util::{stream, Stream};

struct SubscriptionRoot;

#[async_graphql::Subscription]
impl SubscriptionRoot {
    async fn messages(&self) -> impl Stream<Item = String> {
        // Return a stream of messages.
        stream::iter(vec!["hello".to_string()])
    }
}
```

## License

MIT OR Apache-2.0

