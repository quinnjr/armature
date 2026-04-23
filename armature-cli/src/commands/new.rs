//! New project command.

use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use crate::error::{CliError, CliResult};
use crate::generators::{NameCases, ensure_dir, write_file};
use crate::templates::{ProjectData, TemplateRegistry};

/// Create a new Armature project.
pub async fn run(name: &str, template: &str, skip_git: bool, _skip_install: bool) -> CliResult<()> {
    let names = NameCases::from(name);
    let project_dir = PathBuf::from(&names.kebab);

    // Check if directory already exists
    if project_dir.exists() {
        return Err(CliError::FileExists(project_dir.display().to_string()));
    }

    println!(
        "  {} Creating new Armature project: {}",
        "→".cyan().bold(),
        name.cyan()
    );
    println!(
        "  {} Using template: {}",
        "→".cyan().bold(),
        template.cyan()
    );
    println!();

    // Create progress bar
    let pb = ProgressBar::new(5);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("█▓░"),
    );
    pb.enable_steady_tick(Duration::from_millis(100));

    // Step 1: Create project directory
    pb.set_message("Creating project directory...");
    ensure_dir(&project_dir)?;
    ensure_dir(&project_dir.join("src"))?;
    ensure_dir(&project_dir.join("src/controllers"))?;
    pb.inc(1);

    // Step 2: Generate project files based on template
    pb.set_message("Generating project files...");
    generate_project_files(&project_dir, &names, template).await?;
    pb.inc(1);

    // Step 3: Initialize git repository
    if !skip_git {
        pb.set_message("Initializing git repository...");
        init_git(&project_dir)?;
    }
    pb.inc(1);

    // Step 4: Create additional structure based on template
    pb.set_message("Creating project structure...");
    create_project_structure(&project_dir, template)?;
    pb.inc(1);

    // Step 5: Display completion message
    pb.set_message("Finalizing...");
    pb.inc(1);

    pb.finish_and_clear();

    // Print success message
    println!("{}", "✓ Project created successfully!".green().bold());
    println!();
    println!("  {} cd {}", "→".cyan(), names.kebab);
    println!("  {} cargo run", "→".cyan());
    println!();
    println!("  Or use the Armature CLI:");
    println!("  {} armature dev", "→".cyan());
    println!();
    println!("  Generate code:");
    println!("  {} armature generate controller <name>", "→".cyan());
    println!("  {} armature generate service <name>", "→".cyan());
    println!();

    Ok(())
}

/// Generate project files from templates.
async fn generate_project_files(
    project_dir: &std::path::Path,
    names: &NameCases,
    template: &str,
) -> CliResult<()> {
    let templates = TemplateRegistry::new();

    let description = match template {
        "minimal" => format!("A minimal Armature API - {}", names.pascal),
        "full" => format!("A full-featured Armature API - {}", names.pascal),
        "microservice" => format!("An Armature microservice - {}", names.pascal),
        _ => format!("{} - Built with Armature", names.pascal),
    };

    let data = ProjectData::new(
        names.pascal.clone(),
        names.snake.clone(),
        names.kebab.clone(),
        description,
    );

    // Generate Cargo.toml
    let cargo_toml = templates
        .render("cargo_toml", &data)
        .map_err(CliError::Template)?;
    write_file(&project_dir.join("Cargo.toml"), &cargo_toml, false)?;

    // Generate main.rs
    let main_rs = templates
        .render("main_minimal", &data)
        .map_err(CliError::Template)?;
    write_file(&project_dir.join("src/main.rs"), &main_rs, false)?;

    // Generate .env.example
    let env_example = templates
        .render("env_example", &data)
        .map_err(CliError::Template)?;
    write_file(&project_dir.join(".env.example"), &env_example, false)?;

    // Generate README.md
    let readme = templates
        .render("readme", &data)
        .map_err(CliError::Template)?;
    write_file(&project_dir.join("README.md"), &readme, false)?;

    // Generate .gitignore
    write_file(&project_dir.join(".gitignore"), GITIGNORE_CONTENT, false)?;

    // Generate health controller
    write_file(
        &project_dir.join("src/controllers/mod.rs"),
        "pub mod health;\n",
        false,
    )?;
    write_file(
        &project_dir.join("src/controllers/health.rs"),
        HEALTH_CONTROLLER_CONTENT,
        false,
    )?;

    Ok(())
}

/// Initialize git repository.
fn init_git(project_dir: &std::path::Path) -> CliResult<()> {
    let status = Command::new("git")
        .args(["init"])
        .current_dir(project_dir)
        .output();

    match status {
        Ok(output) if output.status.success() => Ok(()),
        Ok(_) => {
            // Git init failed but we continue anyway
            eprintln!("  {} Could not initialize git repository", "⚠".yellow());
            Ok(())
        }
        Err(_) => {
            // Git not installed
            eprintln!(
                "  {} Git not found, skipping repository initialization",
                "⚠".yellow()
            );
            Ok(())
        }
    }
}

/// Create additional project structure based on template.
fn create_project_structure(project_dir: &std::path::Path, template: &str) -> CliResult<()> {
    match template {
        "minimal" => {
            // Minimal template - already created
        }
        "full" => {
            // Full template - add more directories
            ensure_dir(&project_dir.join("src/services"))?;
            ensure_dir(&project_dir.join("src/middleware"))?;
            ensure_dir(&project_dir.join("src/guards"))?;
            ensure_dir(&project_dir.join("src/models"))?;
            ensure_dir(&project_dir.join("tests"))?;

            write_file(
                &project_dir.join("src/services/mod.rs"),
                "// Services go here\n",
                false,
            )?;
            write_file(
                &project_dir.join("src/middleware/mod.rs"),
                "// Middleware go here\n",
                false,
            )?;
            write_file(
                &project_dir.join("src/guards/mod.rs"),
                "// Guards go here\n",
                false,
            )?;
            write_file(
                &project_dir.join("src/models/mod.rs"),
                "// Models go here\n",
                false,
            )?;

            // Add Dockerfile
            write_file(&project_dir.join("Dockerfile"), DOCKERFILE_CONTENT, false)?;

            // Add docker-compose.yml
            write_file(
                &project_dir.join("docker-compose.yml"),
                DOCKER_COMPOSE_CONTENT,
                false,
            )?;
        }
        "microservice" => {
            // Microservice template
            ensure_dir(&project_dir.join("src/handlers"))?;
            ensure_dir(&project_dir.join("src/jobs"))?;

            write_file(
                &project_dir.join("src/handlers/mod.rs"),
                "// Job handlers go here\n",
                false,
            )?;
            write_file(
                &project_dir.join("src/jobs/mod.rs"),
                "// Job definitions go here\n",
                false,
            )?;

            // Add Dockerfile
            write_file(&project_dir.join("Dockerfile"), DOCKERFILE_CONTENT, false)?;
        }
        _ => {
            return Err(CliError::InvalidArgument(format!(
                "Unknown template: {}. Available: minimal, full, microservice",
                template
            )));
        }
    }

    Ok(())
}

// =============================================================================
// STATIC CONTENT
// =============================================================================

const GITIGNORE_CONTENT: &str = r#"# Generated by Cargo
/target/

# Remove Cargo.lock from gitignore if creating an executable, leave it for libraries
# Cargo.lock

# Environment files
.env
.env.local
.env.*.local

# IDE
.idea/
.vscode/
*.swp
*.swo
*~

# OS files
.DS_Store
Thumbs.db

# Debug
*.pdb

# Coverage
*.profraw
coverage/
"#;

const HEALTH_CONTROLLER_CONTENT: &str = r#"//! Health check controller.

use armature::prelude::*;

/// Health check controller for liveness and readiness probes.
#[controller("/health")]
#[derive(Default)]
pub struct HealthController;

impl HealthController {
    /// Liveness probe - is the service running?
    #[get("/")]
    pub async fn health(&self, _req: HttpRequest) -> Result<HttpResponse, Error> {
        HttpResponse::ok().with_json(&serde_json::json!({
            "status": "ok",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    }

    /// Readiness probe - is the service ready to accept traffic?
    #[get("/ready")]
    pub async fn ready(&self, _req: HttpRequest) -> Result<HttpResponse, Error> {
        // TODO: Add actual readiness checks (database, cache, etc.)
        HttpResponse::ok().with_json(&serde_json::json!({
            "status": "ready",
            "checks": {
                "database": "ok",
                "cache": "ok"
            }
        }))
    }
}
"#;

const DOCKERFILE_CONTENT: &str = r#"# Build stage
FROM rust:1.80-slim as builder

WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Create dummy main.rs for dependency caching
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build dependencies
RUN cargo build --release && rm -rf src

# Copy source code
COPY src ./src

# Build the application
RUN touch src/main.rs && cargo build --release

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /app/target/release/app /app/app

# Set environment variables
ENV RUST_LOG=info
ENV PORT=3000

EXPOSE 3000

CMD ["/app/app"]
"#;

const DOCKER_COMPOSE_CONTENT: &str = r#"version: '3.8'

services:
  app:
    build: .
    ports:
      - "3000:3000"
    environment:
      - RUST_LOG=info
      - PORT=3000
      # - DATABASE_URL=postgres://user:password@db:5432/app
      # - REDIS_URL=redis://redis:6379
    # depends_on:
    #   - db
    #   - redis

  # db:
  #   image: postgres:16-alpine
  #   environment:
  #     POSTGRES_USER: user
  #     POSTGRES_PASSWORD: password
  #     POSTGRES_DB: app
  #   volumes:
  #     - postgres_data:/var/lib/postgresql/data
  #   ports:
  #     - "5432:5432"

  # redis:
  #   image: redis:7-alpine
  #   ports:
  #     - "6379:6379"

# volumes:
#   postgres_data:
"#;
