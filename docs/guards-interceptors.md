# Guards and Interceptors

Armature provides a powerful middleware system through **Guards** and **Interceptors**, inspired by Angular and NestJS.

## Table of Contents

- [Overview](#overview)
- [Guards](#guards)
  - [Built-in Guards](#built-in-guards)
  - [Custom Guards](#custom-guards)
  - [Usage Examples](#guard-usage-examples)
- [Interceptors](#interceptors)
  - [Built-in Interceptors](#built-in-interceptors)
  - [Custom Interceptors](#custom-interceptors)
  - [Usage Examples](#interceptor-usage-examples)
- [Complete Example](#complete-example)

## Overview

### Guards

Guards are used to **protect routes** by determining whether a request should be allowed to proceed. They run **before** the route handler and can:

- Authenticate users
- Check permissions/roles
- Validate API keys
- Implement custom authorization logic

Guards return `Result<bool, Error>`:
- `Ok(true)` - Allow request to proceed
- `Ok(false)` - Block request (should return error instead)
- `Err(e)` - Block request with specific error

### Interceptors

Interceptors are used to **transform requests and responses**. They can:

- Log requests and responses
- Modify response data
- Cache responses
- Add custom headers
- Measure performance
- Handle errors

Interceptors wrap the handler execution and can modify both input and output.

## Guards

### Built-in Guards

#### AuthenticationGuard

Checks for a valid Bearer token in the `Authorization` header.

```rust
use armature_framework::{Guard, AuthenticationGuard, GuardContext};

let guard = AuthenticationGuard;
let context = GuardContext::new(request);

match guard.can_activate(&context).await {
    Ok(true) => println!("Authenticated!"),
    Ok(false) => println!("Not authenticated"),
    Err(e) => println!("Auth error: {}", e),
}
```

**Usage:**
```bash
# Valid request
curl http://localhost:3000/api/protected \
  -H "Authorization: Bearer your-token-here"

# Invalid request (will fail)
curl http://localhost:3000/api/protected
```

#### RolesGuard

Checks if the authenticated user has specific roles.

```rust
use armature_framework::{Guard, RolesGuard, GuardContext};

let guard = RolesGuard::new(vec!["admin".to_string(), "moderator".to_string()]);
let context = GuardContext::new(request);

match guard.can_activate(&context).await {
    Ok(true) => println!("Has required role!"),
    Ok(false) | Err(_) => println!("Insufficient permissions"),
}
```

#### ApiKeyGuard

Validates API keys from the `x-api-key` header.

```rust
use armature_framework::{Guard, ApiKeyGuard, GuardContext};

let valid_keys = vec![
    "key-123".to_string(),
    "key-456".to_string(),
];

let guard = ApiKeyGuard::new(valid_keys);
let context = GuardContext::new(request);

match guard.can_activate(&context).await {
    Ok(true) => println!("Valid API key"),
    Ok(false) | Err(_) => println!("Invalid API key"),
}
```

**Usage:**
```bash
curl http://localhost:3000/api/data \
  -H "x-api-key: key-123"
```

### Custom Guards

Create custom guards by implementing the `Guard` trait:

```rust
use armature_framework::{Guard, GuardContext, Error};
use async_trait::async_trait;

pub struct IpWhitelistGuard {
    allowed_ips: Vec<String>,
}

impl IpWhitelistGuard {
    pub fn new(ips: Vec<String>) -> Self {
        Self { allowed_ips: ips }
    }
}

#[async_trait]
impl Guard for IpWhitelistGuard {
    async fn can_activate(&self, context: &GuardContext) -> Result<bool, Error> {
        // Get client IP from headers or connection
        let client_ip = context
            .get_header("x-forwarded-for")
            .or_else(|| context.get_header("x-real-ip"))
            .ok_or_else(|| Error::Forbidden("Cannot determine client IP".to_string()))?;

        if self.allowed_ips.contains(client_ip) {
            Ok(true)
        } else {
            Err(Error::Forbidden(format!("IP {} not whitelisted", client_ip)))
        }
    }
}
```

#### Function-based Custom Guard

For simpler cases, use `CustomGuard`:

```rust
use armature_framework::{CustomGuard, GuardContext, Error};

let guard = CustomGuard::new(|context: &GuardContext| {
    // Only allow requests on weekdays
    let weekday = chrono::Local::now().weekday();
    if weekday == chrono::Weekday::Sat || weekday == chrono::Weekday::Sun {
        Err(Error::Forbidden("Service only available on weekdays".to_string()))
    } else {
        Ok(true)
    }
});
```

### Guard Usage Examples

#### Protecting a Single Route

```rust
use armature_framework::prelude::*;
use armature_framework::{Guard, AuthenticationGuard, GuardContext};

router.add_route(Route {
    method: HttpMethod::GET,
    path: "/api/protected".to_string(),
    handler: Arc::new(move |req| {
        Box::pin(async move {
            // Apply guard
            let guard = AuthenticationGuard;
            let context = GuardContext::new(req.clone());

            match guard.can_activate(&context).await {
                Ok(true) => {},
                Ok(false) | Err(e) => return Err(e),
            }

            // Handler logic
            Ok(HttpResponse::ok().json(json!({ "data": "protected data" }))?)
        })
    }),
});
```

#### Chaining Multiple Guards

```rust
use armature_framework::{Guard, AuthenticationGuard, RolesGuard, GuardContext};

router.add_route(Route {
    method: HttpMethod::DELETE,
    path: "/api/admin/users/:id".to_string(),
    handler: Arc::new(move |req| {
        Box::pin(async move {
            let context = GuardContext::new(req.clone());

            // Check authentication
            let auth_guard = AuthenticationGuard;
            match auth_guard.can_activate(&context).await {
                Ok(true) => {},
                Ok(false) | Err(e) => return Err(e),
            }

            // Check role
            let role_guard = RolesGuard::new(vec!["admin".to_string()]);
            match role_guard.can_activate(&context).await {
                Ok(true) => {},
                Ok(false) | Err(e) => return Err(e),
            }

            // Handler logic - only reached if both guards pass
            let user_id = req.path_params.get("id").unwrap();
            Ok(HttpResponse::ok().json(json!({ "deleted": user_id }))?)
        })
    }),
});
```

## Interceptors

### Built-in Interceptors

#### LoggingInterceptor

Logs all incoming requests and outgoing responses with timing.

```rust
use armature_framework::{Interceptor, LoggingInterceptor, ExecutionContext};

let interceptor = LoggingInterceptor;

// Output:
// → GET /api/users
// ← GET /api/users - 200 (12.3ms)
```

#### TransformInterceptor

Transforms responses using a custom function.

```rust
use armature_framework::{Interceptor, TransformInterceptor, HttpResponse};

let interceptor = TransformInterceptor::new(|mut response| {
    // Add custom header to all responses
    response.headers.insert(
        "X-Powered-By".to_string(),
        "Armature".to_string(),
    );
    response
});
```

#### CacheInterceptor

Caches responses for a specified TTL (Time To Live).

```rust
use armature_framework::{Interceptor, CacheInterceptor};

let interceptor = CacheInterceptor::new(60); // Cache for 60 seconds
```

### Custom Interceptors

Create custom interceptors by implementing the `Interceptor` trait:

```rust
use armature_framework::{Interceptor, ExecutionContext, HttpResponse, Error};
use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;

pub struct CompressionInterceptor {
    min_size: usize,
}

impl CompressionInterceptor {
    pub fn new(min_size: usize) -> Self {
        Self { min_size }
    }
}

#[async_trait]
impl Interceptor for CompressionInterceptor {
    async fn intercept(
        &self,
        context: ExecutionContext,
        next: Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>,
    ) -> Result<HttpResponse, Error> {
        let mut response = next.await?;

        // Compress if response is large enough
        if response.body.len() > self.min_size {
            // Compress the body (pseudo-code)
            // response.body = compress(response.body);
            response.headers.insert(
                "Content-Encoding".to_string(),
                "gzip".to_string(),
            );
        }

        Ok(response)
    }
}
```

#### Metrics Interceptor Example

```rust
use armature_framework::{Interceptor, ExecutionContext, HttpResponse, Error};
use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct MetricsInterceptor {
    request_count: Arc<AtomicU64>,
    total_duration_ms: Arc<AtomicU64>,
}

impl MetricsInterceptor {
    pub fn new() -> Self {
        Self {
            request_count: Arc::new(AtomicU64::new(0)),
            total_duration_ms: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn get_metrics(&self) -> (u64, u64) {
        (
            self.request_count.load(Ordering::Relaxed),
            self.total_duration_ms.load(Ordering::Relaxed),
        )
    }
}

#[async_trait]
impl Interceptor for MetricsInterceptor {
    async fn intercept(
        &self,
        context: ExecutionContext,
        next: Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>,
    ) -> Result<HttpResponse, Error> {
        let start = std::time::Instant::now();

        let result = next.await;

        let duration = start.elapsed().as_millis() as u64;
        self.request_count.fetch_add(1, Ordering::Relaxed);
        self.total_duration_ms.fetch_add(duration, Ordering::Relaxed);

        result
    }
}
```

### Interceptor Usage Examples

#### Applying Interceptors to Routes

Currently, interceptors need to be manually applied within route handlers. A more integrated approach (similar to NestJS's `@UseInterceptors()`) can be added to the macro system in the future.

```rust
use armature_framework::prelude::*;

router.add_route(Route {
    method: HttpMethod::GET,
    path: "/api/data".to_string(),
    handler: Arc::new(move |req| {
        Box::pin(async move {
            // Simulate interceptor logging
            let start = std::time::Instant::now();
            println!("→ {} {}", req.method, req.path);

            // Handler logic
            let response = HttpResponse::ok().json(json!({ "data": "value" }))?;

            // Log completion
            let duration = start.elapsed();
            println!("← {} {} - {} ({:?})", req.method, req.path, response.status, duration);

            Ok(response)
        })
    }),
});
```

## Complete Example

Here's a complete example demonstrating guards and interceptors:

```rust
use armature_framework::prelude::*;
use armature_framework::{AuthenticationGuard, Guard, GuardContext, RolesGuard};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
struct ApiResponse {
    message: String,
}

#[tokio::main]
async fn main() {
    let mut router = Router::new();

    // Public endpoint - no guards
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/public".to_string(),
        handler: Arc::new(move |req| {
            Box::pin(async move {
                println!("→ {} {}", req.method, req.path);
                let response = HttpResponse::ok().json(ApiResponse {
                    message: "Public data".to_string(),
                })?;
                println!("← {} {} - 200", req.method, req.path);
                Ok(response)
            })
        }),
    });

    // Protected endpoint - authentication required
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/protected".to_string(),
        handler: Arc::new(move |req| {
            Box::pin(async move {
                let guard = AuthenticationGuard;
                let context = GuardContext::new(req.clone());

                match guard.can_activate(&context).await {
                    Ok(true) => {},
                    Ok(false) | Err(e) => return Err(e),
                }

                Ok(HttpResponse::ok().json(ApiResponse {
                    message: "Protected data".to_string(),
                })?)
            })
        }),
    });

    // Admin endpoint - authentication + role required
    router.add_route(Route {
        method: HttpMethod::DELETE,
        path: "/admin/delete".to_string(),
        handler: Arc::new(move |req| {
            Box::pin(async move {
                let context = GuardContext::new(req.clone());

                // Check authentication
                let auth_guard = AuthenticationGuard;
                match auth_guard.can_activate(&context).await {
                    Ok(true) => {},
                    Ok(false) | Err(e) => return Err(e),
                }

                // Check role
                let role_guard = RolesGuard::new(vec!["admin".to_string()]);
                match role_guard.can_activate(&context).await {
                    Ok(true) => {},
                    Ok(false) | Err(e) => return Err(e),
                }

                Ok(HttpResponse::ok().json(ApiResponse {
                    message: "Admin action completed".to_string(),
                })?)
            })
        }),
    });

    let app = Application::new(Container::new(), router);

    println!("Server running on http://localhost:3000");
    app.listen(3000).await.unwrap();
}
```

### Testing the Example

```bash
# 1. Public endpoint (no auth required)
curl http://localhost:3000/public
# → 200 OK

# 2. Protected endpoint (no token - will fail)
curl http://localhost:3000/protected
# → 403 Forbidden

# 3. Protected endpoint (with token)
curl http://localhost:3000/protected \
  -H "Authorization: Bearer my-token"
# → 200 OK

# 4. Admin endpoint (token but no role - will fail in real implementation)
curl -X DELETE http://localhost:3000/admin/delete \
  -H "Authorization: Bearer user-token"
# → 403 Forbidden

# 5. Admin endpoint (admin token)
curl -X DELETE http://localhost:3000/admin/delete \
  -H "Authorization: Bearer admin-token"
# → 200 OK
```

## Best Practices

1. **Guard Order**: Apply guards in order of cost - fast checks first (API key) before expensive ones (database lookups)

2. **Error Messages**: Be specific but don't leak security information:
   ```rust
   // Good
   Err(Error::Forbidden("Invalid credentials".to_string()))

   // Bad - leaks information
   Err(Error::Forbidden("User john@example.com not found".to_string()))
   ```

3. **Guard Composition**: Create reusable guard combinations:
   ```rust
   async fn apply_admin_guards(context: &GuardContext) -> Result<(), Error> {
       let auth = AuthenticationGuard;
       match auth.can_activate(context).await {
           Ok(true) => {},
           _ => return Err(Error::Forbidden("Not authenticated".to_string())),
       }

       let roles = RolesGuard::new(vec!["admin".to_string()]);
       match roles.can_activate(context).await {
           Ok(true) => Ok(()),
           _ => Err(Error::Forbidden("Admin role required".to_string())),
       }
   }
   ```

4. **Interceptor Performance**: Keep interceptors lightweight - avoid heavy computation or I/O

5. **Logging**: Use structured logging in interceptors:
   ```rust
   println!(
       "request_id={} method={} path={} duration_ms={} status={}",
       request_id, method, path, duration, status
   );
   ```

## Future Enhancements

The following features are planned for future releases:

1. **Decorator Syntax**: Apply guards via macros
   ```rust
   #[controller("/api")]
   #[use_guards(AuthenticationGuard)]
   struct ApiController {
       #[get("/admin")]
       #[use_guards(RolesGuard::new(vec!["admin"]))]
       async fn admin_only(&self) -> Result<Json<Data>, Error> {
           // ...
       }
   }
   ```

2. **Global Guards**: Apply guards to all routes
   ```rust
   let app = Application::new(container, router)
       .use_global_guard(LoggingInterceptor)
       .use_global_guard(AuthenticationGuard);
   ```

3. **Interceptor Chaining**: Multiple interceptors with defined order
   ```rust
   #[use_interceptors(LoggingInterceptor, CacheInterceptor)]
   ```

4. **Exception Filters**: Dedicated error handling interceptors
   ```rust
   #[use_filters(HttpExceptionFilter)]
   ```

## See Also

- [Authentication Guide](auth-guide.md) - JWT and OAuth2 integration
- [Dependency Injection](di-guide.md) - Injecting guards and interceptors
- [Middleware](use-middleware-guide.md) - Alternative middleware patterns

