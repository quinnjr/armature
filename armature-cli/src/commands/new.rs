//! New project command.

use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use crate::error::{CliError, CliResult};
use crate::generators::{NameCases, ensure_dir, write_file};
use crate::templates::{DevOpsData, ProjectData, TemplateRegistry};

/// Templates recognized by the project generator.
const KNOWN_TEMPLATES: &[&str] = &[
    "minimal",
    "full",
    "microservice",
    "graphql",
    "grpc",
    "lambda",
    "cloudrun",
];

/// Options controlling the shape of a generated project.
///
/// Threaded from both the interactive wizard (database/feature/docker/ci choices)
/// and the non-interactive `--database`/`--docker`/`--ci` flags.
#[derive(Debug, Clone, Default)]
pub struct NewProjectOptions {
    /// Database backend to configure (`postgres`, `mysql`, `sqlite`, `mongodb`,
    /// `redis`, or `none`). `None`/`"none"` configures no database.
    pub database: Option<String>,
    /// Armature feature keys to enable (e.g. `auth`, `cache`, `queue`).
    pub features: Vec<String>,
    /// Force-include a Dockerfile regardless of the template default.
    pub docker: bool,
    /// Include a GitHub Actions CI workflow.
    pub ci: bool,
}

/// Validate a template name without touching the filesystem.
///
/// Returning `Err` here (before any directory is created) guarantees that an
/// unsupported template never leaves a half-created project directory behind.
pub fn validate_template(template: &str) -> CliResult<()> {
    if KNOWN_TEMPLATES.contains(&template) {
        Ok(())
    } else {
        Err(CliError::InvalidArgument(format!(
            "Unknown template: {}. Available: {}",
            template,
            KNOWN_TEMPLATES.join(", ")
        )))
    }
}

/// Create a new Armature project.
pub async fn run(
    name: &str,
    template: &str,
    skip_git: bool,
    _skip_install: bool,
    opts: &NewProjectOptions,
) -> CliResult<()> {
    // Validate the template BEFORE writing any files so a bad template never
    // leaves a half-created directory behind.
    validate_template(template)?;

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
    generate_project_files(&project_dir, &names, template, opts).await?;
    pb.inc(1);

    // Step 3: Initialize git repository
    if !skip_git {
        pb.set_message("Initializing git repository...");
        init_git(&project_dir)?;
    }
    pb.inc(1);

    // Step 4: Create additional structure based on template
    pb.set_message("Creating project structure...");
    create_project_structure(&project_dir, &names, template, opts)?;
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

/// Resolve the database dependency line for a database key, if any.
fn database_dependency(database: Option<&str>) -> Option<&'static str> {
    match database.map(|d| d.to_lowercase()).as_deref() {
        Some("postgres") | Some("postgresql") => Some(
            "sqlx = { version = \"0.8\", features = [\"runtime-tokio-rustls\", \"postgres\", \"macros\"] }",
        ),
        Some("mysql") => Some(
            "sqlx = { version = \"0.8\", features = [\"runtime-tokio-rustls\", \"mysql\", \"macros\"] }",
        ),
        Some("sqlite") => Some(
            "sqlx = { version = \"0.8\", features = [\"runtime-tokio-rustls\", \"sqlite\", \"macros\"] }",
        ),
        Some("mongodb") => Some("mongodb = \"3.1\""),
        Some("redis") => Some(
            "redis = { version = \"0.27\", features = [\"tokio-comp\", \"connection-manager\"] }",
        ),
        _ => None,
    }
}

/// Map a feature key to either an `armature` feature flag or a standalone crate line.
///
/// Returns `Ok(flag)` for an armature feature flag, or `Err(dep_line)` for a
/// separate crate dependency.
fn feature_dependency(feature: &str) -> Option<Result<&'static str, &'static str>> {
    match feature.to_lowercase().as_str() {
        "auth" => Some(Ok("auth")),
        "ratelimit" | "rate-limiting" | "rate_limiting" => Some(Ok("ratelimit")),
        "cache" | "caching" => Some(Ok("cache")),
        "queue" | "jobs" | "background-jobs" => Some(Ok("queue")),
        "opentelemetry" | "telemetry" => Some(Ok("opentelemetry")),
        "websockets" | "realtime" => None, // in core; no extra dependency
        "storage" | "file-uploads" | "uploads" => Some(Err("armature-storage = \"0.1\"")),
        "mail" | "email" => Some(Err("armature-mail = \"0.1\"")),
        _ => None,
    }
}

/// Template-specific dependency lines (framework crates the template needs).
fn template_dependencies(template: &str) -> Vec<&'static str> {
    match template {
        "graphql" => vec!["async-graphql = \"7.0\""],
        "grpc" => vec!["tonic = \"0.12\"", "prost = \"0.13\""],
        "lambda" => vec!["lambda_http = \"0.13\"", "lambda_runtime = \"0.13\""],
        _ => Vec::new(),
    }
}

/// Build the `armature` dependency line and the extra dependency block from the
/// selected template, database, and features.
fn build_dependencies(template: &str, opts: &NewProjectOptions) -> (String, String) {
    let mut armature_features: Vec<String> = Vec::new();
    let mut extra: Vec<String> = Vec::new();

    // Template-specific deps first.
    for dep in template_dependencies(template) {
        extra.push(dep.to_string());
    }

    // Database dependency.
    if let Some(dep) = database_dependency(opts.database.as_deref()) {
        extra.push(dep.to_string());
    }

    // Feature dependencies (armature feature flags + standalone crates).
    for feature in &opts.features {
        match feature_dependency(feature) {
            Some(Ok(flag)) => armature_features.push(flag.to_string()),
            Some(Err(dep)) => extra.push(dep.to_string()),
            None => {}
        }
    }

    armature_features.sort();
    armature_features.dedup();

    // Kept in sync with the `armature` facade crate's current minor version
    // (see `[package] version` for `armature-framework` in the workspace
    // root Cargo.toml, currently 0.2.x) and with `repl_init_script()` in
    // `commands/repl.rs`, which pins the same `"0.2"` for `:dep armature`.
    let armature_dep = if armature_features.is_empty() {
        "armature = \"0.2\"".to_string()
    } else {
        let quoted: Vec<String> = armature_features
            .iter()
            .map(|f| format!("\"{}\"", f))
            .collect();
        format!(
            "armature = {{ version = \"0.2\", features = [{}] }}",
            quoted.join(", ")
        )
    };

    (armature_dep, extra.join("\n"))
}

/// Select the `main.rs` content for a template.
fn main_rs_for(
    template: &str,
    data: &ProjectData,
    templates: &TemplateRegistry,
) -> CliResult<String> {
    match template {
        "graphql" => Ok(MAIN_GRAPHQL_CONTENT.to_string()),
        "grpc" => Ok(MAIN_GRPC_CONTENT.to_string()),
        "lambda" => Ok(MAIN_LAMBDA_CONTENT.to_string()),
        // minimal / full / microservice / cloudrun all share the controller-based main.
        _ => templates
            .render("main_minimal", data)
            .map_err(CliError::Template),
    }
}

/// Generate project files from templates.
async fn generate_project_files(
    project_dir: &std::path::Path,
    names: &NameCases,
    template: &str,
    opts: &NewProjectOptions,
) -> CliResult<()> {
    let templates = TemplateRegistry::new();

    let description = match template {
        "minimal" => format!("A minimal Armature API - {}", names.pascal),
        "full" => format!("A full-featured Armature API - {}", names.pascal),
        "microservice" => format!("An Armature microservice - {}", names.pascal),
        "graphql" => format!("An Armature GraphQL API - {}", names.pascal),
        "grpc" => format!("An Armature gRPC service - {}", names.pascal),
        "lambda" => format!("An Armature AWS Lambda service - {}", names.pascal),
        "cloudrun" => format!("An Armature Google Cloud Run service - {}", names.pascal),
        _ => format!("{} - Built with Armature", names.pascal),
    };

    let (armature_dep, extra_deps) = build_dependencies(template, opts);

    let data = ProjectData::new(
        names.pascal.clone(),
        names.snake.clone(),
        names.kebab.clone(),
        description,
    )
    .with_armature_dep(armature_dep)
    .with_extra_deps(extra_deps);

    // Generate Cargo.toml
    let cargo_toml = templates
        .render("cargo_toml", &data)
        .map_err(CliError::Template)?;
    write_file(&project_dir.join("Cargo.toml"), &cargo_toml, false)?;

    // Generate main.rs (template-specific)
    let main_rs = main_rs_for(template, &data, &templates)?;
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
///
/// The template is assumed already validated (see [`validate_template`]).
fn create_project_structure(
    project_dir: &std::path::Path,
    names: &NameCases,
    template: &str,
    opts: &NewProjectOptions,
) -> CliResult<()> {
    // Whether the template wants a Dockerfile by default (containerized targets).
    let template_wants_docker = matches!(template, "full" | "microservice" | "cloudrun" | "lambda");

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

            // docker-compose (with db/redis services) accompanies the full template.
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
        }
        "graphql" => {
            ensure_dir(&project_dir.join("src/graphql"))?;
            write_file(
                &project_dir.join("src/graphql/mod.rs"),
                GRAPHQL_MODULE_CONTENT,
                false,
            )?;
        }
        "grpc" => {
            ensure_dir(&project_dir.join("src/grpc"))?;
            ensure_dir(&project_dir.join("proto"))?;
            write_file(
                &project_dir.join("src/grpc/mod.rs"),
                GRPC_MODULE_CONTENT,
                false,
            )?;
            write_file(
                &project_dir.join("proto/service.proto"),
                &GRPC_PROTO_CONTENT.replace("{{name}}", &names.snake),
                false,
            )?;
        }
        "lambda" => {
            // Lambda uses a dedicated main.rs already; add a SAM template.
            write_file(
                &project_dir.join("template.yaml"),
                &LAMBDA_SAM_CONTENT.replace("{{name}}", &names.kebab),
                false,
            )?;
        }
        "cloudrun" => {
            // Cloud Run deploys a container listening on $PORT.
            write_file(
                &project_dir.join("service.yaml"),
                &CLOUDRUN_SERVICE_CONTENT.replace("{{name}}", &names.kebab),
                false,
            )?;
        }
        // Unreachable: templates are validated up front.
        other => {
            return Err(CliError::InvalidArgument(format!(
                "Unknown template: {}",
                other
            )));
        }
    }

    // Dockerfile: template default OR forced by --docker. Rendered once, from the
    // handlebars template (uses the project's binary name).
    if template_wants_docker || opts.docker {
        let dockerfile_path = project_dir.join("Dockerfile");
        if !dockerfile_path.exists() {
            let templates = TemplateRegistry::new();
            let devops = DevOpsData::new(
                names.pascal.clone(),
                names.snake.clone(),
                names.kebab.clone(),
            );
            let dockerfile = templates
                .render("dockerfile", &devops)
                .map_err(CliError::Template)?;
            write_file(&dockerfile_path, &dockerfile, false)?;
            write_file(
                &project_dir.join(".dockerignore"),
                DOCKERIGNORE_CONTENT,
                false,
            )?;
        }
    }

    // CI workflow: opt-in via --ci (or the wizard's CI prompt).
    if opts.ci {
        let templates = TemplateRegistry::new();
        let devops = DevOpsData::new(
            names.pascal.clone(),
            names.snake.clone(),
            names.kebab.clone(),
        );
        let workflow = templates
            .render("github_actions", &devops)
            .map_err(CliError::Template)?;
        write_file(
            &project_dir.join(".github/workflows/ci.yml"),
            &workflow,
            false,
        )?;
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

const DOCKERIGNORE_CONTENT: &str = r#"target/
.git/
.github/
Dockerfile
docker-compose.yml
.env
.env.*
*.md
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

// =============================================================================
// GraphQL template
// =============================================================================

const MAIN_GRAPHQL_CONTENT: &str = r#"//! GraphQL API built with Armature Framework.

use armature::prelude::*;

mod controllers;
mod graphql;

use controllers::health::HealthController;

/// Application module.
#[module(controllers: [HealthController])]
#[derive(Default)]
struct AppModule;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Build the GraphQL schema (see `src/graphql`).
    let schema = graphql::build_schema();
    tracing::info!("GraphQL schema ready: {} types", schema.names().len());

    let app = Application::create::<AppModule>().await;

    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse()
        .unwrap_or(3000);

    println!("🚀 GraphQL server on http://127.0.0.1:{}/graphql", port);
    app.listen(port).await?;
    Ok(())
}
"#;

const GRAPHQL_MODULE_CONTENT: &str = r#"//! GraphQL schema definition.

use async_graphql::{EmptyMutation, EmptySubscription, Object, Schema};

/// Root query object.
pub struct Query;

#[Object]
impl Query {
    /// Simple health field to verify the schema wiring.
    async fn ping(&self) -> &str {
        "pong"
    }
}

/// The application GraphQL schema type.
pub type AppSchema = Schema<Query, EmptyMutation, EmptySubscription>;

/// Build the GraphQL schema.
pub fn build_schema() -> AppSchema {
    Schema::build(Query, EmptyMutation, EmptySubscription).finish()
}
"#;

// =============================================================================
// gRPC template
// =============================================================================

const MAIN_GRPC_CONTENT: &str = r#"//! gRPC service built with Armature Framework.

mod grpc;

use tonic::transport::Server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "50051".to_string())
        .parse()
        .unwrap_or(50051);
    let addr = format!("0.0.0.0:{}", port).parse()?;

    println!("🚀 gRPC server listening on {}", addr);

    Server::builder()
        .add_service(grpc::service())
        .serve(addr)
        .await?;

    Ok(())
}
"#;

const GRPC_MODULE_CONTENT: &str = r#"//! gRPC service implementation.
//!
//! The `.proto` definition lives in `proto/service.proto`. Wire up code
//! generation with `tonic-build` in a `build.rs`, then implement the generated
//! service trait here.

use tonic::{Request, Response, Status};

/// gRPC service handler.
#[derive(Debug, Default)]
pub struct GreeterService;

// Once you have generated code from `proto/service.proto`, implement the
// generated trait for `GreeterService` and return it from `service()`.
impl GreeterService {
    /// Example unary handler.
    pub async fn say_hello(&self, request: Request<()>) -> Result<Response<()>, Status> {
        let _ = request;
        Ok(Response::new(()))
    }
}

/// Build the tonic service to register with the server.
pub fn service() -> GreeterService {
    GreeterService
}
"#;

const GRPC_PROTO_CONTENT: &str = r#"syntax = "proto3";

package {{name}};

service Greeter {
  rpc SayHello (HelloRequest) returns (HelloReply);
}

message HelloRequest {
  string name = 1;
}

message HelloReply {
  string message = 1;
}
"#;

// =============================================================================
// AWS Lambda template
// =============================================================================

const MAIN_LAMBDA_CONTENT: &str = r#"//! AWS Lambda function built with Armature Framework.

use lambda_http::{run, service_fn, Body, Error, Request, Response};

async fn handler(_event: Request) -> Result<Response<Body>, Error> {
    let body = serde_json::json!({ "status": "ok" }).to_string();
    let response = Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .map_err(Box::new)?;
    Ok(response)
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt().with_target(false).without_time().init();
    run(service_fn(handler)).await
}
"#;

const LAMBDA_SAM_CONTENT: &str = r#"AWSTemplateFormatVersion: '2010-09-09'
Transform: AWS::Serverless-2016-10-31
Description: {{name}} - Armature AWS Lambda service

Resources:
  Function:
    Type: AWS::Serverless::Function
    Properties:
      FunctionName: {{name}}
      CodeUri: .
      Handler: bootstrap
      Runtime: provided.al2023
      Architectures: [arm64]
      MemorySize: 128
      Timeout: 30
      Events:
        Api:
          Type: HttpApi
          Properties:
            Path: /{proxy+}
            Method: ANY

Outputs:
  ApiUrl:
    Description: HTTP API endpoint URL
    Value: !Sub "https://${ServerlessHttpApi}.execute-api.${AWS::Region}.amazonaws.com/"
"#;

// =============================================================================
// Google Cloud Run template
// =============================================================================

const CLOUDRUN_SERVICE_CONTENT: &str = r#"apiVersion: serving.knative.dev/v1
kind: Service
metadata:
  name: {{name}}
spec:
  template:
    spec:
      containers:
        - image: gcr.io/PROJECT_ID/{{name}}:latest
          ports:
            - containerPort: 3000
          env:
            - name: RUST_LOG
              value: info
          resources:
            limits:
              cpu: "1"
              memory: 512Mi
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_template_accepts_all_known() {
        for t in KNOWN_TEMPLATES {
            assert!(validate_template(t).is_ok(), "{t} should be valid");
        }
    }

    #[test]
    fn validate_template_rejects_unknown() {
        assert!(validate_template("bogus").is_err());
        assert!(validate_template("").is_err());
    }

    // `armature new --template bogus` never reaches this code (clap's
    // `ValueEnum` on `ProjectTemplate` rejects it first — see
    // `clap_rejects_unknown_template_before_new_runs` in
    // `armature-cli/tests/cli.rs`). This test instead exercises `run()`'s own
    // fn-level ordering guarantee directly: `validate_template` is called
    // before any directory is created, so an unknown template string passed
    // straight to `run` (bypassing clap entirely) must fail *and* must not
    // create the project directory.
    #[tokio::test]
    async fn run_rejects_unknown_template_before_creating_directory() {
        let name = "armature_cli_test_run_rejects_unknown_template_ordering";
        let project_dir = std::path::PathBuf::from(name);
        // Paranoia: make sure a stale directory from a previous failed run
        // doesn't make this test pass for the wrong reason.
        let _ = std::fs::remove_dir_all(&project_dir);

        let opts = NewProjectOptions::default();
        let result = run(name, "bogus", true, true, &opts).await;

        assert!(
            result.is_err(),
            "run() must reject an unknown template before doing any work"
        );
        assert!(
            !project_dir.exists(),
            "no project directory must be created when template validation fails first"
        );
    }

    #[test]
    fn database_dependency_maps_backends() {
        assert!(
            database_dependency(Some("postgres"))
                .unwrap()
                .contains("sqlx")
        );
        assert!(
            database_dependency(Some("postgres"))
                .unwrap()
                .contains("postgres")
        );
        assert!(
            database_dependency(Some("mongodb"))
                .unwrap()
                .contains("mongodb")
        );
        assert!(
            database_dependency(Some("redis"))
                .unwrap()
                .contains("redis")
        );
        assert!(database_dependency(Some("none")).is_none());
        assert!(database_dependency(None).is_none());
    }

    #[test]
    fn build_dependencies_enables_armature_features() {
        let opts = NewProjectOptions {
            database: Some("postgres".to_string()),
            features: vec!["auth".to_string(), "cache".to_string()],
            docker: false,
            ci: false,
        };
        let (armature_dep, extra) = build_dependencies("minimal", &opts);
        assert!(armature_dep.contains("features"));
        assert!(armature_dep.contains("auth"));
        assert!(armature_dep.contains("cache"));
        assert!(extra.contains("sqlx"));
    }

    #[test]
    fn build_dependencies_template_specific() {
        let opts = NewProjectOptions::default();
        let (_, extra) = build_dependencies("graphql", &opts);
        assert!(extra.contains("async-graphql"));

        let (_, extra) = build_dependencies("grpc", &opts);
        assert!(extra.contains("tonic"));
        assert!(extra.contains("prost"));

        let (_, extra) = build_dependencies("lambda", &opts);
        assert!(extra.contains("lambda_http"));
    }

    #[test]
    fn build_dependencies_plain_when_no_features() {
        let opts = NewProjectOptions::default();
        let (armature_dep, extra) = build_dependencies("minimal", &opts);
        assert_eq!(armature_dep, "armature = \"0.2\"");
        assert!(extra.is_empty());
    }

    #[test]
    fn standalone_feature_crates_go_to_extra_deps() {
        let opts = NewProjectOptions {
            features: vec!["storage".to_string(), "mail".to_string()],
            ..Default::default()
        };
        let (armature_dep, extra) = build_dependencies("minimal", &opts);
        // storage/mail are separate crates, not armature feature flags.
        assert_eq!(armature_dep, "armature = \"0.2\"");
        assert!(extra.contains("armature-storage"));
        assert!(extra.contains("armature-mail"));
    }
}
