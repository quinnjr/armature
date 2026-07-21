# armature-openapi

OpenAPI/Swagger documentation for the Armature framework.

## Features

- **Programmatic Builder** - Build OpenAPI 3.0 specs by hand with a fluent builder API
- **Swagger UI** - HTML response helper for interactive API documentation (loads UI assets from a CDN; requires internet access, not self-contained)
- **Export** - JSON/YAML spec export

There is currently no route introspection, handler type inference, or
request-validation support in this crate — specs are assembled entirely by
hand via `OpenApiBuilder`.

## Installation

```toml
[dependencies]
armature-openapi = "0.1"
```

## Quick Start

```rust
use armature_openapi::{OpenApiBuilder, SwaggerConfig, swagger_ui_response, spec_json_response};

// Build the spec by hand with the fluent builder.
let spec = OpenApiBuilder::new("My API", "1.0.0")
    .description("A wonderful API")
    .server("http://localhost:3000", None)
    .build();

// Wrap it in a SwaggerConfig to serve it.
let config = SwaggerConfig::new("/api-docs", spec).with_title("My API Documentation");

// `swagger_ui_response(&config)` returns an `HttpResponse` rendering the Swagger UI HTML page.
// `spec_json_response(&config)` returns an `HttpResponse` serving the raw spec as JSON.
let ui_response = swagger_ui_response(&config).unwrap();
let json_response = spec_json_response(&config).unwrap();

assert_eq!(config.path, "/api-docs");
assert_eq!(config.title, "My API Documentation");
```

Wire these into your router as you would any other handler, e.g. serving
`ui_response` at `GET /api-docs` and `json_response` at `GET /openapi.json`.

## Adding Paths and Operations

```rust
use armature_openapi::{OpenApiBuilder, PathItemBuilder, OperationBuilder, Response};
use std::collections::HashMap;

let list_users = OperationBuilder::new()
    .summary("List users")
    .operation_id("listUsers")
    .response(
        "200",
        Response {
            description: "List of users".to_string(),
            content: None,
        },
    )
    .build();

let path_item = PathItemBuilder::new().get(list_users).build();

let mut spec = OpenApiBuilder::new("User API", "1.0.0").build();
spec.paths.insert("/users".to_string(), path_item);

assert!(spec.paths.contains_key("/users"));
```

## Schemas

Schemas are also built by hand, either with the raw `Schema` struct or the
helper functions in `armature_openapi::builder` (`string_schema`,
`integer_schema`, `object_schema`, `array_schema`, `ref_schema`, ...):

```rust
use armature_openapi::{object_schema, string_schema};
use std::collections::HashMap;

let mut properties = HashMap::new();
properties.insert("name".to_string(), string_schema());
properties.insert("email".to_string(), string_schema());

let create_user_schema = object_schema(properties, vec!["name".to_string(), "email".to_string()]);

assert_eq!(create_user_schema.properties.unwrap().len(), 2);
```

## Authentication

```rust
use armature_openapi::{OpenApiBuilder, ApiKeyLocation};

let spec = OpenApiBuilder::new("Secure API", "1.0.0")
    .add_bearer_auth("bearer")
    .add_api_key_auth("api_key", "X-API-Key", ApiKeyLocation::Header)
    .build();

assert!(spec.components.is_some());
```

Supported security schemes: HTTP (e.g. Bearer), API Key, OAuth2, and OpenID
Connect (`SecurityScheme::OpenIdConnect`).

## License

MIT OR Apache-2.0
