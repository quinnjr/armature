# armature-cli

CLI tool for the Armature framework.

## Features

- **Project Generation** - Create new projects
- **Code Generation** - Generate controllers, models, etc.
- **Development Server** - Hot reload development
- **Build Tools** - Production builds and optimization

## Installation

```bash
cargo install armature-cli
```

## Commands

### Create Project

```bash
armature new my-app
cd my-app
```

### Generate Code

```bash
# Generate a controller
armature generate controller users

# Generate a model
armature generate model user

# Generate a full CRUD scaffold with fields
armature generate scaffold post --fields "title:string,body:text,published:bool"
```

### Development Server

```bash
# Start with hot reload
armature dev

# Specify port
armature dev --port 8080
```

### Build

```bash
# Development build
armature build

# Production build
armature build --release
```

## Configuration

Create `armature.toml` in your project root:

```toml
[project]
name = "my-app"

[server]
port = 3000
host = "127.0.0.1"

[database]
url = "postgres://localhost/mydb"
```

## License

MIT OR Apache-2.0

