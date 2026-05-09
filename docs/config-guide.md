# Configuration Management Guide

This guide explains how to use the configuration system in Armature, inspired by NestJS's @nestjs/config.

## Overview

Armature provides a comprehensive configuration management system through the `armature-config` module. It supports multiple configuration sources, environment variables, validation, and seamless dependency injection integration.

## Features

✅ **Environment Variables** - Load from system environment
✅ **`.env` File Support** - Load from dotenv files
✅ **Multiple Formats** - JSON, TOML, and ENV files
✅ **Type-Safe** - Strongly typed configuration
✅ **Validation** - Built-in validation rules
✅ **DI Integration** - Injectable configuration service
✅ **Prefix Support** - Namespace environment variables
✅ **Hierarchical** - Nested configuration objects

## Installation

Add the config feature to your `Cargo.toml`:

```toml
[dependencies]
armature-framework = { version = "0.1", features = ["config"] }
armature-config = "0.1"
```

## Quick Start

### 1. Create Configuration Service

```rust
use armature_config::ConfigService;

let config = ConfigService::builder()
    .with_prefix("APP".to_string())
    .load_env()
    .load_dotenv(None)
    .build()?;
```

### 2. Access Configuration

```rust
// Get string value
let app_name = config.get_string("app.name")?;

// Get with default
let port = config.get_or("server.port", 3000);

// Get typed value
let debug: bool = config.get("app.debug")?;
```

### 3. Inject into Services

```rust
#[injectable]
#[derive(Clone)]
struct MyService {
    config: ConfigService,
}

impl MyService {
    fn do_something(&self) {
        let api_key = self.config.get_string("api.key").unwrap();
        // Use api_key...
    }
}
```

## Configuration Sources

### Environment Variables

Load from system environment:

```rust
let config = ConfigService::builder()
    .load_env()
    .build()?;

// Access
let value = config.get_string("path")?;
```

**With Prefix:**
```rust
// Loads APP_DATABASE_HOST as "database.host"
let config = ConfigService::builder()
    .with_prefix("APP".to_string())
    .load_env()
    .build()?;
```

### .env Files

Create a `.env` file:
```env
APP_NAME=My Application
APP_PORT=3000
DATABASE_HOST=localhost
DATABASE_PORT=5432
```

Load it:
```rust
let config = ConfigService::builder()
    .load_dotenv(None)  // Loads from .env
    .build()?;

// Or specify path
let config = ConfigService::builder()
    .load_dotenv(Some(".env.production".to_string()))
    .build()?;
```

### JSON Files

Create `config.json`:
```json
{
  "app": {
    "name": "My App",
    "port": 3000
  },
  "database": {
    "host": "localhost",
    "port": 5432
  }
}
```

Load it:
```rust
use armature_config::FileFormat;

let config = ConfigService::builder()
    .add_file("config.json".to_string(), FileFormat::Json)
    .build()?;
```

### TOML Files

Create `config.toml`:
```toml
[app]
name = "My App"
port = 3000

[database]
host = "localhost"
port = 5432
```

Load it:
```rust
let config = ConfigService::builder()
    .add_file("config.toml".to_string(), FileFormat::Toml)
    .build()?;
```

### Multiple Sources

Load from multiple sources (later sources override earlier ones):

```rust
let config = ConfigService::builder()
    .add_file("config.json".to_string(), FileFormat::Json)
    .load_dotenv(None)
    .load_env()
    .build()?;
```

## Type-Safe Configuration

### Define Configuration Structs

```rust
use serde::{Deserialize, Serialize};
use armature_config::Validate;

#[derive(Debug, Deserialize, Serialize)]
struct AppConfig {
    app: ApplicationConfig,
    database: DatabaseConfig,
}

#[derive(Debug, Deserialize, Serialize)]
struct ApplicationConfig {
    name: String,
    port: u16,
    debug: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct DatabaseConfig {
    host: String,
    port: u16,
    username: String,
    password: String,
}
```

### Load and Validate

```rust
// Implement validation
impl Validate for AppConfig {
    fn validate(&self) -> armature_config::Result<()> {
        armature_config::ConfigValidator::not_empty(&self.app.name, "app.name")?;
        armature_config::ConfigValidator::is_port(self.app.port, "app.port")?;
        Ok(())
    }
}

// Load and validate
let manager = config.manager();
let app_config: AppConfig = manager.load_validated()?;
```

## Validation

### Built-in Validators

```rust
use armature_config::ConfigValidator;

// Not empty
ConfigValidator::not_empty(&value, "field_name")?;

// Range check
ConfigValidator::in_range(port, 1, 65535, "port")?;

// One of allowed values
ConfigValidator::one_of(&env, &["dev", "prod"], "environment")?;

// URL validation
ConfigValidator::is_url(&api_url, "api_url")?;

// Email validation
ConfigValidator::is_email(&email, "email")?;

// Port validation
ConfigValidator::is_port(port, "port")?;
```

### Custom Validation

```rust
impl Validate for MyConfig {
    fn validate(&self) -> armature_config::Result<()> {
        // Custom validation logic
        if self.min_value > self.max_value {
            return Err(ConfigError::ValidationError(
                "min_value must be less than max_value".to_string()
            ));
        }

        // Use built-in validators
        ConfigValidator::not_empty(&self.name, "name")?;
        ConfigValidator::in_range(self.timeout, 0, 3600, "timeout")?;

        Ok(())
    }
}
```

## Accessing Configuration

### Basic Access

```rust
// Get value (returns Result)
let name: String = config.get("app.name")?;

// Get with default
let port = config.get_or("app.port", 3000);

// Check if key exists
if config.has("feature.enabled") {
    // Key exists
}
```

### Type-Specific Getters

```rust
// String
let name = config.get_string("app.name")?;

// Integer
let port = config.get_int("server.port")?;

// Boolean
let debug = config.get_bool("app.debug")?;

// Float
let ratio = config.get_float("app.ratio")?;
```

### Nested Values

```rust
// Access nested configuration
let db_host = config.get_string("database.host")?;
let db_port = config.get_int("database.port")?;

// Or load as struct
#[derive(Deserialize)]
struct DatabaseConfig {
    host: String,
    port: i64,
}

let db_config: DatabaseConfig = config.get("database")?;
```

## Dependency Injection

### Register Config Service

```rust
#[injectable]
#[derive(Clone)]
struct ConfigService {
    // ConfigService is itself injectable
}

#[module(
    providers: [ConfigService],
    controllers: [AppController]
)]
struct AppModule;
```

### Inject into Services

```rust
#[injectable]
#[derive(Clone)]
struct UserService {
    config: ConfigService,
}

impl UserService {
    fn connect_database(&self) {
        let host = self.config.get_string("database.host").unwrap();
        let port = self.config.get_int("database.port").unwrap();

        // Connect to database...
    }
}
```

### Inject into Controllers

```rust
#[controller("/api")]
#[derive(Clone)]
struct ApiController {
    config: ConfigService,
}

impl ApiController {
    fn get_version(&self) -> Result<Json<String>, Error> {
        let version = self.config.get_string("app.version")
            .unwrap_or_else(|_| "unknown".to_string());
        Ok(Json(version))
    }
}
```

## Best Practices

### 1. Use Environment-Specific Files

```
.env.development
.env.staging
.env.production
```

```rust
let env = std::env::var("ENV").unwrap_or("development".to_string());
let env_file = format!(".env.{}", env);

let config = ConfigService::builder()
    .load_dotenv(Some(env_file))
    .load_env()  // Override with system env vars
    .build()?;
```

### 2. Provide Defaults

```rust
let port = config.get_or("server.port", 3000);
let host = config.get_or("server.host", "0.0.0.0".to_string());
```

### 3. Validate Early

```rust
// Validate at startup
let app_config: AppConfig = config.manager().load_validated()?;

// Use validated config
let app = Application::create::<AppModule>().await;
// Configuration is available via the DI container
```

### 4. Use Type-Safe Configuration

```rust
// Define strong types
#[derive(Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
    #[serde(default = "default_workers")]
    workers: usize,
}

fn default_workers() -> usize { 4 }

// Load typed config
let server_config: ServerConfig = config.get("server")?;
```

### 5. Namespace with Prefixes

```rust
// All env vars start with APP_
let config = ConfigService::builder()
    .with_prefix("APP".to_string())
    .load_env()
    .build()?;

// APP_DATABASE_HOST becomes "database.host"
```

### 6. Keep Secrets Secure

```rust
// Load secrets from environment, not config files
let api_key = config.get_string("api.key")?;
let db_password = config.get_string("database.password")?;

// Don't commit .env files with secrets!
// Use .env.example instead
```

## Configuration Patterns

### Factory Pattern

```rust
struct DatabaseFactory;

impl DatabaseFactory {
    fn create(config: &ConfigService) -> Result<Database> {
        let host = config.get_string("database.host")?;
        let port = config.get_int("database.port")?;

        Database::connect(&host, port as u16)
    }
}
```

### Feature Flags

```rust
#[derive(Deserialize)]
struct FeatureFlags {
    new_ui: bool,
    beta_features: bool,
    analytics: bool,
}

let features: FeatureFlags = config.get("features")?;

if features.new_ui {
    // Use new UI
}
```

### Multi-Environment Setup

```rust
#[derive(Deserialize)]
struct Environment {
    name: String,      // "development", "production"
    debug: bool,
    log_level: String,
}

let env: Environment = config.get("environment")?;

match env.name.as_str() {
    "production" => setup_production(),
    "development" => setup_development(),
    _ => setup_default(),
}
```

## Testing

### Mock Configuration

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_with_mock_config() {
        let config = ConfigService::new();
        config.manager().set("test.value", "mock_value").unwrap();

        let service = MyService { config };
        assert_eq!(service.get_test_value(), "mock_value");
    }
}
```

### Test-Specific Config

```rust
#[tokio::test]
async fn test_api_endpoint() {
    let config = ConfigService::builder()
        .add_file("config.test.json".to_string(), FileFormat::Json)
        .build()
        .unwrap();

    // Test with config
}
```

## Error Handling

```rust
use armature_config::ConfigError;

match config.get_string("required.key") {
    Ok(value) => println!("Value: {}", value),
    Err(ConfigError::KeyNotFound(key)) => {
        eprintln!("Missing required config: {}", key);
    }
    Err(ConfigError::ValidationError(msg)) => {
        eprintln!("Invalid config: {}", msg);
    }
    Err(e) => {
        eprintln!("Config error: {}", e);
    }
}
```

## Advanced Usage

### Merge Configurations

```rust
let base_config = ConfigService::builder()
    .add_file("config.base.json".to_string(), FileFormat::Json)
    .build()?;

let env_config = ConfigService::builder()
    .add_file("config.prod.json".to_string(), FileFormat::Json)
    .build()?;

base_config.manager().merge(env_config.manager())?;
```

### Dynamic Configuration

```rust
// Update configuration at runtime
config.manager().set("feature.enabled", true)?;

// Check and update
if !config.has("cache.ttl") {
    config.manager().set("cache.ttl", 3600)?;
}
```

### Configuration Watchers (Future)

```rust
// Watch for configuration changes
config.watch("config.json", |new_config| {
    println!("Configuration updated!");
});
```

## Comparison with NestJS

### NestJS
```typescript
@Module({
  imports: [
    ConfigModule.forRoot({
      envFilePath: '.env',
      isGlobal: true,
    }),
  ],
})
export class AppModule {}

@Injectable()
export class AppService {
  constructor(private config: ConfigService) {}

  getPort(): number {
    return this.config.get<number>('PORT');
  }
}
```

### Armature
```rust
#[injectable]
#[derive(Clone)]
struct AppService {
    config: ConfigService,
}

impl AppService {
    fn get_port(&self) -> i64 {
        self.config.get_int("port").unwrap()
    }
}

#[module(
    providers: [ConfigService, AppService],
    controllers: []
)]
struct AppModule;
```

## Summary

Armature's configuration system provides:

✅ **Multiple Sources** - Env, .env, JSON, TOML
✅ **Type-Safe** - Strong typing and validation
✅ **DI Integration** - Seamlessly injectable
✅ **Flexible** - Hierarchical and namespace support
✅ **Production Ready** - Validation and error handling
✅ **NestJS-Like** - Familiar patterns for NestJS users

For complete examples, see `examples/config_example.rs` in the repository.

