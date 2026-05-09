## GraphQL Configuration and Documentation

Armature's GraphQL module provides comprehensive configuration options for controlling playgrounds, documentation endpoints, and schema introspection.

---

## Table of Contents

- [Configuration](#configuration)
- [GraphQL Config Options](#graphql-config-options)
- [Development vs Production](#development-vs-production)
- [Schema Documentation](#schema-documentation)
- [Playground Options](#playground-options)
- [Security Considerations](#security-considerations)
- [Examples](#examples)

---

## Configuration

### Basic Configuration

```rust
use armature_graphql::GraphQLConfig;

// Default configuration (playgrounds enabled)
let config = GraphQLConfig::new("/graphql");

// Development configuration (all features enabled)
let config = GraphQLConfig::development("/graphql");

// Production configuration (playgrounds disabled)
let config = GraphQLConfig::production("/graphql");
```

---

## GraphQL Config Options

### Available Options

```rust
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
```

### Builder Pattern

```rust
let config = GraphQLConfig::new("/api/graphql")
    .with_playground(true)
    .with_graphiql(false)
    .with_schema_docs(true)
    .with_introspection(true)
    .with_max_depth(10)
    .with_max_complexity(100)
    .with_validation(true)
    .with_tracing(false);
```

---

## Development vs Production

### Development Configuration

Enable all features for the best developer experience:

```rust
let config = GraphQLConfig::development("/graphql");

// Equivalent to:
let config = GraphQLConfig::new("/graphql")
    .with_playground(true)
    .with_graphiql(true)
    .with_schema_docs(true)
    .with_introspection(true)
    .with_tracing(true);
```

**Features enabled:**
- ✅ GraphQL Playground
- ✅ GraphiQL
- ✅ Schema documentation
- ✅ Introspection queries
- ✅ Apollo tracing

### Production Configuration

Disable playgrounds and introspection for security:

```rust
let config = GraphQLConfig::production("/graphql");

// Equivalent to:
let config = GraphQLConfig::new("/graphql")
    .with_playground(false)
    .with_graphiql(false)
    .with_schema_docs(false)
    .with_introspection(false);
```

**Features disabled:**
- ❌ GraphQL Playground
- ❌ GraphiQL
- ❌ Schema documentation (can be enabled separately)
- ❌ Introspection queries

---

## Schema Documentation

### Documentation Endpoint

Armature generates beautiful, interactive schema documentation:

```rust
use armature_graphql::{generate_schema_docs_html, Schema};

let html = generate_schema_docs_html(
    &schema,
    "/graphql",      // GraphQL endpoint
    "My API"         // API title
);
```

### Features of Schema Documentation

1. **Interactive Schema Viewer**
   - Browse types, queries, mutations, subscriptions
   - Syntax highlighting for SDL
   - Copy schema to clipboard

2. **Getting Started Guide**
   - API endpoint information
   - Example queries
   - Integration instructions

3. **Example Queries**
   - Introspection queries
   - Type information queries
   - Common patterns

4. **Beautiful UI**
   - Modern, responsive design
   - Tabbed interface
   - Professional styling

### Accessing Documentation

Once enabled, access documentation at:
- **Schema Docs**: `http://localhost:3000/graphql/schema`
- **SDL Download**: `http://localhost:3000/graphql/schema.graphql`

---

## Playground Options

### GraphQL Playground

Full-featured GraphQL IDE with:
- Query editor with syntax highlighting
- Variable editor
- Response viewer
- Schema documentation sidebar
- Query history
- Multiple tabs

```rust
// Enable Playground
let config = GraphQLConfig::new("/graphql")
    .with_playground(true)
    .with_playground_endpoint("/graphql/playground");
```

**Access at**: `http://localhost:3000/graphql/playground`

### GraphiQL

Lighter alternative with:
- Query editor
- Variable support
- Schema explorer
- Query execution
- Faster load times

```rust
// Enable GraphiQL
let config = GraphQLConfig::new("/graphql")
    .with_graphiql(true)
    .with_graphiql_endpoint("/graphql/graphiql");
```

**Access at**: `http://localhost:3000/graphql/graphiql`

### Disabling Playgrounds

For production environments:

```rust
let config = GraphQLConfig::production("/graphql");
// Playgrounds are disabled by default
```

Or selectively disable:

```rust
let config = GraphQLConfig::new("/graphql")
    .with_playground(false)
    .with_graphiql(false);
```

---

## Security Considerations

### Introspection Queries

Introspection allows clients to query the GraphQL schema structure. While useful for development, it can expose your API structure in production.

**Recommendation**: Disable introspection in production:

```rust
let config = if cfg!(debug_assertions) {
    GraphQLConfig::development("/graphql")
} else {
    GraphQLConfig::production("/graphql")
};
```

### Query Complexity Limits

Prevent abuse with complexity limits:

```rust
let config = GraphQLConfig::new("/graphql")
    .with_max_depth(10)          // Limit query depth
    .with_max_complexity(100);   // Limit overall complexity
```

### Playground Access Control

Implement authentication for playgrounds in production:

```rust
#[get("/playground")]
async fn playground(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
    // Check authentication
    if !is_authenticated(&req) {
        return Err(Error::Unauthorized);
    }

    // Only allow in development or for authorized users
    if !cfg!(debug_assertions) && !is_admin(&req) {
        return Err(Error::Forbidden);
    }

    Ok(/* playground HTML */)
}
```

---

## Examples

### Example 1: Development Setup

```rust
use armature_graphql::GraphQLConfig;

let config = GraphQLConfig::development("/graphql");

// All features enabled for development
assert!(config.enable_playground);
assert!(config.enable_graphiql);
assert!(config.enable_schema_docs);
assert!(config.enable_introspection);
```

### Example 2: Production Setup

```rust
let config = GraphQLConfig::production("/graphql");

// Playgrounds disabled for security
assert!(!config.enable_playground);
assert!(!config.enable_graphiql);
assert!(!config.enable_introspection);
```

### Example 3: Custom Configuration

```rust
let config = GraphQLConfig::new("/api/v1/graphql")
    .with_playground(false)           // Disable Playground
    .with_graphiql(true)              // Enable GraphiQL
    .with_schema_docs(true)           // Enable documentation
    .with_introspection(true)         // Enable introspection
    .with_max_depth(15)               // Limit depth
    .with_max_complexity(200)         // Limit complexity
    .with_tracing(false);             // Disable tracing
```

### Example 4: Environment-Based Configuration

```rust
use std::env;

let config = match env::var("ENV").unwrap_or_default().as_str() {
    "production" => GraphQLConfig::production("/graphql"),
    "staging" => GraphQLConfig::new("/graphql")
        .with_playground(true)
        .with_introspection(true)
        .with_schema_docs(false),
    _ => GraphQLConfig::development("/graphql"),
};
```

### Example 5: Full Controller Implementation

```rust
use armature_framework::prelude::*;
use armature_graphql::*;

#[controller("/graphql")]
struct GraphQLController {
    schema: Schema<Query, Mutation, Subscription>,
    config: GraphQLConfig,
}

impl GraphQLController {
    #[post("/")]
    async fn execute(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        // Execute GraphQL query
        // ...
    }

    #[get("/playground")]
    async fn playground(&self, _req: HttpRequest) -> Result<HttpResponse, Error> {
        if !self.config.enable_playground {
            return Err(Error::NotFound);
        }

        let html = graphql_playground_html(&self.config.endpoint);
        Ok(HttpResponse::ok()
            .with_header("Content-Type", "text/html")
            .with_body(html.into_bytes()))
    }

    #[get("/schema")]
    async fn schema_docs(&self, _req: HttpRequest) -> Result<HttpResponse, Error> {
        if !self.config.enable_schema_docs {
            return Err(Error::NotFound);
        }

        let html = generate_schema_docs_html(&self.schema, &self.config.endpoint, "My API");
        Ok(HttpResponse::ok()
            .with_header("Content-Type", "text/html")
            .with_body(html.into_bytes()))
    }
}
```

---

## Best Practices

1. **Use Environment-Based Config**
   - Development: All features enabled
   - Staging: Selective features
   - Production: Minimal features

2. **Protect Sensitive Endpoints**
   - Add authentication to playgrounds
   - Use rate limiting
   - Monitor access logs

3. **Enable Documentation Selectively**
   - Public APIs: Enable schema docs
   - Internal APIs: Disable in production
   - Authenticated APIs: Require auth

4. **Set Complexity Limits**
   - Prevent denial-of-service attacks
   - Limit query depth
   - Monitor query complexity

5. **Use HTTPS in Production**
   - Encrypt GraphQL traffic
   - Protect sensitive data
   - Enable CORS appropriately

---

## Summary

Armature's GraphQL configuration provides:

✅ **Flexible playground options** (Playground, GraphiQL)
✅ **Interactive schema documentation**
✅ **Development vs production configurations**
✅ **Security controls** (introspection, complexity limits)
✅ **Customizable endpoints**
✅ **Easy integration** with Armature framework

For complete examples, see:
- `examples/graphql_with_docs.rs`
- `examples/graphql_api.rs`
- `examples/graphql_programmatic.rs`


