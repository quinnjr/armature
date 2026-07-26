# armature-config

Configuration management for the Armature framework.

## Features

- **Multiple Sources** - Environment variables, `.env` files, and config files
- **File Formats** - JSON, TOML, and `.env`
- **Type-Safe** - Get typed values or deserialize into your own structs
- **Nested Keys** - Access nested config with dot paths (`database.host`)

## Installation

```toml
[dependencies]
armature-config = "0.1"
```

## Quick Start

```rust
use armature_config::{ConfigService, FileFormat};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ConfigService::builder()
        .with_prefix("APP".to_string())
        .load_env()
        .add_file("config.toml".to_string(), FileFormat::Toml)
        .build()?;

    // Typed getters; string values (e.g. from env vars) are parse-coerced.
    let port = config.get_int("APP_PORT")?;
    let debug = config.get_bool("APP_DEBUG")?;

    // Dot paths reach into nested config files.
    let db_host = config.get_string("database.host")?;

    println!("Port: {}, debug: {}, db host: {}", port, debug, db_host);
    Ok(())
}
```

You can also use the lower-level [`ConfigManager`] directly:

```rust
use armature_config::ConfigManager;

let manager = ConfigManager::new();
manager.set("app.port", 3000i64)?;
let port: i64 = manager.get("app.port")?;
# Ok::<(), armature_config::ConfigError>(())
```

## Environment Variables

```bash
APP_DATABASE_URL=postgres://localhost/mydb
APP_PORT=3000
APP_DEBUG=true
```

## Config Files

```toml
# config.toml
database_url = "postgres://localhost/mydb"
port = 3000
debug = true
```

## License

MIT OR Apache-2.0

