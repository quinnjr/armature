# HTTP Status Codes and Error Handling

Armature provides comprehensive HTTP status code support and structured error handling for building robust web applications.

## Table of Contents

- [HttpStatus Enum](#httpstatus-enum)
- [Error Variants](#error-variants)
- [Usage Examples](#usage-examples)
- [Best Practices](#best-practices)

## HttpStatus Enum

The `HttpStatus` enum provides type-safe access to all standard HTTP status codes defined in RFC 7231, RFC 6585, and related standards.

### Available Status Codes

#### 1xx Informational
- `Continue` (100)
- `SwitchingProtocols` (101)
- `Processing` (102)
- `EarlyHints` (103)

#### 2xx Success
- `Ok` (200)
- `Created` (201)
- `Accepted` (202)
- `NonAuthoritativeInformation` (203)
- `NoContent` (204)
- `ResetContent` (205)
- `PartialContent` (206)
- `MultiStatus` (207)
- `AlreadyReported` (208)
- `ImUsed` (226)

#### 3xx Redirection
- `MultipleChoices` (300)
- `MovedPermanently` (301)
- `Found` (302)
- `SeeOther` (303)
- `NotModified` (304)
- `UseProxy` (305)
- `TemporaryRedirect` (307)
- `PermanentRedirect` (308)

#### 4xx Client Errors
- `BadRequest` (400)
- `Unauthorized` (401)
- `PaymentRequired` (402)
- `Forbidden` (403)
- `NotFound` (404)
- `MethodNotAllowed` (405)
- `NotAcceptable` (406)
- `ProxyAuthenticationRequired` (407)
- `RequestTimeout` (408)
- `Conflict` (409)
- `Gone` (410)
- `LengthRequired` (411)
- `PreconditionFailed` (412)
- `PayloadTooLarge` (413)
- `UriTooLong` (414)
- `UnsupportedMediaType` (415)
- `RangeNotSatisfiable` (416)
- `ExpectationFailed` (417)
- `ImATeapot` (418)
- `MisdirectedRequest` (421)
- `UnprocessableEntity` (422)
- `Locked` (423)
- `FailedDependency` (424)
- `TooEarly` (425)
- `UpgradeRequired` (426)
- `PreconditionRequired` (428)
- `TooManyRequests` (429)
- `RequestHeaderFieldsTooLarge` (431)
- `UnavailableForLegalReasons` (451)

#### 5xx Server Errors
- `InternalServerError` (500)
- `NotImplemented` (501)
- `BadGateway` (502)
- `ServiceUnavailable` (503)
- `GatewayTimeout` (504)
- `HttpVersionNotSupported` (505)
- `VariantAlsoNegotiates` (506)
- `InsufficientStorage` (507)
- `LoopDetected` (508)
- `NotExtended` (510)
- `NetworkAuthenticationRequired` (511)

### HttpStatus Methods

```rust
use armature_framework::HttpStatus;

let status = HttpStatus::Ok;

// Get numeric code
assert_eq!(status.code(), 200);

// Get reason phrase
assert_eq!(status.reason(), "OK");

// Display format
assert_eq!(status.to_string(), "200 OK");

// Category checks
assert!(status.is_success());
assert!(!status.is_error());

// Create from code
let status = HttpStatus::from_code(404).unwrap();
assert_eq!(status, HttpStatus::NotFound);
```

## Error Variants

The `Error` enum provides typed error variants for all common HTTP error scenarios (4xx and 5xx status codes).

### 4xx Client Error Variants

```rust
use armature_framework::Error;

// Validation errors
Error::BadRequest("Invalid input".to_string())
Error::UnprocessableEntity("Invalid JSON schema".to_string())

// Authentication errors
Error::Unauthorized("Invalid credentials".to_string())
Error::Forbidden("Access denied".to_string())

// Resource errors
Error::NotFound("User not found".to_string())
Error::Gone("Resource permanently deleted".to_string())
Error::Conflict("Resource already exists".to_string())

// Rate limiting
Error::TooManyRequests("Rate limit exceeded".to_string())

// Special
Error::ImATeapot("Easter egg!".to_string())
```

### 5xx Server Error Variants

```rust
use armature_framework::Error;

// General server errors
Error::Internal("Unexpected error".to_string())
Error::InternalServerError("Database connection failed".to_string())

// Service errors
Error::ServiceUnavailable("Database is down".to_string())
Error::BadGateway("Upstream service failed".to_string())
Error::GatewayTimeout("Upstream service timeout".to_string())

// Implementation
Error::NotImplemented("Feature coming soon".to_string())
```

### Error Methods

```rust
use armature_framework::Error;

let error = Error::NotFound("User not found".to_string());

// Get status code
assert_eq!(error.status_code(), 404);

// Get HttpStatus enum
assert_eq!(error.http_status(), HttpStatus::NotFound);

// Check error category
assert!(error.is_client_error());
assert!(!error.is_server_error());
```

## Usage Examples

### Basic Error Handling

```rust
use armature_framework::prelude::*;
use armature_framework::{Error, HttpStatus};

#[controller("/api")]
struct UserController;

impl UserController {
    fn get_user(&self, id: u32) -> Result<Json<User>, Error> {
        if id == 0 {
            return Err(Error::BadRequest("ID must be greater than 0".to_string()));
        }

        let user = database.find_user(id)
            .ok_or_else(|| Error::NotFound(format!("User {} not found", id)))?;

        Ok(Json(user))
    }

    fn create_user(&self, user: User) -> Result<Json<User>, Error> {
        // Check if user already exists
        if database.user_exists(&user.email) {
            return Err(Error::Conflict("User with this email already exists".to_string()));
        }

        // Validate input
        if !user.email.contains('@') {
            return Err(Error::UnprocessableEntity("Invalid email format".to_string()));
        }

        let created = database.create_user(user)
            .map_err(|e| Error::InternalServerError(e.to_string()))?;

        Ok(Json(created))
    }
}
```

### Custom Error Responses

```rust
use armature_framework::prelude::*;
use armature_framework::{Error, HttpStatus};
use serde::Serialize;

#[derive(Serialize)]
struct ErrorResponse {
    status: u16,
    error: String,
    message: String,
    timestamp: String,
}

fn handle_error(error: Error) -> HttpResponse {
    let status = error.status_code();
    let response = ErrorResponse {
        status,
        error: error.http_status().reason().to_string(),
        message: error.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    HttpResponse {
        status,
        headers: std::collections::HashMap::new(),
        body: serde_json::to_vec(&response).unwrap(),
    }
}

// In route handler
router.add_route(Route {
    method: HttpMethod::GET,
    path: "/api/users/:id".to_string(),
    handler: Arc::new(move |req| {
        Box::pin(async move {
            match controller.get_user(id) {
                Ok(user) => user.into_response(),
                Err(e) => Ok(handle_error(e)),
            }
        })
    }),
});
```

### Rate Limiting Example

```rust
use armature_framework::prelude::*;
use armature_framework::Error;
use std::sync::Arc;
use tokio::sync::Mutex;

#[injectable]
struct RateLimiter {
    requests: Arc<Mutex<HashMap<String, (u32, Instant)>>>,
    max_requests: u32,
    window: Duration,
}

impl RateLimiter {
    async fn check(&self, ip: &str) -> Result<(), Error> {
        let mut requests = self.requests.lock().await;
        let now = Instant::now();

        match requests.get_mut(ip) {
            Some((count, start)) if now.duration_since(*start) < self.window => {
                if *count >= self.max_requests {
                    return Err(Error::TooManyRequests(
                        format!("Rate limit exceeded. Max {} requests per {:?}",
                                self.max_requests, self.window)
                    ));
                }
                *count += 1;
            }
            _ => {
                requests.insert(ip.to_string(), (1, now));
            }
        }

        Ok(())
    }
}
```

### Service Unavailable Example

```rust
use armature_framework::prelude::*;
use armature_framework::Error;

#[injectable]
struct HealthChecker {
    database: DatabaseService,
    cache: CacheService,
}

impl HealthChecker {
    async fn check_health(&self) -> Result<(), Error> {
        // Check database
        if !self.database.is_healthy().await {
            return Err(Error::ServiceUnavailable(
                "Database is unavailable".to_string()
            ));
        }

        // Check cache
        if !self.cache.is_healthy().await {
            return Err(Error::ServiceUnavailable(
                "Cache is unavailable".to_string()
            ));
        }

        Ok(())
    }
}

// In route handler
router.add_route(Route {
    method: HttpMethod::GET,
    path: "/health".to_string(),
    handler: Arc::new(move |_req| {
        let checker = health_checker.clone();
        Box::pin(async move {
            checker.check_health().await?;
            Ok(HttpResponse::ok().json(json!({ "status": "healthy" }))?)
        })
    }),
});
```

### All Status Codes Example

```rust
use armature_framework::prelude::*;
use armature_framework::{Error, HttpStatus};

#[controller("/api")]
struct ItemController {
    service: ItemService,
}

impl ItemController {
    fn handle_request(&self, id: u32) -> Result<Json<Item>, Error> {
        match id {
            // Success cases
            1 => Ok(Json(Item { id: 1, name: "Item 1".to_string() })),

            // 4xx Client Errors
            400 => Err(Error::BadRequest("Invalid request".to_string())),
            401 => Err(Error::Unauthorized("Authentication required".to_string())),
            403 => Err(Error::Forbidden("Access denied".to_string())),
            404 => Err(Error::NotFound("Item not found".to_string())),
            409 => Err(Error::Conflict("Item already exists".to_string())),
            422 => Err(Error::UnprocessableEntity("Invalid data".to_string())),
            429 => Err(Error::TooManyRequests("Rate limit exceeded".to_string())),

            // 5xx Server Errors
            500 => Err(Error::InternalServerError("Server error".to_string())),
            501 => Err(Error::NotImplemented("Not implemented".to_string())),
            502 => Err(Error::BadGateway("Upstream error".to_string())),
            503 => Err(Error::ServiceUnavailable("Service down".to_string())),

            _ => Err(Error::NotFound("Unknown ID".to_string())),
        }
    }
}
```

## Best Practices

### 1. Use Specific Error Types

```rust
// Good - specific error
Error::NotFound("User not found".to_string())
Error::Conflict("Email already registered".to_string())

// Bad - generic error
Error::Internal("Something went wrong".to_string())
```

### 2. Provide Helpful Error Messages

```rust
// Good - descriptive message
Error::UnprocessableEntity(
    "Password must be at least 8 characters and contain a number".to_string()
)

// Bad - vague message
Error::BadRequest("Invalid input".to_string())
```

### 3. Don't Leak Sensitive Information

```rust
// Good - safe message
Error::InternalServerError("Database operation failed".to_string())

// Bad - leaks implementation details
Error::InternalServerError(
    "PostgreSQL connection to db.internal.company.com:5432 failed".to_string()
)
```

### 4. Use Appropriate Status Codes

```rust
// Create resource - 201 Created
HttpResponse::created().json(user)?

// Update resource - 200 OK
HttpResponse::ok().json(updated_user)?

// Delete resource - 204 No Content
HttpResponse::no_content()

// Resource not found - 404
Err(Error::NotFound("Resource not found".to_string()))

// Validation failed - 422
Err(Error::UnprocessableEntity("Invalid email".to_string()))
```

### 5. Handle Errors Consistently

```rust
// Create a centralized error handler
fn handle_error(error: Error) -> HttpResponse {
    let status = error.status_code();

    // Log server errors
    if error.is_server_error() {
        eprintln!("Server error: {}", error);
    }

    // Create consistent response format
    let body = json!({
        "error": {
            "code": status,
            "message": error.to_string(),
            "type": error.http_status().reason(),
        }
    });

    HttpResponse {
        status,
        headers: HashMap::new(),
        body: serde_json::to_vec(&body).unwrap(),
    }
}
```

### 6. Use Result Propagation

```rust
// Good - use ? operator
fn create_user(&self, data: UserData) -> Result<User, Error> {
    let validated = self.validate(data)?;
    let user = self.database.create(validated)?;
    self.email_service.send_welcome(user.email)?;
    Ok(user)
}

// Bad - manual error handling
fn create_user(&self, data: UserData) -> Result<User, Error> {
    match self.validate(data) {
        Ok(validated) => match self.database.create(validated) {
            Ok(user) => match self.email_service.send_welcome(user.email) {
                Ok(_) => Ok(user),
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        },
        Err(e) => Err(e),
    }
}
```

## Testing Error Responses

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_not_found_error() {
        let error = Error::NotFound("User not found".to_string());
        assert_eq!(error.status_code(), 404);
        assert!(error.is_client_error());
        assert!(!error.is_server_error());
    }

    #[tokio::test]
    async fn test_rate_limit_error() {
        let error = Error::TooManyRequests("Rate limit exceeded".to_string());
        assert_eq!(error.status_code(), 429);
        assert_eq!(error.http_status(), HttpStatus::TooManyRequests);
    }

    #[tokio::test]
    async fn test_error_response() {
        let result = controller.get_user(999);
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert_eq!(error.status_code(), 404);
    }
}
```

## See Also

- [Guards & Interceptors](guards-interceptors.md) - Middleware for error handling
- [Validation](validation-guide.md) - Input validation patterns
- [Logging](logging-guide.md) - Error logging strategies

