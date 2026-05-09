# Error Transformation Guide

Armature provides a centralized error transformation system for consistent, configurable, and secure error handling across your application.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Basic Usage](#basic-usage)
- [Response Formats](#response-formats)
- [Error Response Structure](#error-response-structure)
- [Error Transformer](#error-transformer)
- [Validation Errors](#validation-errors)
- [Sensitive Data Filtering](#sensitive-data-filtering)
- [Custom Transformers](#custom-transformers)
- [Preset Configurations](#preset-configurations)
- [Best Practices](#best-practices)
- [API Reference](#api-reference)
- [Summary](#summary)

## Overview

The error transformation system provides a centralized way to:

- Convert application errors to HTTP responses
- Format errors consistently (JSON, HTML, Problem Details)
- Filter sensitive information
- Add context and metadata
- Log errors appropriately

## Features

- ✅ Multiple response formats (JSON, plain text, HTML, RFC 7807)
- ✅ Configurable error response structure
- ✅ Sensitive data filtering (passwords, tokens, etc.)
- ✅ Validation error aggregation
- ✅ Custom error transformers
- ✅ Production/development modes
- ✅ Error logging integration
- ✅ Problem Details (RFC 7807) support

## Basic Usage

### Simple Error Transformation

```rust
use armature_core::{Error, HttpRequest, HttpResponse};
use armature_core::error_transform::{ErrorTransformer, ResponseFormat};

async fn handle_request(req: HttpRequest) -> Result<HttpResponse, Error> {
    // Your handler logic...
    Err(Error::NotFound("User not found".into()))
}

async fn error_handler(req: HttpRequest) -> HttpResponse {
    let transformer = ErrorTransformer::new()
        .format(ResponseFormat::Json)
        .production_mode(true);

    match handle_request(req.clone()).await {
        Ok(response) => response,
        Err(error) => transformer.transform(&error, &req),
    }
}
```

### Using ErrorResponseBuilder

For quick error responses:

```rust
use armature_core::error_transform::ErrorResponseBuilder;

// Quick error responses
let response = ErrorResponseBuilder::bad_request("Invalid input");
let response = ErrorResponseBuilder::not_found("Resource not found");
let response = ErrorResponseBuilder::internal_error("Something went wrong");
```

## Response Formats

Armature supports multiple standardized error response formats used across different platforms and specifications.

### JSON (Default)

Simple, clean JSON format for general use.

```rust
let transformer = ErrorTransformer::new()
    .format(ResponseFormat::Json);
```

Response:
```json
{
  "status": 404,
  "message": "User not found",
  "error_type": "NOT_FOUND",
  "path": "/users/123",
  "timestamp": "2024-01-15T10:30:00Z",
  "request_id": "req-abc123"
}
```

### Plain Text

```rust
let transformer = ErrorTransformer::new()
    .format(ResponseFormat::PlainText);
```

Response:
```
Error 404: User not found
Details: The requested resource could not be found
```

### HTML

```rust
let transformer = ErrorTransformer::new()
    .format(ResponseFormat::Html);
```

Renders a styled HTML error page suitable for browser display.

### Problem Details (RFC 7807)

Standard IETF format for HTTP API problem details.

```rust
let transformer = ErrorTransformer::new()
    .format(ResponseFormat::ProblemDetails);
```

Response (`application/problem+json`):
```json
{
  "type": "NOT_FOUND",
  "title": "User not found",
  "status": 404,
  "detail": "The requested user could not be found",
  "instance": "/users/123"
}
```

### JSON:API

JSON:API specification format (https://jsonapi.org/format/#errors).

```rust
let transformer = ErrorTransformer::new()
    .format(ResponseFormat::JsonApi);
```

Response (`application/vnd.api+json`):
```json
{
  "errors": [
    {
      "status": "422",
      "code": "VALIDATION_ERROR",
      "title": "Validation failed",
      "detail": "Email format is invalid",
      "source": {
        "pointer": "/data/attributes/email"
      }
    }
  ]
}
```

### GraphQL

GraphQL specification error format (https://spec.graphql.org).

```rust
let transformer = ErrorTransformer::new()
    .format(ResponseFormat::GraphQL);
```

Response:
```json
{
  "data": null,
  "errors": [
    {
      "message": "User not found",
      "path": ["/users/123"],
      "extensions": {
        "code": "NOT_FOUND",
        "status": 404
      }
    }
  ]
}
```

### Google/gRPC

Google Cloud API and gRPC-style error format.

```rust
let transformer = ErrorTransformer::new()
    .format(ResponseFormat::Google);
```

Response:
```json
{
  "error": {
    "code": 400,
    "message": "Invalid argument",
    "status": "INVALID_ARGUMENT",
    "details": [
      {
        "@type": "type.googleapis.com/google.rpc.BadRequest.FieldViolation",
        "field": "email",
        "description": "Invalid email format"
      }
    ]
  }
}
```

### AWS

AWS service-style error format.

```rust
let transformer = ErrorTransformer::new()
    .format(ResponseFormat::Aws);
```

Response (`application/x-amz-json-1.1`):
```json
{
  "__type": "ValidationException",
  "message": "Validation error occurred",
  "Code": "VALIDATION_ERROR",
  "RequestId": "req-abc123"
}
```

### Azure

Microsoft Azure REST API error format.

```rust
let transformer = ErrorTransformer::new()
    .format(ResponseFormat::Azure);
```

Response:
```json
{
  "error": {
    "code": "InvalidInput",
    "message": "The request is invalid",
    "target": "/api/users",
    "details": [
      {
        "code": "ValidationError",
        "message": "Email is required",
        "target": "email"
      }
    ]
  }
}
```

### Format Comparison Table

| Format | Content-Type | Best For |
|--------|--------------|----------|
| `Json` | `application/json` | General APIs |
| `PlainText` | `text/plain` | CLI tools, debugging |
| `Html` | `text/html` | Browser-facing apps |
| `ProblemDetails` | `application/problem+json` | RESTful APIs (RFC 7807) |
| `JsonApi` | `application/vnd.api+json` | JSON:API implementations |
| `GraphQL` | `application/json` | GraphQL APIs |
| `Google` | `application/json` | Google Cloud/gRPC style |
| `Aws` | `application/x-amz-json-1.1` | AWS-compatible APIs |
| `Azure` | `application/json` | Azure-compatible APIs |

## Error Response Structure

### ErrorResponse

```rust
use armature_core::error_transform::{ErrorResponse, ValidationError};

let response = ErrorResponse::new(400)
    .message("Validation failed")
    .code("ERR_VALIDATION")
    .details("One or more fields are invalid")
    .error_type("VALIDATION_ERROR")
    .path("/api/users")
    .request_id("req-123")
    .with_metadata("field_count", 3)
    .with_validation_error(
        ValidationError::new("email", "Invalid email format")
    );
```

### Available Fields

| Field | Type | Description |
|-------|------|-------------|
| `status` | `u16` | HTTP status code |
| `code` | `Option<String>` | Application-specific error code |
| `message` | `String` | Human-readable message |
| `details` | `Option<String>` | Detailed description |
| `error_type` | `Option<String>` | Error category |
| `path` | `Option<String>` | Request path |
| `timestamp` | `Option<String>` | ISO 8601 timestamp |
| `request_id` | `Option<String>` | Request ID for tracing |
| `metadata` | `HashMap` | Additional data |
| `validation_errors` | `Vec` | Field validation errors |

## Error Transformer

### Configuration Options

```rust
let transformer = ErrorTransformer::new()
    // Response format
    .format(ResponseFormat::Json)

    // Production mode (hides internal details)
    .production_mode(true)

    // Include/exclude options
    .include_stack_trace(false)
    .include_path(true)
    .include_timestamp(true)

    // Security
    .filter_sensitive_data(true)

    // Custom error codes
    .map_error_code("NOT_FOUND", "ERR_404")
    .map_error_code("VALIDATION_ERROR", "ERR_422");
```

### Transform Methods

```rust
// Simple transform
let response = transformer.transform(&error, &request);

// Transform with context
let context = ErrorContext::from_request(request)
    .user_id("user-123")
    .with_data("operation", "create_user");

let response = transformer.transform_with_context(&error, &context);
```

## Validation Errors

### Single Validation Error

```rust
use armature_core::error_transform::ValidationError;

let error = ValidationError::new("email", "Invalid email format")
    .rule("email")
    .value("not-an-email");
```

### Multiple Validation Errors

```rust
use armature_core::error_transform::{ErrorResponse, ValidationError, ErrorResponseBuilder};

let response = ErrorResponseBuilder::validation_error(vec![
    ValidationError::new("email", "Invalid email format"),
    ValidationError::new("password", "Password must be at least 8 characters"),
    ValidationError::new("age", "Must be at least 18"),
]);
```

Response:
```json
{
  "status": 422,
  "message": "Validation failed",
  "error_type": "VALIDATION_ERROR",
  "validation_errors": [
    {
      "field": "email",
      "message": "Invalid email format"
    },
    {
      "field": "password",
      "message": "Password must be at least 8 characters"
    },
    {
      "field": "age",
      "message": "Must be at least 18"
    }
  ]
}
```

## Sensitive Data Filtering

The transformer automatically filters sensitive data when `filter_sensitive_data(true)`:

### Filtered Patterns

- Passwords: `password=secret` → `password=[FILTERED]`
- API keys: `api_key=abc123` → `api_key=[FILTERED]`
- Tokens: `token=xyz` → `token=[FILTERED]`
- Bearer tokens: `Bearer abc` → `Bearer [FILTERED]`
- Secrets: `secret=val` → `secret=[FILTERED]`
- Credit cards: `4111111111111111` → `[CARD FILTERED]`
- SSN: `123-45-6789` → `[SSN FILTERED]`

### Example

```rust
let transformer = ErrorTransformer::new()
    .filter_sensitive_data(true);

let error = Error::BadRequest(
    "Invalid request: password=secret123, api_key=abc".into()
);

// Error message becomes:
// "Invalid request: password=[FILTERED], api_key=[FILTERED]"
```

## Custom Transformers

### Add Custom Transform Logic

```rust
let transformer = ErrorTransformer::new()
    .with_transformer(|error, context| {
        // Custom logic for specific errors
        match error {
            Error::NotFound(_) => Some(
                ErrorResponse::new(404)
                    .message("Resource not found")
                    .code("RESOURCE_NOT_FOUND")
                    .with_metadata("suggestion", "Check the resource ID")
            ),
            Error::Unauthorized(_) => Some(
                ErrorResponse::new(401)
                    .message("Authentication required")
                    .with_metadata("auth_url", "/login")
            ),
            _ => None, // Fall through to default handling
        }
    });
```

### Add Custom Logging

```rust
let transformer = ErrorTransformer::new()
    .with_logger(|error, context, response| {
        // Custom logging logic
        if error.is_server_error() {
            eprintln!(
                "[ERROR] {} {} - {} ({})",
                context.request.method,
                context.request.path,
                error,
                response.request_id.as_deref().unwrap_or("unknown")
            );
        }
    });
```

### Add Error Filters

```rust
let transformer = ErrorTransformer::new()
    .with_filter(|error| {
        // Transform or filter errors
        match error {
            Error::Internal(msg) if msg.contains("database") => {
                Error::ServiceUnavailable("Service temporarily unavailable".into())
            }
            _ => error.clone(),
        }
    });
```

## Preset Configurations

### Development

```rust
let transformer = ErrorTransformer::development();
```

Features:
- Verbose error messages
- Stack traces enabled
- No sensitive data filtering
- Console logging

### Production

```rust
let transformer = ErrorTransformer::production();
```

Features:
- Generic messages for server errors
- No stack traces
- Sensitive data filtering
- Structured logging

### API

```rust
let transformer = ErrorTransformer::api();
```

Features:
- RFC 7807 Problem Details format
- Production mode
- Sensitive data filtering

## Best Practices

### 1. Use Production Mode in Production

```rust
let transformer = if cfg!(debug_assertions) {
    ErrorTransformer::development()
} else {
    ErrorTransformer::production()
};
```

### 2. Always Filter Sensitive Data

```rust
let transformer = ErrorTransformer::new()
    .filter_sensitive_data(true);  // Always enabled
```

### 3. Include Request IDs

```rust
// Add request ID middleware first
// Then error transformer will include it automatically
```

### 4. Map Error Codes for APIs

```rust
let transformer = ErrorTransformer::new()
    .map_error_code("NOT_FOUND", "RESOURCE_NOT_FOUND")
    .map_error_code("VALIDATION_ERROR", "INVALID_INPUT")
    .map_error_code("UNAUTHORIZED", "AUTH_REQUIRED");
```

### 5. Use Validation Errors for Form Inputs

```rust
// Instead of:
Err(Error::BadRequest("Email is invalid".into()))

// Use:
ErrorResponseBuilder::validation_error(vec![
    ValidationError::new("email", "Invalid email format")
        .rule("email")
        .value(email_input)
])
```

### 6. Log Server Errors, Not Client Errors

```rust
let transformer = ErrorTransformer::new()
    .with_logger(|error, ctx, response| {
        if error.is_server_error() {
            // Log full details for debugging
            tracing::error!(
                error = %error,
                path = ctx.request.path,
                "Server error"
            );
        }
        // Don't log 4xx errors (user mistakes)
    });
```

## Common Pitfalls

### ❌ Exposing Internal Details

```rust
// Bad: Exposes database schema
Err(Error::Internal("Column 'users.password_hash' not found".into()))

// Good: Generic message
Err(Error::Internal("Database error".into()))
```

### ❌ Including Sensitive Data in Errors

```rust
// Bad: Includes password in error
Err(Error::BadRequest(format!(
    "Invalid credentials for {}: password={}",
    email, password
)))

// Good: No sensitive data
Err(Error::Unauthorized("Invalid credentials".into()))
```

### ❌ Not Using Validation Errors

```rust
// Bad: Single error message for multiple issues
Err(Error::BadRequest("Email and password are invalid".into()))

// Good: Structured validation errors
ErrorResponseBuilder::validation_error(vec![
    ValidationError::new("email", "Invalid format"),
    ValidationError::new("password", "Too short"),
])
```

## API Reference

### Types

| Type | Description |
|------|-------------|
| `ErrorResponse` | Structured error response |
| `ErrorTransformer` | Central error transformer |
| `ErrorContext` | Request context for errors |
| `ValidationError` | Field validation error |
| `ResponseFormat` | Output format (JSON, HTML, etc.) |
| `ProblemDetails` | RFC 7807 structure |
| `ErrorResponseBuilder` | Quick error response builder |

### ErrorTransformer Methods

| Method | Description |
|--------|-------------|
| `new()` | Create with defaults |
| `development()` | Development preset |
| `production()` | Production preset |
| `api()` | API preset (RFC 7807) |
| `format(fmt)` | Set response format |
| `production_mode(bool)` | Enable/disable production mode |
| `filter_sensitive_data(bool)` | Enable/disable filtering |
| `with_transformer(fn)` | Add custom transformer |
| `with_logger(fn)` | Add custom logger |
| `transform(error, request)` | Transform error to response |

### ErrorResponse Methods

| Method | Description |
|--------|-------------|
| `new(status)` | Create new response |
| `message(msg)` | Set message |
| `code(code)` | Set error code |
| `details(details)` | Set details |
| `with_metadata(k, v)` | Add metadata |
| `with_validation_error(e)` | Add validation error |
| `to_json()` | Convert to JSON string |
| `to_html()` | Convert to HTML string |
| `into_http_response(fmt)` | Convert to HttpResponse |

## Summary

**Key Points:**

1. **Centralize error handling** with `ErrorTransformer`
2. **Use production mode** in production (hides internal details)
3. **Filter sensitive data** always
4. **Use validation errors** for form/input validation
5. **Choose appropriate format** (JSON for APIs, HTML for web)
6. **Log server errors** but not client errors
7. **Include request IDs** for debugging

**Quick Reference:**

```rust
use armature_core::error_transform::{
    ErrorTransformer, ErrorResponse, ValidationError,
    ErrorResponseBuilder, ResponseFormat
};

// Production transformer
let transformer = ErrorTransformer::production();

// Transform error
let response = transformer.transform(&error, &request);

// Quick errors
let response = ErrorResponseBuilder::not_found("User not found");

// Validation errors
let response = ErrorResponseBuilder::validation_error(vec![
    ValidationError::new("email", "Invalid format"),
]);

// Custom response
let response = ErrorResponse::new(400)
    .message("Bad request")
    .code("ERR_400")
    .into_http_response(ResponseFormat::Json);
```

