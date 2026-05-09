# OpenAPI & Swagger UI Guide

Generate OpenAPI 3.0 specifications and serve interactive API documentation with Swagger UI.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Quick Start](#quick-start)
- [Programmatic Builder](#programmatic-builder)
- [Swagger UI Integration](#swagger-ui-integration)
- [Schema Definition](#schema-definition)
- [Security](#security)
- [Best Practices](#best-practices)
- [Examples](#examples)

## Overview

The `armature-openapi` module provides tools for generating OpenAPI 3.0 specifications and serving interactive API documentation. It allows you to document your APIs programmatically and serve beautiful, interactive documentation via Swagger UI.

### Why OpenAPI?

- **Industry Standard**: OpenAPI (formerly Swagger) is the de facto standard for REST API documentation
- **Interactive**: Test your API directly from the documentation
- **Code Generation**: Generate client SDKs in multiple languages
- **Validation**: Validate requests/responses against the specification
- **Discoverability**: Make your API easy to understand and use

## Features

✅ **Programmatic Builder**
- Fluent API for building OpenAPI specs
- Type-safe specification construction
- Helper functions for common patterns

✅ **Swagger UI Integration**
- Beautiful, interactive API documentation
- Test endpoints directly from the browser
- No configuration required

✅ **Multiple Export Formats**
- JSON export
- YAML export
- HTML (Swagger UI)

✅ **Full OpenAPI 3.0 Support**
- All HTTP methods (GET, POST, PUT, DELETE, PATCH)
- Request/response schemas
- Authentication (Bearer, API Key, OAuth2)
- Tags and organization

## Quick Start

### Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
armature-framework = { version = "0.1", features = ["openapi"] }
```

### Basic Example

```rust
use armature_framework::prelude::*;
use armature_framework::armature_openapi::*;

// Build OpenAPI specification
let spec = OpenApiBuilder::new("My API", "1.0.0")
    .description("A wonderful API")
    .server("http://localhost:3000", None)
    .tag("users", Some("User endpoints".to_string()))
    .build();

// Create Swagger UI config
let config = SwaggerConfig::new("/api-docs", spec);

// Serve Swagger UI
#[get("/api-docs")]
async fn swagger_ui(config: SwaggerConfig) -> Result<HttpResponse, Error> {
    swagger_ui_response(&config)
}
```

## Programmatic Builder

### Creating a Specification

```rust
use armature_openapi::*;

let spec = OpenApiBuilder::new("User API", "1.0.0")
    // Basic info
    .description("A comprehensive user management API")
    .terms_of_service("https://example.com/terms")

    // Contact
    .contact(
        Some("API Support".to_string()),
        Some("https://example.com/support".to_string()),
        Some("support@example.com".to_string()),
    )

    // License
    .license("MIT", Some("https://opensource.org/licenses/MIT".to_string()))

    // Servers
    .server("https://api.example.com", Some("Production".to_string()))
    .server("http://localhost:3000", Some("Development".to_string()))

    // Tags
    .tag("users", Some("User management".to_string()))
    .tag("auth", Some("Authentication".to_string()))

    .build();
```

### Adding Endpoints

```rust
let spec = OpenApiBuilder::new("API", "1.0.0")
    .path(
        "/users",
        PathItemBuilder::new()
            .get(
                OperationBuilder::new()
                    .summary("List all users")
                    .description("Returns a paginated list of users")
                    .tag("users")
                    .operation_id("listUsers")
                    .parameter(Parameter {
                        name: "page".to_string(),
                        location: ParameterLocation::Query,
                        description: Some("Page number".to_string()),
                        required: Some(false),
                        schema: Some(integer_schema()),
                    })
                    .response(
                        "200",
                        Response {
                            description: "Successful response".to_string(),
                            content: Some({
                                let mut content = HashMap::new();
                                content.insert(
                                    "application/json".to_string(),
                                    MediaType {
                                        schema: Some(array_schema(ref_schema("User"))),
                                    },
                                );
                                content
                            }),
                        },
                    )
                    .build(),
            )
            .post(
                OperationBuilder::new()
                    .summary("Create a user")
                    .tag("users")
                    .operation_id("createUser")
                    .request_body(RequestBody {
                        description: Some("User to create".to_string()),
                        content: {
                            let mut content = HashMap::new();
                            content.insert(
                                "application/json".to_string(),
                                MediaType {
                                    schema: Some(ref_schema("User")),
                                },
                            );
                            content
                        },
                        required: Some(true),
                    })
                    .response(
                        "201",
                        Response {
                            description: "User created".to_string(),
                            content: Some({
                                let mut content = HashMap::new();
                                content.insert(
                                    "application/json".to_string(),
                                    MediaType {
                                        schema: Some(ref_schema("User")),
                                    },
                                );
                                content
                            }),
                        },
                    )
                    .build(),
            )
            .build(),
    )
    .build();
```

### Path Parameters

```rust
.path(
    "/users/{id}",
    PathItemBuilder::new()
        .get(
            OperationBuilder::new()
                .summary("Get user by ID")
                .tag("users")
                .parameter(Parameter {
                    name: "id".to_string(),
                    location: ParameterLocation::Path,
                    description: Some("User ID".to_string()),
                    required: Some(true),
                    schema: Some(integer_schema()),
                })
                .response(
                    "200",
                    Response {
                        description: "User found".to_string(),
                        content: Some({
                            let mut content = HashMap::new();
                            content.insert(
                                "application/json".to_string(),
                                MediaType {
                                    schema: Some(ref_schema("User")),
                                },
                            );
                            content
                        }),
                    },
                )
                .response(
                    "404",
                    Response {
                        description: "User not found".to_string(),
                        content: None,
                    },
                )
                .build(),
        )
        .build(),
)
```

## Schema Definition

### Primitive Types

```rust
use armature_openapi::*;

// String
let name_schema = string_schema();

// Integer
let id_schema = integer_schema();

// Number (floating point)
let price_schema = number_schema();

// Boolean
let active_schema = boolean_schema();
```

### Arrays

```rust
// Array of strings
let tags_schema = array_schema(string_schema());

// Array of objects
let users_schema = array_schema(ref_schema("User"));
```

### Objects

```rust
let user_schema = object_schema(
    {
        let mut props = HashMap::new();
        props.insert("id".to_string(), integer_schema());
        props.insert("name".to_string(), string_schema());
        props.insert("email".to_string(), string_schema());
        props.insert("age".to_string(), integer_schema());
        props.insert("active".to_string(), boolean_schema());
        props
    },
    vec![
        "id".to_string(),
        "name".to_string(),
        "email".to_string(),
    ],
);
```

### Nested Objects

```rust
let address_schema = object_schema(
    {
        let mut props = HashMap::new();
        props.insert("street".to_string(), string_schema());
        props.insert("city".to_string(), string_schema());
        props.insert("country".to_string(), string_schema());
        props
    },
    vec!["street".to_string(), "city".to_string()],
);

let user_with_address = object_schema(
    {
        let mut props = HashMap::new();
        props.insert("id".to_string(), integer_schema());
        props.insert("name".to_string(), string_schema());
        props.insert("address".to_string(), address_schema);
        props
    },
    vec!["id".to_string(), "name".to_string()],
);
```

### Reusable Schemas

```rust
let spec = OpenApiBuilder::new("API", "1.0.0")
    // Define schema once
    .schema("User", object_schema(
        {
            let mut props = HashMap::new();
            props.insert("id".to_string(), integer_schema());
            props.insert("name".to_string(), string_schema());
            props.insert("email".to_string(), string_schema());
            props
        },
        vec!["id".to_string(), "name".to_string(), "email".to_string()],
    ))
    .schema("Address", object_schema(
        {
            let mut props = HashMap::new();
            props.insert("street".to_string(), string_schema());
            props.insert("city".to_string(), string_schema());
            props
        },
        vec!["street".to_string(), "city".to_string()],
    ))
    // Reference schemas in endpoints
    .path(
        "/users",
        PathItemBuilder::new()
            .get(
                OperationBuilder::new()
                    .summary("List users")
                    .response(
                        "200",
                        Response {
                            description: "Success".to_string(),
                            content: Some({
                                let mut content = HashMap::new();
                                content.insert(
                                    "application/json".to_string(),
                                    MediaType {
                                        schema: Some(array_schema(ref_schema("User"))),
                                    },
                                );
                                content
                            }),
                        },
                    )
                    .build(),
            )
            .build(),
    )
    .build();
```

## Swagger UI Integration

### Basic Setup

```rust
use armature_framework::prelude::*;
use armature_framework::armature_openapi::*;

#[controller("/api-docs")]
struct ApiDocsController {
    config: SwaggerConfig,
}

impl ApiDocsController {
    #[get("/")]
    async fn swagger_ui(&self) -> Result<HttpResponse, Error> {
        swagger_ui_response(&self.config)
    }

    #[get("/openapi.json")]
    async fn openapi_json(&self) -> Result<HttpResponse, Error> {
        spec_json_response(&self.config)
    }

    #[get("/openapi.yaml")]
    async fn openapi_yaml(&self) -> Result<HttpResponse, Error> {
        spec_yaml_response(&self.config)
    }
}
```

### Custom Title

```rust
let config = SwaggerConfig::new("/api-docs", spec)
    .with_title("My Amazing API Documentation");
```

### Multiple Versions

```rust
// Version 1
let spec_v1 = OpenApiBuilder::new("My API", "1.0.0")
    .server("https://api.example.com/v1", None)
    .build();

let config_v1 = SwaggerConfig::new("/api-docs/v1", spec_v1);

// Version 2
let spec_v2 = OpenApiBuilder::new("My API", "2.0.0")
    .server("https://api.example.com/v2", None)
    .build();

let config_v2 = SwaggerConfig::new("/api-docs/v2", spec_v2);
```

## Security

### Bearer Authentication (JWT)

```rust
let spec = OpenApiBuilder::new("API", "1.0.0")
    // Add security scheme
    .add_bearer_auth("bearer")

    // Apply to specific endpoint
    .path(
        "/users",
        PathItemBuilder::new()
            .get(
                OperationBuilder::new()
                    .summary("List users")
                    .security({
                        let mut req = HashMap::new();
                        req.insert("bearer".to_string(), vec![]);
                        req
                    })
                    .response("200", Response {
                        description: "Success".to_string(),
                        content: None,
                    })
                    .build(),
            )
            .build(),
    )
    .build();
```

### API Key Authentication

```rust
let spec = OpenApiBuilder::new("API", "1.0.0")
    .add_api_key_auth(
        "api_key",
        "X-API-Key",
        ApiKeyLocation::Header,
    )
    .path(
        "/users",
        PathItemBuilder::new()
            .get(
                OperationBuilder::new()
                    .summary("List users")
                    .security({
                        let mut req = HashMap::new();
                        req.insert("api_key".to_string(), vec![]);
                        req
                    })
                    .response("200", Response {
                        description: "Success".to_string(),
                        content: None,
                    })
                    .build(),
            )
            .build(),
    )
    .build();
```

### OAuth2

```rust
let spec = OpenApiBuilder::new("API", "1.0.0")
    .security_scheme(
        "oauth2",
        SecurityScheme::OAuth2 {
            flows: OAuthFlows {
                authorization_code: Some(OAuthFlow {
                    authorization_url: Some("https://example.com/oauth/authorize".to_string()),
                    token_url: Some("https://example.com/oauth/token".to_string()),
                    refresh_url: None,
                    scopes: {
                        let mut scopes = HashMap::new();
                        scopes.insert("read".to_string(), "Read access".to_string());
                        scopes.insert("write".to_string(), "Write access".to_string());
                        scopes
                    },
                }),
                ..Default::default()
            },
        },
    )
    .build();
```

### Global Security

```rust
// Apply authentication to all endpoints by default
let spec = OpenApiBuilder::new("API", "1.0.0")
    .add_bearer_auth("bearer")
    .security({
        let mut req = HashMap::new();
        req.insert("bearer".to_string(), vec![]);
        req
    })
    .build();
```

## Best Practices

### 1. Use Descriptive Names

```rust
// ✅ Good
.operation_id("getUserById")
.summary("Get a user by their unique identifier")

// ❌ Bad
.operation_id("get1")
.summary("Get")
```

### 2. Provide Examples

```rust
// Add examples to schemas
let user_schema = object_schema(
    {
        let mut props = HashMap::new();
        props.insert("id".to_string(), integer_schema());
        props.insert("name".to_string(), string_schema());
        props
    },
    vec!["id".to_string(), "name".to_string()],
);
```

### 3. Document Error Responses

```rust
.response("200", Response {
    description: "Successful response".to_string(),
    content: Some(/* ... */),
})
.response("400", Response {
    description: "Invalid request parameters".to_string(),
    content: None,
})
.response("401", Response {
    description: "Authentication required".to_string(),
    content: None,
})
.response("403", Response {
    description: "Insufficient permissions".to_string(),
    content: None,
})
.response("404", Response {
    description: "Resource not found".to_string(),
    content: None,
})
.response("500", Response {
    description: "Internal server error".to_string(),
    content: None,
})
```

### 4. Use Tags for Organization

```rust
let spec = OpenApiBuilder::new("E-commerce API", "1.0.0")
    .tag("products", Some("Product catalog".to_string()))
    .tag("orders", Some("Order management".to_string()))
    .tag("users", Some("User accounts".to_string()))
    .tag("auth", Some("Authentication".to_string()))
    .build();
```

### 5. Version Your API

```rust
// Include version in URL
.server("https://api.example.com/v1", Some("Version 1".to_string()))
.server("https://api.example.com/v2", Some("Version 2".to_string()))
```

### 6. Keep Schemas DRY

```rust
// Define common schemas once
let spec = OpenApiBuilder::new("API", "1.0.0")
    .schema("Error", object_schema(/* ... */))
    .schema("User", object_schema(/* ... */))
    .schema("Product", object_schema(/* ... */))
    // Then reference them
    .path("/users", /* use ref_schema("User") */)
    .build();
```

## Examples

### Complete REST API

```rust
use armature_openapi::*;

let spec = OpenApiBuilder::new("Task Manager API", "1.0.0")
    .description("A simple task management API")
    .server("http://localhost:3000", Some("Development".to_string()))

    // Tags
    .tag("tasks", Some("Task operations".to_string()))
    .tag("users", Some("User operations".to_string()))

    // Auth
    .add_bearer_auth("bearer")

    // Schemas
    .schema("Task", object_schema(
        {
            let mut props = HashMap::new();
            props.insert("id".to_string(), integer_schema());
            props.insert("title".to_string(), string_schema());
            props.insert("completed".to_string(), boolean_schema());
            props
        },
        vec!["id".to_string(), "title".to_string()],
    ))

    // Endpoints
    .path(
        "/tasks",
        PathItemBuilder::new()
            .get(
                OperationBuilder::new()
                    .summary("List tasks")
                    .tag("tasks")
                    .security({
                        let mut req = HashMap::new();
                        req.insert("bearer".to_string(), vec![]);
                        req
                    })
                    .response("200", Response {
                        description: "Success".to_string(),
                        content: Some({
                            let mut content = HashMap::new();
                            content.insert(
                                "application/json".to_string(),
                                MediaType {
                                    schema: Some(array_schema(ref_schema("Task"))),
                                },
                            );
                            content
                        }),
                    })
                    .build(),
            )
            .post(
                OperationBuilder::new()
                    .summary("Create task")
                    .tag("tasks")
                    .security({
                        let mut req = HashMap::new();
                        req.insert("bearer".to_string(), vec![]);
                        req
                    })
                    .request_body(RequestBody {
                        description: Some("Task to create".to_string()),
                        content: {
                            let mut content = HashMap::new();
                            content.insert(
                                "application/json".to_string(),
                                MediaType {
                                    schema: Some(ref_schema("Task")),
                                },
                            );
                            content
                        },
                        required: Some(true),
                    })
                    .response("201", Response {
                        description: "Created".to_string(),
                        content: Some({
                            let mut content = HashMap::new();
                            content.insert(
                                "application/json".to_string(),
                                MediaType {
                                    schema: Some(ref_schema("Task")),
                                },
                            );
                            content
                        }),
                    })
                    .build(),
            )
            .build(),
    )

    .build();
```

## Summary

**Key Features:**
- ✅ Programmatic OpenAPI 3.0 spec generation
- ✅ Interactive Swagger UI documentation
- ✅ JSON/YAML export
- ✅ Full security scheme support
- ✅ Type-safe builders

**When to Use:**
- Documenting REST APIs
- Generating client SDKs
- API contract validation
- Developer onboarding
- API discovery

**Next Steps:**
1. Define your API schemas
2. Document each endpoint
3. Add security schemes
4. Serve Swagger UI
5. Export OpenAPI spec

Happy documenting! 📚✨


