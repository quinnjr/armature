//! OpenAPI 3.0 specification generation and Swagger UI integration for Armature
//!
//! This crate provides tools for generating OpenAPI specifications and serving
//! interactive API documentation via Swagger UI.
//!
//! ## Features
//!
//! - 📝 **Programmatic API** - Build OpenAPI specs with fluent builder
//! - 📊 **Swagger UI response helper** - `swagger_ui_response` renders an interactive
//!   Swagger UI page, loading its JS/CSS assets from a CDN (not self-contained; requires
//!   network access and is incompatible with a strict CSP)
//! - 📤 **JSON/YAML Export** - Multiple export formats
//! - 🔒 **Type-Safe** - Strongly typed OpenAPI 3.0 specification
//! - 🔐 **Auth Schemes** - Bearer, API Key, OAuth2, OpenID Connect
//! - 📋 **Schema Support** - Request/response schemas
//!
//! ## Quick Start - Basic API Spec
//!
//! ```
//! use armature_openapi::OpenApiBuilder;
//!
//! let spec = OpenApiBuilder::new("My API", "1.0.0")
//!     .description("A wonderful API")
//!     .server("http://localhost:3000", None)
//!     .build();
//!
//! assert_eq!(spec.info.title, "My API");
//! assert_eq!(spec.info.version, "1.0.0");
//! assert_eq!(spec.servers.len(), 1);
//! ```
//!
//! ## Adding Authentication
//!
//! ```
//! use armature_openapi::{OpenApiBuilder, ApiKeyLocation};
//!
//! let spec = OpenApiBuilder::new("Secure API", "1.0.0")
//!     .add_bearer_auth("bearer")
//!     .add_api_key_auth("api_key", "X-API-Key", ApiKeyLocation::Header)
//!     .build();
//!
//! assert!(spec.components.is_some());
//! let components = spec.components.unwrap();
//! assert!(components.security_schemes.contains_key("bearer"));
//! assert!(components.security_schemes.contains_key("api_key"));
//! ```
//!
//! ## Adding Paths and Operations
//!
//! ```
//! use armature_openapi::{OpenApiBuilder, PathItem, Operation, Response};
//! use std::collections::HashMap;
//!
//! let mut get_operation = Operation::default();
//! get_operation.summary = Some("Get user by ID".to_string());
//! get_operation.operation_id = Some("getUserById".to_string());
//!
//! let mut responses = HashMap::new();
//! responses.insert("200".to_string(), Response {
//!     description: "Successful response".to_string(),
//!     content: None,
//! });
//! get_operation.responses = responses;
//!
//! let mut path_item = PathItem::default();
//! path_item.get = Some(get_operation);
//!
//! let mut spec = OpenApiBuilder::new("User API", "1.0.0").build();
//! spec.paths.insert("/users/{id}".to_string(), path_item);
//!
//! assert!(spec.paths.contains_key("/users/{id}"));
//! assert!(spec.paths.get("/users/{id}").unwrap().get.is_some());
//! ```
//!
//! ## Swagger UI Configuration
//!
//! ```
//! use armature_openapi::{OpenApiBuilder, SwaggerConfig};
//!
//! let spec = OpenApiBuilder::new("My API", "1.0.0")
//!     .description("API with Swagger UI")
//!     .build();
//!
//! let swagger_config = SwaggerConfig::new("/api-docs", spec)
//!     .with_title("My API Documentation");
//!
//! // Configuration ready
//! assert_eq!(swagger_config.path, "/api-docs");
//! assert_eq!(swagger_config.title, "My API Documentation");
//! ```
//!
//! ## Adding Multiple Servers
//!
//! ```
//! use armature_openapi::OpenApiBuilder;
//!
//! let spec = OpenApiBuilder::new("Multi-Env API", "1.0.0")
//!     .server("https://api.production.com", Some("Production server".to_string()))
//!     .server("https://api.staging.com", Some("Staging server".to_string()))
//!     .server("http://localhost:3000", Some("Local development".to_string()));
//!
//! let built_spec = spec.build();
//!
//! assert_eq!(built_spec.servers.len(), 3);
//! assert_eq!(built_spec.servers[0].url, "https://api.production.com");
//! assert_eq!(built_spec.servers[1].url, "https://api.staging.com");
//! assert_eq!(built_spec.servers[2].url, "http://localhost:3000");
//! assert_eq!(built_spec.servers[0].description, Some("Production server".to_string()));
//! ```
//!
//! ## README Quick Start
//!
//! This mirrors the example in `README.md` verbatim, so it is guaranteed to compile.
//!
//! ```
//! use armature_openapi::{OpenApiBuilder, SwaggerConfig, swagger_ui_response, spec_json_response};
//!
//! // Build the spec by hand with the fluent builder.
//! let spec = OpenApiBuilder::new("My API", "1.0.0")
//!     .description("A wonderful API")
//!     .server("http://localhost:3000", None)
//!     .build();
//!
//! // Wrap it in a SwaggerConfig to serve it.
//! let config = SwaggerConfig::new("/api-docs", spec).with_title("My API Documentation");
//!
//! // `swagger_ui_response(&config)` returns an `HttpResponse` rendering the Swagger UI HTML page.
//! // `spec_json_response(&config)` returns an `HttpResponse` serving the raw spec as JSON.
//! let ui_response = swagger_ui_response(&config).unwrap();
//! let json_response = spec_json_response(&config).unwrap();
//!
//! assert_eq!(config.path, "/api-docs");
//! assert_eq!(config.title, "My API Documentation");
//! assert_eq!(ui_response.status, 200);
//! assert_eq!(json_response.status, 200);
//! ```

pub mod builder;
pub mod playground;
pub mod spec;
pub mod swagger;

pub use builder::*;
pub use playground::*;
pub use spec::*;
pub use swagger::*;
