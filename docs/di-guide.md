# Dependency Injection Guide

This guide explains how dependency injection works in Armature and how to use it effectively.

## Overview

Armature provides a complete dependency injection system inspired by Angular. Services are automatically injected into controllers based on their field types, enabling loose coupling and testability.

## Core Concepts

### 1. Injectable Services

Mark a struct with `#[injectable]` to make it available for injection:

```rust
#[injectable]
#[derive(Default, Clone)]
struct DatabaseService {
    connection_string: String,
}
```

**Requirements:**
- Must implement `Default` (for automatic instantiation)
- Must implement `Clone` (for sharing across the application)
- Must be `Send + Sync + 'static` (for thread safety)

### 2. Service Dependencies

Services can depend on other services by declaring them as fields:

```rust
#[injectable]
#[derive(Default, Clone)]
struct UserService {
    database: DatabaseService,  // Will be auto-injected
    logger: LoggerService,       // Will be auto-injected
}
```

### 3. Controllers with DI

Controllers automatically receive injected services:

```rust
#[controller("/users")]
#[derive(Default, Clone)]
struct UserController {
    user_service: UserService,  // Automatically injected!
}

impl UserController {
    // Methods can now use self.user_service
    fn get_users(&self) -> Result<Json<Vec<User>>, Error> {
        let users = self.user_service.find_all();
        Ok(Json(users))
    }
}
```

## How It Works

### Registration Order

The framework automatically handles dependency registration in the correct order:

1. **Imported modules** are registered first (depth-first)
2. **Providers** (services) are registered in declaration order
3. **Controllers** are instantiated with resolved dependencies
4. **Routes** are registered for each controller

### Dependency Resolution

When a controller is created:

1. The framework inspects the controller's fields
2. For each field, it resolves the service from the DI container
3. The controller is constructed with all dependencies injected
4. The controller instance is cached for reuse

### Container Lifecycle

- Services are **singletons** by default
- Once created, the same instance is shared across the application
- This ensures efficient resource usage (e.g., database connections)

## Usage Examples

### Example 1: Simple Service Injection

```rust
use armature_framework::prelude::*;

// Service with no dependencies
#[injectable]
#[derive(Default, Clone)]
struct ConfigService {
    api_url: String,
}

// Controller using the service
#[controller("/api")]
#[derive(Default, Clone)]
struct ApiController {
    config: ConfigService,
}

impl ApiController {
    #[get("/info")]
    async fn info(&self) -> Result<Json<String>, Error> {
        Ok(Json(self.config.api_url.clone()))
    }
}

#[module(
    providers: [ConfigService],
    controllers: [ApiController]
)]
#[derive(Default)]
struct AppModule;
```

### Example 2: Service Chain

```rust
// Level 1: Base service
#[injectable]
#[derive(Default, Clone)]
struct LoggerService;

// Level 2: Service depending on Logger
#[injectable]
#[derive(Default, Clone)]
struct DatabaseService {
    logger: LoggerService,
}

// Level 3: Service depending on Database
#[injectable]
#[derive(Default, Clone)]
struct UserService {
    database: DatabaseService,
}

// Level 4: Controller depending on UserService
#[controller("/users")]
#[derive(Default, Clone)]
struct UserController {
    user_service: UserService,
}

#[module(
    providers: [LoggerService, DatabaseService, UserService],
    controllers: [UserController]
)]
#[derive(Default)]
struct AppModule;
```

The framework ensures all dependencies are resolved in the correct order.

### Example 3: Multiple Dependencies

```rust
#[injectable]
#[derive(Default, Clone)]
struct AuthService;

#[injectable]
#[derive(Default, Clone)]
struct CacheService;

#[injectable]
#[derive(Default, Clone)]
struct EmailService;

#[injectable]
#[derive(Default, Clone)]
struct UserService {
    auth: AuthService,
    cache: CacheService,
    email: EmailService,
}

#[controller("/users")]
#[derive(Default, Clone)]
struct UserController {
    user_service: UserService,
    auth_service: AuthService,  // Can inject same service multiple times
}
```

## Module System

### Provider Declaration

Providers must be declared in the module:

```rust
#[module(
    providers: [ServiceA, ServiceB, ServiceC],
    controllers: [ControllerX, ControllerY]
)]
```

**Order matters for providers:**
- List services with no dependencies first
- Then list services that depend on earlier services
- The framework registers them in declaration order

### Module Imports

Modules can import other modules to access their services:

```rust
#[module(
    providers: [SharedService],
    exports: [SharedService]  // Make available to importers
)]
#[derive(Default)]
struct SharedModule;

#[module(
    providers: [UserService],
    controllers: [UserController],
    imports: [SharedModule]  // Import shared services
)]
#[derive(Default)]
struct UserModule;
```

## Registering Built-in Services

Armature provides many built-in services that you can register in the DI container. Here are examples of how to register commonly used services.

### Health Check Service

```rust
use armature_core::{
    Container, Provider, HealthService, HealthServiceBuilder,
    MemoryHealthIndicator, DiskHealthIndicator, UptimeHealthIndicator,
};

// Method 1: Register a pre-built HealthService using the builder
fn register_health_service(container: &Container) {
    let health_service = HealthServiceBuilder::new()
        .with_defaults()  // Adds memory, disk, and uptime indicators
        .with_info(|info| {
            info.name("my-api")
                .version("1.0.0")
                .description("My REST API")
        })
        .build();

    container.register(health_service);
}

// Method 2: Register with custom indicators only
fn register_custom_health_service(container: &Container) {
    let health_service = HealthServiceBuilder::new()
        .with_indicator(MemoryHealthIndicator::new(0.9))  // 90% threshold
        .with_indicator(UptimeHealthIndicator::default())
        .build();

    container.register(health_service);
}

// Using the health service in a controller
#[controller("/health")]
#[derive(Default, Clone)]
struct HealthController {
    health_service: HealthService,
}

impl HealthController {
    #[get("/")]
    async fn check(&self) -> Result<HttpResponse, Error> {
        let response = self.health_service.check().await;
        Ok(HttpResponse::new(response.status.http_status_code())
            .with_json(&response)?)
    }

    #[get("/live")]
    async fn liveness(&self) -> Result<HttpResponse, Error> {
        let response = self.health_service.liveness().await;
        Ok(HttpResponse::new(response.status.http_status_code())
            .with_json(&response)?)
    }

    #[get("/ready")]
    async fn readiness(&self) -> Result<HttpResponse, Error> {
        let response = self.health_service.readiness().await;
        Ok(HttpResponse::new(response.status.http_status_code())
            .with_json(&response)?)
    }
}
```

### Registering Services in Modules

```rust
use armature_core::{
    Module, Container, ProviderRegistration, ControllerRegistration,
    HealthService, HealthServiceBuilder,
};
use std::any::TypeId;

struct AppModule;

impl Module for AppModule {
    fn providers(&self) -> Vec<ProviderRegistration> {
        vec![
            // Register HealthService with custom configuration
            ProviderRegistration {
                type_id: TypeId::of::<HealthService>(),
                type_name: "HealthService",
                register_fn: |container| {
                    let health_service = HealthServiceBuilder::new()
                        .with_defaults()
                        .with_info(|info| info.name("my-app").version("1.0.0"))
                        .build();
                    container.register(health_service);
                },
            },
            // Register other services...
        ]
    }

    fn controllers(&self) -> Vec<ControllerRegistration> {
        vec![]  // Your controllers
    }

    fn imports(&self) -> Vec<Box<dyn Module>> {
        vec![]
    }

    fn exports(&self) -> Vec<TypeId> {
        vec![TypeId::of::<HealthService>()]  // Export for child modules
    }
}
```

### Using Dynamic Modules for Service Registration

```rust
use armature_core::{DynamicModule, HealthService, HealthServiceBuilder, provider_registration};

// Create a reusable health module
fn create_health_module(app_name: &str, app_version: &str) -> DynamicModule {
    let name = app_name.to_string();
    let version = app_version.to_string();

    DynamicModule::new("HealthModule")
        .with_provider(ProviderRegistration {
            type_id: std::any::TypeId::of::<HealthService>(),
            type_name: "HealthService",
            register_fn: move |container| {
                let health_service = HealthServiceBuilder::new()
                    .with_defaults()
                    .build();
                container.register(health_service);
            },
        })
        .export::<HealthService>()
}

// Use it in your application
let health_module = create_health_module("my-api", "1.0.0");
```

### Database and Cache Services

```rust
use armature_core::{Container, Provider};

// Example: Custom database service wrapper
#[derive(Clone)]
struct DatabaseService {
    connection_string: String,
    // pool: Arc<Pool>  // Your actual connection pool
}

impl Provider for DatabaseService {}

impl DatabaseService {
    pub fn new(connection_string: &str) -> Self {
        Self {
            connection_string: connection_string.to_string(),
        }
    }
}

// Register in container
fn setup_database(container: &Container, connection_string: &str) {
    let db_service = DatabaseService::new(connection_string);
    container.register(db_service);
}

// Example: Cache service with configuration
#[derive(Clone)]
struct CacheService {
    ttl_seconds: u64,
}

impl Provider for CacheService {}

impl CacheService {
    pub fn new(ttl_seconds: u64) -> Self {
        Self { ttl_seconds }
    }
}

fn setup_cache(container: &Container) {
    container.register(CacheService::new(300));  // 5 minute TTL
}
```

## Advanced Patterns

### Constructor Injection

The generated `new_with_di` method is automatically called:

```rust
// Generated automatically by #[controller]
impl UserController {
    pub fn new_with_di(container: &Container) -> Result<Self, Error> {
        Ok(Self {
            user_service: (*container.resolve::<UserService>()?).clone(),
        })
    }
}
```

### Manual DI (for advanced use cases)

You can manually work with the container:

```rust
let container = Container::new();

// Register a service
container.register(MyService::default());

// Register with factory function
container.register_factory(|| {
    MyComplexService::new_with_config("some-config")
});

// Resolve a service
let service = container.resolve::<MyService>()?;

// Check if service exists
if container.has::<MyService>() {
    println!("MyService is registered");
}
```

### Registering Services from Configuration

```rust
use armature_core::{Container, Provider};
use std::env;

#[derive(Clone)]
struct AppConfig {
    database_url: String,
    redis_url: String,
    log_level: String,
}

impl Provider for AppConfig {}

impl AppConfig {
    pub fn from_env() -> Self {
        Self {
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://localhost/app".to_string()),
            redis_url: env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://localhost:6379".to_string()),
            log_level: env::var("LOG_LEVEL")
                .unwrap_or_else(|_| "info".to_string()),
        }
    }
}

fn setup_app(container: &Container) {
    // Load configuration from environment
    let config = AppConfig::from_env();
    container.register(config.clone());

    // Use config to set up other services
    let db_service = DatabaseService::new(&config.database_url);
    container.register(db_service);
}
```

### Testing with DI

DI makes testing easier by allowing mock injection:

```rust
#[cfg(test)]
mod tests {
    #[injectable]
    #[derive(Default, Clone)]
    struct MockDatabaseService {
        // Mock implementation
    }

    #[test]
    fn test_controller() {
        let container = Container::new();
        container.register(MockDatabaseService::default());

        let controller = UserController::new_with_di(&container).unwrap();
        // Test controller with mock dependencies
    }
}
```

## Best Practices

### 1. Keep Services Stateless

Services should be stateless or have immutable state:

```rust
// Good: Stateless
#[injectable]
#[derive(Default, Clone)]
struct UserService {
    db: DatabaseService,  // Shared connection pool
}

// Avoid: Mutable state
#[injectable]
#[derive(Default, Clone)]
struct CounterService {
    count: i32,  // This won't work as expected with Clone
}
```

### 2. Use Descriptive Names

```rust
// Good
#[injectable]
struct UserAuthenticationService;

// Avoid
#[injectable]
struct Service1;
```

### 3. Minimize Dependencies

Keep the dependency graph shallow:

```rust
// Good: 2-3 dependencies max
#[injectable]
struct UserService {
    database: DatabaseService,
    cache: CacheService,
}

// Avoid: Too many dependencies (consider refactoring)
#[injectable]
struct GodService {
    dep1: Service1,
    dep2: Service2,
    // ... 10 more dependencies
}
```

### 4. Interface Segregation

Create focused services with single responsibilities:

```rust
// Good: Focused services
#[injectable]
struct UserRepository;  // Data access

#[injectable]
struct UserValidator;  // Validation logic

#[injectable]
struct UserNotifier;   // Notifications

// Avoid: God object
#[injectable]
struct UserEverything;  // Does everything
```

## Troubleshooting

### "Provider not found" Error

**Cause:** Service not registered in module or wrong type.

**Solution:** Ensure the service is in the `providers` array:

```rust
#[module(
    providers: [MyService],  // Must be listed here!
    controllers: [MyController]
)]
```

### Circular Dependencies

**Cause:** Service A depends on B, B depends on A.

**Solution:** Refactor to break the cycle:

```rust
// Bad: Circular dependency
struct ServiceA { b: ServiceB }
struct ServiceB { a: ServiceA }  // Circular!

// Good: Extract shared dependency
struct ServiceA { shared: SharedService }
struct ServiceB { shared: SharedService }
struct SharedService { /* shared logic */ }
```

### Clone Not Implemented

**Cause:** Service doesn't implement `Clone`.

**Solution:** Add `#[derive(Clone)]` or implement it manually:

```rust
#[injectable]
#[derive(Default, Clone)]  // Add Clone here
struct MyService;
```

## Performance Considerations

### Singleton Pattern

- Services are created once and reused
- No performance overhead after initial creation
- Thread-safe through `Arc` internally

### Clone Overhead

- `Clone` on services is usually cheap (clones Arc pointers)
- For expensive resources, use Arc/Rc internally:

```rust
#[injectable]
#[derive(Default, Clone)]
struct DatabaseService {
    pool: Arc<ConnectionPool>,  // Cheap to clone
}
```

## Future Enhancements

Planned features for the DI system:

- [ ] `@Scope` decorator for request-scoped services
- [ ] `@Factory` for custom instantiation logic
- [ ] `@Lazy` for lazy-loaded services
- [ ] Interface-based injection with traits
- [ ] Conditional providers
- [ ] Provider configuration

## Comparison with Other Frameworks

### vs Spring (Java)
- Similar `@Injectable` / `@Service` concepts
- Similar `@Controller` pattern
- No XML configuration needed

### vs Angular (TypeScript)
- Nearly identical decorator syntax
- Same module system
- Constructor injection works similarly

### vs Actix-web (Rust)
- More explicit DI (vs implicit Data extractors)
- Compile-time safety
- Better testability

## Summary

Armature's DI system provides:

✅ **Automatic injection** based on field types
✅ **Type-safe** resolution at compile time
✅ **Modular** organization with imports/exports
✅ **Testable** through dependency injection
✅ **Performant** with singleton pattern
✅ **Familiar** syntax for Angular/Spring developers

The DI system is the foundation of Armature, enabling clean, maintainable, and testable code.

