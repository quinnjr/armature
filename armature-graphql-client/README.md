# armature-graphql-client

GraphQL client with subscription support for the Armature framework.

## Features

- **Query/Mutation** - Execute GraphQL operations
- **Subscriptions** - Real-time updates via WebSocket
- **Batching** - Send multiple queries in a single request via `batch()`
- **Caching** - Optional response caching for queries

## Installation

```toml
[dependencies]
armature-graphql-client = "0.1"
```

## Quick Start

```rust,ignore
use armature_graphql_client::{GraphQLClient, GraphQLClientConfig};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct GetUserVars {
    id: String,
}

#[derive(Deserialize)]
struct User {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct GetUserResponse {
    user: User,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = GraphQLClient::new("https://api.example.com/graphql");

    let response: GetUserResponse = client
        .query("query GetUser($id: ID!) { user(id: $id) { id name } }")
        .variables(GetUserVars { id: "123".into() })
        .send()
        .await?;

    println!("User: {}", response.user.name);
    Ok(())
}
```

## Subscriptions

```rust,ignore
use armature_graphql_client::GraphQLClient;
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = GraphQLClient::new("wss://api.example.com/graphql");

    let mut subscription = client
        .subscribe("subscription { messageAdded { id content } }")
        .send()
        .await?;

    while let Some(result) = subscription.next().await {
        match result {
            Ok(data) => println!("Received: {:?}", data),
            Err(e) => eprintln!("Error: {}", e),
        }
    }

    Ok(())
}
```

## License

MIT OR Apache-2.0
