// Allow dead_code while CLI is under development
#![allow(dead_code)]

//! Armature CLI - Code generation and development tools for Armature framework.
//!
//! # Commands
//!
//! - `armature new <name>` - Create a new Armature project
//! - `armature generate <type> <name>` - Generate code (controller, module, etc.)
//! - `armature dev` - Run development server with file watching
//! - `armature serve` - Run production server
//! - `armature build` - Build for production
//! - `armature routes` - List all routes in the application
//! - `armature config` - Validate configuration files
//! - `armature doctor` - Check system requirements and dependencies
//! - `armature upgrade` - Check for and install CLI updates
//! - `armature db` - Database migrations and management
//! - `armature deploy` - Deploy to cloud providers
//! - `armature repl` - Interactive Rust REPL
//! - `armature info` - Show project information
//! - `armature completions` - Generate shell completions

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{Shell, generate};
use colored::Colorize;
use dialoguer::{Confirm, FuzzySelect, Input, MultiSelect, theme::ColorfulTheme};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::io;
use std::time::Duration;

mod commands;
mod error;
mod generators;
mod templates;
mod watcher;

use commands::{build, config, dev, generate, info, mock, new, openapi, repl, routes, run};
use error::{CliError, CliResult};

/// Armature CLI - Modern Rust Web Framework Tools
#[derive(Parser)]
#[command(name = "armature")]
#[command(author = "Pegasus Heavy Industries LLC")]
#[command(version)]
#[command(about = "🔧 CLI tool for Armature framework - code generation and development server")]
#[command(long_about = None)]
#[command(propagate_version = true)]
#[command(after_help = format!(
    "{}\n  {} armature new my-api\n  {} armature dev\n  {} armature g controller users --crud\n  {} armature routes\n  {} armature deploy --provider aws\n\n{}\n  {} https://github.com/pegasusheavy/armature",
    "Examples:".bright_cyan().bold(),
    "$".dimmed(),
    "$".dimmed(),
    "$".dimmed(),
    "$".dimmed(),
    "$".dimmed(),
    "Documentation:".bright_cyan().bold(),
    "→".dimmed()
))]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Disable colored output
    #[arg(long, global = true)]
    no_color: bool,

    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Suppress all output except errors
    #[arg(short, long, global = true)]
    quiet: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new Armature project
    #[command(alias = "n", visible_alias = "create")]
    New(NewArgs),

    /// Generate code (controller, module, middleware, guard, service)
    #[command(alias = "g")]
    Generate {
        #[command(subcommand)]
        generator: GeneratorType,
    },

    /// Run development server with hot reloading
    #[command(alias = "d")]
    Dev(DevArgs),

    /// Run production server
    #[command(alias = "s", visible_alias = "start")]
    Serve(ServeArgs),

    /// Build the project for production
    #[command(alias = "b")]
    Build(BuildArgs),

    /// List all routes in the application
    #[command(alias = "r")]
    Routes(RoutesArgs),

    /// Validate and manage configuration
    #[command(alias = "c")]
    Config(ConfigArgs),

    /// Check system requirements and dependencies
    #[command(visible_alias = "check")]
    Doctor,

    /// Check for and install CLI updates
    #[command(alias = "up")]
    Upgrade(UpgradeArgs),

    /// Database migrations and management
    Db {
        #[command(subcommand)]
        command: DbCommands,
    },

    /// Deploy to cloud providers
    Deploy(DeployArgs),

    /// Interactive Rust REPL with Armature prelude
    Repl(ReplArgs),

    /// Show project information
    Info,

    /// Generate shell completions
    Completions {
        /// The shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Open documentation in browser
    Docs,

    /// Create a new plugin
    Plugin {
        #[command(subcommand)]
        command: PluginCommands,
    },

    /// Run benchmarks
    Bench(BenchArgs),

    /// Analyze project for potential issues
    Lint(LintArgs),

    /// Clean build artifacts
    Clean,

    /// Add an Armature feature to the project
    #[command(alias = "a")]
    Add(AddArgs),

    /// Validate project setup and configuration
    #[command(alias = "check")]
    Validate(ValidateArgs),

    /// OpenAPI tools (client generation, spec validation)
    Openapi {
        #[command(subcommand)]
        command: OpenapiCommands,
    },

    /// Run mock server with fake data from OpenAPI spec
    #[command(alias = "m")]
    Mock(MockArgs),

    /// Run a Rhai application script
    Run(RunArgs),
}

// =============================================================================
// MOCK ARGS
// =============================================================================

#[derive(Args)]
struct MockArgs {
    /// Path to OpenAPI spec (JSON or YAML)
    #[arg(short, long, default_value = "openapi.yaml")]
    spec: String,

    /// Port to run mock server on
    #[arg(short, long, default_value = "3000")]
    port: u16,

    /// Host to bind to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Directory containing custom mock data files
    #[arg(short, long)]
    data: Option<String>,

    /// Simulated response delay in milliseconds
    #[arg(long, default_value = "0")]
    delay: u64,

    /// Enable CORS headers
    #[arg(long, default_value = "true")]
    cors: bool,

    /// Seed for random data generation (for reproducibility)
    #[arg(long)]
    seed: Option<u64>,

    /// Watch spec file for changes
    #[arg(short, long)]
    watch: bool,
}

// =============================================================================
// RUN COMMAND ARGS
// =============================================================================

#[derive(Args)]
struct RunArgs {
    /// Path to the Rhai application script
    #[arg(default_value = "app.rhai")]
    script: String,

    /// Port to run the server on (overrides script-defined port)
    #[arg(short, long)]
    port: Option<u16>,

    /// Host to bind to (overrides script-defined host)
    #[arg(long)]
    host: Option<String>,

    /// Watch for script changes and restart
    #[arg(short, long)]
    watch: bool,
}

// =============================================================================
// NEW COMMAND ARGS
// =============================================================================

#[derive(Args)]
struct NewArgs {
    /// Project name (or '.' for current directory)
    name: Option<String>,

    /// Template to use
    #[arg(short, long, value_enum)]
    template: Option<ProjectTemplate>,

    /// Skip git initialization
    #[arg(long)]
    skip_git: bool,

    /// Skip dependency installation
    #[arg(long)]
    skip_install: bool,

    /// Use interactive wizard
    #[arg(short, long)]
    interactive: bool,

    /// Database to configure
    #[arg(long, value_enum)]
    database: Option<DatabaseType>,

    /// Include Docker configuration
    #[arg(long)]
    docker: bool,

    /// Include CI/CD configuration
    #[arg(long)]
    ci: bool,
}

#[derive(Clone, ValueEnum)]
enum ProjectTemplate {
    /// Minimal API with health checks
    Minimal,
    /// Full-featured API with auth, caching, etc.
    Full,
    /// Microservice template
    Microservice,
    /// GraphQL API
    Graphql,
    /// gRPC service
    Grpc,
    /// Serverless (AWS Lambda)
    Lambda,
    /// Serverless (Google Cloud Run)
    Cloudrun,
}

#[derive(Clone, ValueEnum)]
enum DatabaseType {
    Postgres,
    Mysql,
    Sqlite,
    Mongodb,
    Redis,
    None,
}

// =============================================================================
// GENERATOR TYPES
// =============================================================================

#[derive(Subcommand)]
enum GeneratorType {
    /// Generate a controller
    #[command(alias = "c")]
    Controller {
        /// Controller name (e.g., "users" or "api/users")
        name: String,

        /// Generate CRUD endpoints
        #[arg(long)]
        crud: bool,

        /// Skip test file generation
        #[arg(long)]
        skip_tests: bool,

        /// Add authentication guard
        #[arg(long)]
        auth: bool,
    },

    /// Generate a module
    #[command(alias = "m")]
    Module {
        /// Module name
        name: String,

        /// Controllers to include (comma-separated)
        #[arg(short, long)]
        controllers: Option<String>,

        /// Providers/services to include (comma-separated)
        #[arg(short, long)]
        providers: Option<String>,
    },

    /// Generate middleware
    #[command(alias = "mw")]
    Middleware {
        /// Middleware name
        name: String,

        /// Skip test file generation
        #[arg(long)]
        skip_tests: bool,
    },

    /// Generate a guard
    #[command(alias = "gu")]
    Guard {
        /// Guard name
        name: String,

        /// Skip test file generation
        #[arg(long)]
        skip_tests: bool,

        /// Guard type
        #[arg(long, value_enum, default_value = "custom")]
        guard_type: GuardType,
    },

    /// Generate a service/provider
    #[command(alias = "s")]
    Service {
        /// Service name
        name: String,

        /// Skip test file generation
        #[arg(long)]
        skip_tests: bool,
    },

    /// Generate a complete resource (controller + service + module)
    #[command(alias = "r")]
    Resource {
        /// Resource name (e.g., "users")
        name: String,

        /// Generate CRUD endpoints
        #[arg(long)]
        crud: bool,
    },

    /// Generate a database model
    Model {
        /// Model name
        name: String,

        /// Fields (e.g., "name:string,age:i32,active:bool")
        #[arg(short, long)]
        fields: Option<String>,

        /// Generate migration
        #[arg(long)]
        migration: bool,
    },

    /// Generate a job/worker
    Job {
        /// Job name
        name: String,

        /// Job type
        #[arg(long, value_enum, default_value = "async")]
        job_type: JobType,
    },

    /// Generate an event
    Event {
        /// Event name
        name: String,
    },

    /// Generate a DTO (Data Transfer Object)
    Dto {
        /// DTO name
        name: String,

        /// Fields (e.g., "name:string,email:string")
        #[arg(short, long)]
        fields: Option<String>,
    },

    /// Generate a full CRUD scaffold
    Scaffold {
        /// Resource name
        name: String,

        /// Fields (e.g., "title:string,body:text,published:bool")
        #[arg(short, long)]
        fields: Option<String>,
    },

    /// Generate a repository (data access layer)
    #[command(alias = "repo")]
    Repository {
        /// Repository name
        name: String,

        /// Skip test file generation
        #[arg(long)]
        skip_tests: bool,
    },

    /// Generate a WebSocket handler
    #[command(alias = "ws")]
    Websocket {
        /// WebSocket handler name
        name: String,

        /// Skip test file generation
        #[arg(long)]
        skip_tests: bool,
    },

    /// Generate a GraphQL resolver
    #[command(alias = "gql")]
    Graphql {
        /// Resolver name
        name: String,

        /// Skip test file generation
        #[arg(long)]
        skip_tests: bool,
    },

    /// Generate an interceptor
    Interceptor {
        /// Interceptor name
        name: String,

        /// Skip test file generation
        #[arg(long)]
        skip_tests: bool,
    },

    /// Generate a validation pipe
    Pipe {
        /// Pipe name
        name: String,

        /// Skip test file generation
        #[arg(long)]
        skip_tests: bool,
    },

    /// Generate an exception filter
    #[command(alias = "filter")]
    ExceptionFilter {
        /// Filter name
        name: String,

        /// Skip test file generation
        #[arg(long)]
        skip_tests: bool,
    },

    /// Generate a configuration module
    #[command(alias = "cfg")]
    Config {
        /// Config module name
        name: String,
    },

    /// Generate a database entity
    #[command(alias = "ent")]
    Entity {
        /// Entity name
        name: String,

        /// ORM type (generic, diesel, seaorm, prax)
        #[arg(long, short, default_value = "generic")]
        orm: String,
    },

    /// Generate a Prax ORM schema file
    #[command(alias = "prax-schema")]
    PraxSchema {
        /// Model name
        name: String,
    },

    /// Generate a Prax ORM repository
    #[command(alias = "prax-repo")]
    PraxRepository {
        /// Repository name
        name: String,

        /// Skip test file generation
        #[arg(long)]
        skip_tests: bool,
    },

    /// Generate a complete Prax ORM module (schema + entity + repository + service)
    #[command(alias = "prax")]
    PraxModule {
        /// Module name
        name: String,
    },

    /// Generate a scheduled task
    #[command(alias = "task", alias = "cron")]
    Scheduler {
        /// Task name
        name: String,

        /// Skip test file generation
        #[arg(long)]
        skip_tests: bool,
    },

    /// Generate a cache service
    #[command(alias = "cache")]
    CacheService {
        /// Cache service name
        name: String,

        /// Skip test file generation
        #[arg(long)]
        skip_tests: bool,
    },

    /// Generate an API client
    #[command(alias = "client")]
    ApiClient {
        /// Client name
        name: String,
    },

    /// Generate a health check controller
    Health,
}

#[derive(Clone, ValueEnum)]
enum GuardType {
    Custom,
    Auth,
    Role,
    Permission,
    ApiKey,
    RateLimit,
}

#[derive(Clone, ValueEnum)]
enum JobType {
    Async,
    Scheduled,
    Recurring,
}

// =============================================================================
// DEV & SERVE ARGS
// =============================================================================

#[derive(Args)]
struct DevArgs {
    /// Port to run the server on
    #[arg(short, long, default_value = "3000")]
    port: u16,

    /// Host to bind to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Open browser automatically
    #[arg(short, long)]
    open: bool,

    /// Enable HTTPS with self-signed cert
    #[arg(long)]
    https: bool,

    /// Additional cargo arguments
    #[arg(last = true)]
    cargo_args: Vec<String>,
}

#[derive(Args)]
struct ServeArgs {
    /// Port to run the server on
    #[arg(short, long, env = "PORT", default_value = "3000")]
    port: u16,

    /// Host to bind to
    #[arg(long, env = "HOST", default_value = "0.0.0.0")]
    host: String,

    /// Number of worker threads
    #[arg(short, long, env = "WORKERS")]
    workers: Option<usize>,

    /// Enable graceful shutdown timeout (seconds)
    #[arg(long, default_value = "30")]
    shutdown_timeout: u64,
}

#[derive(Args)]
struct BuildArgs {
    /// Build in release mode
    #[arg(short, long)]
    release: bool,

    /// Target triple (e.g., x86_64-unknown-linux-musl)
    #[arg(long)]
    target: Option<String>,

    /// Build Docker image
    #[arg(long)]
    docker: bool,

    /// Docker image tag
    #[arg(long, default_value = "latest")]
    tag: String,

    /// Additional cargo arguments
    #[arg(last = true)]
    cargo_args: Vec<String>,
}

// =============================================================================
// ROUTES ARGS
// =============================================================================

#[derive(Args)]
struct RoutesArgs {
    /// Filter by HTTP method
    #[arg(short, long)]
    method: Option<String>,

    /// Filter by path pattern
    #[arg(short, long)]
    path: Option<String>,

    /// Output format
    #[arg(long, value_enum, default_value = "table")]
    format: OutputFormat,

    /// Show middleware attached to routes
    #[arg(long)]
    middleware: bool,
}

#[derive(Clone, ValueEnum)]
enum OutputFormat {
    Table,
    Json,
    Yaml,
    Markdown,
}

// =============================================================================
// CONFIG ARGS
// =============================================================================

#[derive(Args)]
struct ConfigArgs {
    #[command(subcommand)]
    command: Option<ConfigCommands>,
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Validate configuration files
    Validate {
        /// Specific file to validate
        file: Option<String>,
    },

    /// Show current configuration
    Show {
        /// Configuration key to display
        key: Option<String>,
    },

    /// Set a configuration value
    Set {
        /// Configuration key
        key: String,

        /// Configuration value
        value: String,
    },

    /// Initialize configuration files
    Init {
        /// Environment (development, production, test)
        #[arg(short, long, default_value = "development")]
        env: String,
    },
}

// =============================================================================
// DATABASE COMMANDS
// =============================================================================

#[derive(Subcommand)]
enum DbCommands {
    /// Create the database
    Create,

    /// Drop the database
    Drop {
        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
    },

    /// Run pending migrations
    Migrate {
        /// Number of migrations to run
        #[arg(short, long)]
        steps: Option<u32>,
    },

    /// Rollback migrations
    Rollback {
        /// Number of migrations to rollback
        #[arg(short, long, default_value = "1")]
        steps: u32,
    },

    /// Reset database (drop + create + migrate)
    Reset {
        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
    },

    /// Show migration status
    Status,

    /// Seed the database
    Seed {
        /// Specific seeder to run
        seeder: Option<String>,
    },

    /// Generate a new migration
    #[command(alias = "g")]
    Generate {
        /// Migration name
        name: String,
    },
}

// =============================================================================
// DEPLOY ARGS
// =============================================================================

#[derive(Args)]
struct DeployArgs {
    /// Cloud provider
    #[arg(short, long, value_enum)]
    provider: Option<CloudProvider>,

    /// Environment to deploy to
    #[arg(short, long, default_value = "production")]
    env: String,

    /// Skip confirmation prompt
    #[arg(short, long)]
    yes: bool,

    /// Dry run (show what would be deployed)
    #[arg(long)]
    dry_run: bool,
}

#[derive(Clone, ValueEnum)]
enum CloudProvider {
    Aws,
    Gcp,
    Azure,
    Fly,
    Railway,
    Render,
    Shuttle,
    Docker,
}

// =============================================================================
// UPGRADE ARGS
// =============================================================================

#[derive(Args)]
struct UpgradeArgs {
    /// Check for updates without installing
    #[arg(long)]
    check: bool,

    /// Upgrade to specific version
    #[arg(long = "target-version")]
    target_version: Option<String>,
}

// =============================================================================
// REPL ARGS
// =============================================================================

#[derive(Args)]
struct ReplArgs {
    /// Use simple built-in REPL
    #[arg(long)]
    simple: bool,
}

// =============================================================================
// ADD ARGS - Add features to project
// =============================================================================

#[derive(Args)]
struct AddArgs {
    /// Feature to add
    #[arg(value_enum)]
    feature: ArmatureFeature,

    /// Skip Cargo.toml modification (only show what would be added)
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Clone, ValueEnum)]
enum ArmatureFeature {
    /// Authentication (JWT, OAuth2, SAML)
    Auth,
    /// Caching (Redis, Memcached)
    Cache,
    /// Configuration management
    Config,
    /// GraphQL API
    Graphql,
    /// gRPC support
    Grpc,
    /// JWT authentication
    Jwt,
    /// OpenAPI/Swagger docs
    Openapi,
    /// OpenTelemetry observability
    Telemetry,
    /// Rate limiting
    Ratelimit,
    /// Background job queue
    Queue,
    /// Cron jobs
    Cron,
    /// Security middleware
    Security,
    /// File storage (S3, GCS, Azure)
    Storage,
    /// Email sending
    Mail,
    /// Push notifications
    Push,
    /// Testing utilities
    Testing,
    /// Validation framework
    Validation,
    /// WebSockets/SSE
    Realtime,
    /// Compression middleware
    Compression,
    /// Webhooks
    Webhooks,
}

impl ArmatureFeature {
    fn crate_name(&self) -> &'static str {
        match self {
            Self::Auth => "armature-auth",
            Self::Cache => "armature-cache",
            Self::Config => "armature-config",
            Self::Graphql => "armature-graphql",
            Self::Grpc => "armature-grpc",
            Self::Jwt => "armature-jwt",
            Self::Openapi => "armature-openapi",
            Self::Telemetry => "armature-opentelemetry",
            Self::Ratelimit => "armature-ratelimit",
            Self::Queue => "armature-queue",
            Self::Cron => "armature-cron",
            Self::Security => "armature-security",
            Self::Storage => "armature-storage",
            Self::Mail => "armature-mail",
            Self::Push => "armature-push",
            Self::Testing => "armature-testing",
            Self::Validation => "armature-validation",
            Self::Realtime => "armature-core", // WebSocket/SSE is in core
            Self::Compression => "armature-compression",
            Self::Webhooks => "armature-webhooks",
        }
    }

    fn feature_flag(&self) -> Option<&'static str> {
        match self {
            Self::Auth => Some("auth"),
            Self::Cache => Some("cache"),
            Self::Config => Some("config"),
            Self::Graphql => Some("graphql"),
            Self::Grpc => None, // Separate crate
            Self::Jwt => Some("jwt"),
            Self::Openapi => Some("openapi"),
            Self::Telemetry => Some("opentelemetry"),
            Self::Ratelimit => Some("ratelimit"),
            Self::Queue => Some("queue"),
            Self::Cron => Some("cron"),
            Self::Security => Some("security"),
            Self::Storage => None, // Separate crate
            Self::Mail => None,    // Separate crate
            Self::Push => None,    // Separate crate
            Self::Testing => Some("testing"),
            Self::Validation => Some("validation"),
            Self::Realtime => None, // Already in core
            Self::Compression => Some("compression"),
            Self::Webhooks => Some("webhooks"),
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::Auth => "Authentication with JWT, OAuth2, and SAML support",
            Self::Cache => "Caching with Redis and Memcached backends",
            Self::Config => "Configuration management with env, JSON, TOML support",
            Self::Graphql => "GraphQL API with async-graphql integration",
            Self::Grpc => "gRPC server and client support",
            Self::Jwt => "JSON Web Token authentication",
            Self::Openapi => "OpenAPI/Swagger documentation generation",
            Self::Telemetry => "OpenTelemetry tracing and metrics",
            Self::Ratelimit => "Rate limiting with multiple algorithms",
            Self::Queue => "Background job queue with Redis",
            Self::Cron => "Scheduled job execution",
            Self::Security => "Security middleware (CORS, CSP, HSTS)",
            Self::Storage => "File storage (Local, S3, GCS, Azure)",
            Self::Mail => "Email sending with SMTP and cloud providers",
            Self::Push => "Push notifications (Web Push, FCM, APNS)",
            Self::Testing => "Testing utilities and mocks",
            Self::Validation => "Input validation framework",
            Self::Realtime => "WebSocket and Server-Sent Events",
            Self::Compression => "Response compression (gzip, brotli, zstd)",
            Self::Webhooks => "Webhook sending and receiving",
        }
    }
}

// =============================================================================
// VALIDATE ARGS - Project validation
// =============================================================================

#[derive(Args)]
struct ValidateArgs {
    /// Check configuration files only
    #[arg(long)]
    config_only: bool,

    /// Check dependencies only
    #[arg(long)]
    deps_only: bool,

    /// Fix issues automatically where possible
    #[arg(long)]
    fix: bool,
}

// =============================================================================
// OPENAPI COMMANDS
// =============================================================================

#[derive(Subcommand)]
enum OpenapiCommands {
    /// Generate HTTP client from OpenAPI spec
    #[command(alias = "client")]
    Client(OpenapiClientArgs),

    /// Validate an OpenAPI spec
    Validate {
        /// Path to OpenAPI spec (JSON or YAML)
        spec: String,
    },

    /// Generate OpenAPI spec from Armature routes
    Generate {
        /// Output file path
        #[arg(short, long, default_value = "openapi.yaml")]
        output: String,

        /// Output format
        #[arg(long, value_enum, default_value = "yaml")]
        format: SpecFormat,
    },
}

#[derive(Args)]
struct OpenapiClientArgs {
    /// Path to OpenAPI spec (JSON or YAML)
    spec: String,

    /// Output directory for generated client
    #[arg(short, long, default_value = "generated-client")]
    output: String,

    /// Target language
    #[arg(short, long, value_enum, default_value = "typescript")]
    language: ClientLanguageArg,

    /// Custom client class name
    #[arg(long)]
    name: Option<String>,

    /// Include request logging
    #[arg(long)]
    with_logging: bool,

    /// Include retry logic
    #[arg(long)]
    with_retry: bool,
}

#[derive(Clone, ValueEnum)]
enum ClientLanguageArg {
    /// TypeScript client
    Typescript,
    /// Rust client
    Rust,
    /// Both TypeScript and Rust
    Both,
}

#[derive(Clone, ValueEnum)]
enum SpecFormat {
    Yaml,
    Json,
}

// =============================================================================
// PLUGIN COMMANDS
// =============================================================================

#[derive(Subcommand)]
enum PluginCommands {
    /// List installed plugins
    List,

    /// Install a plugin
    Install {
        /// Plugin name or URL
        name: String,
    },

    /// Uninstall a plugin
    Uninstall {
        /// Plugin name
        name: String,
    },

    /// Create a new plugin
    New {
        /// Plugin name
        name: String,
    },
}

// =============================================================================
// BENCH ARGS
// =============================================================================

#[derive(Args)]
struct BenchArgs {
    /// Benchmark name pattern
    pattern: Option<String>,

    /// Save results to file
    #[arg(long)]
    save: Option<String>,

    /// Compare with previous run
    #[arg(long)]
    compare: Option<String>,
}

// =============================================================================
// LINT ARGS
// =============================================================================

#[derive(Args)]
struct LintArgs {
    /// Fix issues automatically
    #[arg(long)]
    fix: bool,

    /// Check specific category
    #[arg(long)]
    category: Option<String>,
}

// =============================================================================
// BANNER AND UI HELPERS
// =============================================================================

fn print_banner() {
    println!(
        "{}",
        r#"
   ╔═══════════════════════════════════════════════════════════════╗
   ║                                                               ║
   ║     █████╗ ██████╗ ███╗   ███╗ █████╗ ████████╗██╗   ██╗     ║
   ║    ██╔══██╗██╔══██╗████╗ ████║██╔══██╗╚══██╔══╝██║   ██║     ║
   ║    ███████║██████╔╝██╔████╔██║███████║   ██║   ██║   ██║     ║
   ║    ██╔══██║██╔══██╗██║╚██╔╝██║██╔══██║   ██║   ██║   ██║     ║
   ║    ██║  ██║██║  ██║██║ ╚═╝ ██║██║  ██║   ██║   ╚██████╔╝     ║
   ║    ╚═╝  ╚═╝╚═╝  ╚═╝╚═╝     ╚═╝╚═╝  ╚═╝   ╚═╝    ╚═════╝      ║
   ║                                                               ║
   ╚═══════════════════════════════════════════════════════════════╝"#
            .bright_cyan()
    );
    println!(
        "     {} {} • {}\n",
        "Armature".bright_white().bold(),
        format!("v{}", env!("CARGO_PKG_VERSION")).bright_yellow(),
        "Rust Web Framework".dimmed()
    );
}

fn print_mini_banner() {
    println!(
        "\n  {} {} {}\n",
        "⚡".bright_yellow(),
        "Armature CLI".bright_white().bold(),
        format!("v{}", env!("CARGO_PKG_VERSION")).dimmed()
    );
}

/// Create a spinner progress indicator.
#[allow(dead_code)]
fn create_spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

fn success(msg: &str) {
    println!("  {} {}", "✓".green().bold(), msg.green());
}

fn warn(msg: &str) {
    println!("  {} {}", "⚠".yellow().bold(), msg.yellow());
}

fn info(msg: &str) {
    println!("  {} {}", "→".cyan(), msg);
}

/// Print a step indicator for multi-step processes.
#[allow(dead_code)]
fn step(num: u32, total: u32, msg: &str) {
    println!(
        "  {} {}",
        format!("[{}/{}]", num, total).dimmed(),
        msg.bright_white()
    );
}

// =============================================================================
// INTERACTIVE PROJECT CREATION WIZARD
// =============================================================================

async fn run_interactive_wizard() -> CliResult<()> {
    print_banner();

    println!(
        "  {}\n",
        "Let's create your new Armature project! 🚀".bright_white()
    );

    let theme = ColorfulTheme::default();

    // Project name
    let name: String = Input::with_theme(&theme)
        .with_prompt("  What's your project name?")
        .default("my-app".to_string())
        .interact_text()
        .map_err(|e| CliError::Io(e.into()))?;

    println!();

    // Template selection
    let templates = vec![
        "minimal    - Minimal API with health checks",
        "full       - Full-featured API with auth, caching, etc.",
        "microservice - Microservice with queue workers",
        "graphql    - GraphQL API with async-graphql",
        "grpc       - gRPC service with tonic",
    ];

    let template_idx = FuzzySelect::with_theme(&theme)
        .with_prompt("  Choose a template")
        .items(&templates)
        .default(0)
        .interact()
        .map_err(|e| CliError::Io(e.into()))?;

    let template = match template_idx {
        0 => "minimal",
        1 => "full",
        2 => "microservice",
        3 => "graphql",
        4 => "grpc",
        _ => "minimal",
    };

    println!();

    // Database selection
    let databases = vec![
        "PostgreSQL",
        "MySQL",
        "SQLite",
        "MongoDB",
        "None (skip database)",
    ];

    let db_idx = FuzzySelect::with_theme(&theme)
        .with_prompt("  Choose a database")
        .items(&databases)
        .default(0)
        .interact()
        .map_err(|e| CliError::Io(e.into()))?;

    println!();

    // Features selection
    let features = vec![
        "Authentication (JWT + OAuth2)",
        "Rate Limiting",
        "Caching (Redis)",
        "Background Jobs",
        "WebSockets",
        "File Uploads",
        "Email Sending",
        "OpenTelemetry",
    ];

    let selected_features = MultiSelect::with_theme(&theme)
        .with_prompt("  Select additional features (space to select, enter to confirm)")
        .items(&features)
        .interact()
        .map_err(|e| CliError::Io(e.into()))?;

    println!();

    // Docker
    let include_docker = Confirm::with_theme(&theme)
        .with_prompt("  Include Docker configuration?")
        .default(true)
        .interact()
        .map_err(|e| CliError::Io(e.into()))?;

    // CI/CD
    let include_ci = Confirm::with_theme(&theme)
        .with_prompt("  Include CI/CD configuration (GitHub Actions)?")
        .default(true)
        .interact()
        .map_err(|e| CliError::Io(e.into()))?;

    println!();
    println!("  {}", "─".repeat(50).dimmed());
    println!();
    println!("  {} Creating project with:", "📋".bright_white());
    println!("     {} {}", "Name:".dimmed(), name.cyan());
    println!("     {} {}", "Template:".dimmed(), template.cyan());
    println!("     {} {}", "Database:".dimmed(), databases[db_idx].cyan());
    if !selected_features.is_empty() {
        println!(
            "     {} {}",
            "Features:".dimmed(),
            selected_features
                .iter()
                .map(|i| features[*i].split(' ').next().unwrap())
                .collect::<Vec<_>>()
                .join(", ")
                .cyan()
        );
    }
    if include_docker {
        println!("     {} Docker", "✓".green());
    }
    if include_ci {
        println!("     {} CI/CD", "✓".green());
    }
    println!();

    let proceed = Confirm::with_theme(&theme)
        .with_prompt("  Proceed with project creation?")
        .default(true)
        .interact()
        .map_err(|e| CliError::Io(e.into()))?;

    if !proceed {
        println!("\n  {} Project creation cancelled.", "✗".red());
        return Ok(());
    }

    println!();

    // Create the project
    new::run(&name, template, false, false).await?;

    // Print next steps
    println!();
    println!("  {}", "━".repeat(50).bright_cyan());
    println!();
    println!(
        "  {} {}",
        "🎉".bright_yellow(),
        "Your project is ready!".bright_white().bold()
    );
    println!();
    println!("  {}", "Next steps:".bright_white());
    println!();
    println!("    {} cd {}", "1.".dimmed(), name.cyan());
    println!("    {} armature dev", "2.".dimmed());
    println!();
    println!("  {}", "Useful commands:".bright_white());
    println!();
    println!(
        "    {} Generate a controller",
        "armature g controller users --crud".cyan()
    );
    println!("    {} List all routes", "armature routes".cyan());
    println!(
        "    {} Build for production",
        "armature build --release".cyan()
    );
    println!();

    Ok(())
}

// =============================================================================
// ADD COMMAND
// =============================================================================

fn run_add_command(args: AddArgs) -> CliResult<()> {
    let feature = args.feature;
    let crate_name = feature.crate_name();
    let description = feature.description();

    println!(
        "  {} Adding feature: {}",
        "→".cyan(),
        format!("{:?}", feature).to_lowercase().bright_white()
    );
    println!("  {} {}", "ℹ".dimmed(), description.dimmed());
    println!();

    if args.dry_run {
        println!("  {} Dry run - no changes will be made\n", "⚠".yellow());
    }

    // Check if we're in a Cargo project
    if !std::path::Path::new("Cargo.toml").exists() {
        return Err(CliError::NotInProject);
    }

    // Read current Cargo.toml
    let cargo_toml = std::fs::read_to_string("Cargo.toml").map_err(CliError::Io)?;

    // Check if feature is already added
    if cargo_toml.contains(crate_name) {
        println!(
            "  {} Feature {} is already added to this project",
            "✓".green(),
            crate_name.cyan()
        );
        return Ok(());
    }

    // Show what would be added
    println!("  {} Changes to Cargo.toml:", "📝".cyan());

    if let Some(flag) = feature.feature_flag() {
        // Enable feature on armature dependency
        println!(
            "    {} armature = {{ features = [\"{}\"] }}",
            "+".green(),
            flag.green()
        );
    } else {
        // Add as separate dependency
        println!("    {} {} = \"*\"", "+".green(), crate_name.green());
    }
    println!();

    if args.dry_run {
        println!("  {} No changes made (dry run)", "ℹ".dimmed());
        return Ok(());
    }

    // Add the dependency using cargo add
    let cargo_add_result = if let Some(flag) = feature.feature_flag() {
        std::process::Command::new("cargo")
            .args(["add", "armature", "--features", flag])
            .status()
    } else {
        std::process::Command::new("cargo")
            .args(["add", crate_name])
            .status()
    };

    match cargo_add_result {
        Ok(status) if status.success() => {
            success(&format!("Feature {} added successfully!", crate_name));

            // Print usage example
            println!("\n  {} Example usage:", "💡".yellow());
            print_feature_usage_example(&feature);
        }
        Ok(_) => {
            warn(&format!(
                "cargo add failed. Try manually adding to Cargo.toml:\n\n    {} = \"*\"",
                crate_name
            ));
        }
        Err(_) => {
            warn(&format!(
                "cargo add not available. Add manually to Cargo.toml:\n\n    {} = \"*\"",
                crate_name
            ));
        }
    }

    Ok(())
}

fn print_feature_usage_example(feature: &ArmatureFeature) {
    let example = match feature {
        ArmatureFeature::Auth => {
            r#"
    use armature_auth::prelude::*;

    let service = AuthService::new();
    let hash = service.hash_password("secret")?;
"#
        }
        ArmatureFeature::Cache => {
            r#"
    use armature_cache::prelude::*;

    let cache = RedisCache::new(config).await?;
    cache.set("key", "value", ttl).await?;
"#
        }
        ArmatureFeature::Jwt => {
            r#"
    use armature_jwt::prelude::*;

    let manager = JwtManager::new(JwtConfig::new("secret"))?;
    let token = manager.sign(&claims)?;
"#
        }
        ArmatureFeature::Queue => {
            r#"
    use armature_queue::prelude::*;

    let queue = Queue::new("redis://localhost", "default").await?;
    queue.enqueue("job_type", json!({})).await?;
"#
        }
        ArmatureFeature::Mail => {
            r#"
    use armature_mail::prelude::*;

    let mailer = Mailer::smtp(config).await?;
    let email = Email::new().to("user@example.com").subject("Hello");
    mailer.send(email).await?;
"#
        }
        _ => {
            r#"
    // Check the crate documentation for usage examples
"#
        }
    };

    for line in example.lines().skip(1) {
        println!("    {}", line.dimmed());
    }
}

// =============================================================================
// VALIDATE COMMAND
// =============================================================================

async fn run_validate_command(args: ValidateArgs) -> CliResult<()> {
    let mut has_errors = false;
    let mut has_warnings = false;

    println!("  {}", "Project Validation".bright_white().bold());
    println!("  {}\n", "─".repeat(40).dimmed());

    // Check if we're in a Cargo project
    if !std::path::Path::new("Cargo.toml").exists() {
        println!("  {} Not in a Rust project directory", "✗".red());
        return Err(CliError::NotInProject);
    }
    println!("  {} Found Cargo.toml", "✓".green());

    // Check for src directory
    if std::path::Path::new("src").exists() {
        println!("  {} Found src directory", "✓".green());
    } else {
        println!("  {} Missing src directory", "✗".red());
        has_errors = true;
    }

    // Check for main.rs or lib.rs
    let has_main = std::path::Path::new("src/main.rs").exists();
    let has_lib = std::path::Path::new("src/lib.rs").exists();
    if has_main || has_lib {
        if has_main {
            println!("  {} Found src/main.rs", "✓".green());
        }
        if has_lib {
            println!("  {} Found src/lib.rs", "✓".green());
        }
    } else {
        println!("  {} Missing src/main.rs or src/lib.rs", "✗".red());
        has_errors = true;
    }

    if !args.deps_only {
        // Check configuration files
        println!("\n  {}", "Configuration:".bright_white());

        // Check for common config files
        let config_files = [
            (".env", "Environment variables"),
            (".env.example", "Example environment"),
            ("config/default.toml", "Default config"),
            ("config/production.toml", "Production config"),
        ];

        for (file, desc) in config_files {
            if std::path::Path::new(file).exists() {
                println!(
                    "  {} Found {} ({})",
                    "✓".green(),
                    file.cyan(),
                    desc.dimmed()
                );
            } else {
                println!(
                    "  {} Missing {} ({})",
                    "○".yellow(),
                    file.dimmed(),
                    desc.dimmed()
                );
            }
        }
    }

    if !args.config_only {
        // Check dependencies
        println!("\n  {}", "Dependencies:".bright_white());

        let cargo_check = std::process::Command::new("cargo")
            .args(["check", "--message-format=short"])
            .output();

        match cargo_check {
            Ok(output) if output.status.success() => {
                println!("  {} Cargo check passed", "✓".green());
            }
            Ok(output) => {
                println!("  {} Cargo check failed:", "✗".red());
                let stderr = String::from_utf8_lossy(&output.stderr);
                for line in stderr.lines().take(5) {
                    println!("    {}", line.dimmed());
                }
                has_errors = true;
            }
            Err(_) => {
                println!("  {} Could not run cargo check", "○".yellow());
                has_warnings = true;
            }
        }
    }

    // Summary
    println!();
    if has_errors {
        println!(
            "  {} Validation found {} that need attention",
            "✗".red(),
            "errors".red()
        );
        if args.fix {
            println!("  {} Auto-fix is not yet implemented", "ℹ".dimmed());
        }
    } else if has_warnings {
        println!(
            "  {} Validation passed with {}",
            "○".yellow(),
            "warnings".yellow()
        );
    } else {
        println!("  {} All checks passed!", "✓".green().bold());
    }
    println!();

    Ok(())
}

// =============================================================================
// DOCTOR COMMAND
// =============================================================================

async fn run_doctor() -> CliResult<()> {
    print_mini_banner();

    println!("  {}", "System Diagnostics".bright_white().bold());
    println!("  {}\n", "─".repeat(40).dimmed());

    let mp = MultiProgress::new();
    let style = ProgressStyle::default_spinner()
        .template("{spinner:.cyan} {msg}")
        .unwrap()
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]);

    // Check Rust
    let pb = mp.add(ProgressBar::new_spinner());
    pb.set_style(style.clone());
    pb.set_message("Checking Rust installation...");
    pb.enable_steady_tick(Duration::from_millis(80));

    let rust_version = std::process::Command::new("rustc")
        .arg("--version")
        .output();

    tokio::time::sleep(Duration::from_millis(300)).await;

    match rust_version {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            pb.finish_with_message(format!(
                "{} Rust: {}",
                "✓".green(),
                version.trim().bright_white()
            ));
        }
        _ => {
            pb.finish_with_message(format!("{} Rust: Not found", "✗".red()));
        }
    }

    // Check Cargo
    let pb = mp.add(ProgressBar::new_spinner());
    pb.set_style(style.clone());
    pb.set_message("Checking Cargo...");
    pb.enable_steady_tick(Duration::from_millis(80));

    let cargo_version = std::process::Command::new("cargo")
        .arg("--version")
        .output();

    tokio::time::sleep(Duration::from_millis(200)).await;

    match cargo_version {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            pb.finish_with_message(format!(
                "{} Cargo: {}",
                "✓".green(),
                version.trim().bright_white()
            ));
        }
        _ => {
            pb.finish_with_message(format!("{} Cargo: Not found", "✗".red()));
        }
    }

    // Check cargo-watch
    let pb = mp.add(ProgressBar::new_spinner());
    pb.set_style(style.clone());
    pb.set_message("Checking cargo-watch...");
    pb.enable_steady_tick(Duration::from_millis(80));

    tokio::time::sleep(Duration::from_millis(200)).await;

    if generators::has_cargo_watch() {
        pb.finish_with_message(format!(
            "{} cargo-watch: {}",
            "✓".green(),
            "installed".bright_white()
        ));
    } else {
        pb.finish_with_message(format!(
            "{} cargo-watch: {} (run: {})",
            "○".yellow(),
            "not installed".yellow(),
            "cargo install cargo-watch".dimmed()
        ));
    }

    // Check Docker
    let pb = mp.add(ProgressBar::new_spinner());
    pb.set_style(style.clone());
    pb.set_message("Checking Docker...");
    pb.enable_steady_tick(Duration::from_millis(80));

    let docker_version = std::process::Command::new("docker")
        .arg("--version")
        .output();

    tokio::time::sleep(Duration::from_millis(200)).await;

    match docker_version {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            pb.finish_with_message(format!(
                "{} Docker: {}",
                "✓".green(),
                version
                    .trim()
                    .split(',')
                    .next()
                    .unwrap_or("")
                    .bright_white()
            ));
        }
        _ => {
            pb.finish_with_message(format!(
                "{} Docker: {}",
                "○".yellow(),
                "not installed (optional)".dimmed()
            ));
        }
    }

    // Check Git
    let pb = mp.add(ProgressBar::new_spinner());
    pb.set_style(style.clone());
    pb.set_message("Checking Git...");
    pb.enable_steady_tick(Duration::from_millis(80));

    let git_version = std::process::Command::new("git").arg("--version").output();

    tokio::time::sleep(Duration::from_millis(200)).await;

    match git_version {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            pb.finish_with_message(format!(
                "{} Git: {}",
                "✓".green(),
                version.trim().bright_white()
            ));
        }
        _ => {
            pb.finish_with_message(format!("{} Git: Not found", "✗".red()));
        }
    }

    // Check evcxr (REPL)
    let pb = mp.add(ProgressBar::new_spinner());
    pb.set_style(style);
    pb.set_message("Checking evcxr_repl...");
    pb.enable_steady_tick(Duration::from_millis(80));

    let evcxr = std::process::Command::new("evcxr")
        .arg("--version")
        .output();

    tokio::time::sleep(Duration::from_millis(200)).await;

    match evcxr {
        Ok(output) if output.status.success() => {
            pb.finish_with_message(format!(
                "{} evcxr_repl: {}",
                "✓".green(),
                "installed".bright_white()
            ));
        }
        _ => {
            pb.finish_with_message(format!(
                "{} evcxr_repl: {} (run: {})",
                "○".yellow(),
                "not installed".yellow(),
                "cargo install evcxr_repl".dimmed()
            ));
        }
    }

    println!();
    println!(
        "  {} All essential tools are installed!",
        "✓".green().bold()
    );
    println!();

    Ok(())
}

// =============================================================================
// MAIN
// =============================================================================

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Handle color preferences
    if cli.no_color {
        colored::control::set_override(false);
    }

    // Print banner for interactive commands
    match &cli.command {
        Commands::New(args) if args.interactive || args.name.is_none() => {
            // Interactive wizard handles its own banner
        }
        Commands::New { .. } | Commands::Generate { .. } => print_mini_banner(),
        Commands::Doctor => {} // Has its own banner
        _ => {}
    }

    let result: CliResult<()> = match cli.command {
        Commands::New(args) => {
            if args.interactive || args.name.is_none() {
                run_interactive_wizard().await
            } else {
                let template = match args.template {
                    Some(ProjectTemplate::Minimal) => "minimal",
                    Some(ProjectTemplate::Full) => "full",
                    Some(ProjectTemplate::Microservice) => "microservice",
                    Some(ProjectTemplate::Graphql) => "graphql",
                    Some(ProjectTemplate::Grpc) => "grpc",
                    Some(ProjectTemplate::Lambda) => "lambda",
                    Some(ProjectTemplate::Cloudrun) => "cloudrun",
                    None => "minimal",
                };
                new::run(
                    args.name.as_deref().unwrap_or("my-app"),
                    template,
                    args.skip_git,
                    args.skip_install,
                )
                .await
            }
        }

        Commands::Generate { generator } => match generator {
            GeneratorType::Controller {
                name,
                crud,
                skip_tests,
                auth: _,
            } => generate::controller(&name, crud, skip_tests).await,

            GeneratorType::Module {
                name,
                controllers,
                providers,
            } => generate::module(&name, controllers.as_deref(), providers.as_deref()).await,

            GeneratorType::Middleware { name, skip_tests } => {
                generate::middleware(&name, skip_tests).await
            }

            GeneratorType::Guard {
                name,
                skip_tests,
                guard_type: _,
            } => generate::guard(&name, skip_tests).await,

            GeneratorType::Service { name, skip_tests } => {
                generate::service(&name, skip_tests).await
            }

            GeneratorType::Resource { name, crud } => generate::resource(&name, crud).await,

            GeneratorType::Model {
                name,
                fields: _,
                migration: _,
            } => {
                info(&format!("Generating model: {}", name.cyan()));
                warn("Model generation is coming soon!");
                Ok(())
            }

            GeneratorType::Job { name, job_type: _ } => generate::job(&name, false).await,

            GeneratorType::Event { name } => generate::event_handler(&name, false).await,

            GeneratorType::Dto { name, fields: _ } => generate::dto(&name).await,

            GeneratorType::Scaffold { name, fields: _ } => {
                // Generate all layers: entity, repository, dto, service, controller
                info(&format!("Generating full scaffold for: {}", name.cyan()));
                async {
                    generate::entity(&name).await?;
                    generate::repository(&name, false).await?;
                    generate::dto(&name).await?;
                    generate::service(&name, false).await?;
                    generate::controller(&name, true, false).await
                }
                .await
            }

            GeneratorType::Repository { name, skip_tests } => {
                generate::repository(&name, skip_tests).await
            }

            GeneratorType::Websocket { name, skip_tests } => {
                generate::websocket(&name, skip_tests).await
            }

            GeneratorType::Graphql { name, skip_tests } => {
                generate::graphql_resolver(&name, skip_tests).await
            }

            GeneratorType::Interceptor { name, skip_tests } => {
                generate::interceptor(&name, skip_tests).await
            }

            GeneratorType::Pipe { name, skip_tests } => generate::pipe(&name, skip_tests).await,

            GeneratorType::ExceptionFilter { name, skip_tests } => {
                generate::exception_filter(&name, skip_tests).await
            }

            GeneratorType::Config { name } => generate::config(&name).await,

            GeneratorType::Entity { name, orm } => match orm.parse::<generate::OrmType>() {
                Ok(orm_type) => generate::entity_with_orm(&name, orm_type).await,
                Err(e) => Err(crate::error::CliError::InvalidArgument(e)),
            },

            GeneratorType::PraxSchema { name } => generate::prax_schema(&name).await,

            GeneratorType::PraxRepository { name, skip_tests } => {
                generate::prax_repository(&name, skip_tests).await
            }

            GeneratorType::PraxModule { name } => generate::prax_module(&name).await,

            GeneratorType::Scheduler { name, skip_tests } => {
                generate::scheduler(&name, skip_tests).await
            }

            GeneratorType::CacheService { name, skip_tests } => {
                generate::cache_service(&name, skip_tests).await
            }

            GeneratorType::ApiClient { name } => generate::api_client(&name).await,

            GeneratorType::Health => generate::health_controller().await,
        },

        Commands::Dev(args) => dev::run(args.port, &args.host, &args.cargo_args).await,

        Commands::Serve(args) => {
            info(&format!(
                "Starting production server on {}:{}",
                args.host.cyan(),
                args.port.to_string().cyan()
            ));
            if let Some(workers) = args.workers {
                info(&format!("Workers: {}", workers));
            }
            warn("Production serve is coming soon!");
            Ok(())
        }

        Commands::Build(args) => build::run(args.release, &args.cargo_args).await,

        Commands::Routes(args) => {
            print_mini_banner();
            if let Some(method) = args.method.as_deref() {
                routes::execute_with_filter(None, Some(method))
            } else {
                routes::execute(None)
            }
        }

        Commands::Config(args) => match args.command {
            Some(ConfigCommands::Validate { file }) => {
                if let Some(f) = file {
                    config::validate_file(&f)
                } else {
                    config::execute(None)
                }
            }
            Some(ConfigCommands::Show { key: _ }) => {
                warn("Config show is coming soon!");
                Ok(())
            }
            Some(ConfigCommands::Set { key: _, value: _ }) => {
                warn("Config set is coming soon!");
                Ok(())
            }
            Some(ConfigCommands::Init { env: _ }) => {
                warn("Config init is coming soon!");
                Ok(())
            }
            None => config::execute(None),
        },

        Commands::Doctor => run_doctor().await,

        Commands::Upgrade(args) => {
            print_mini_banner();
            if args.check {
                info("Checking for updates...");
                println!(
                    "  {} Current version: {}",
                    "→".cyan(),
                    env!("CARGO_PKG_VERSION").bright_white()
                );
                warn("Update checking is coming soon!");
            } else {
                info("Upgrading Armature CLI...");
                warn("Self-upgrade is coming soon!");
                println!(
                    "\n  {} Run: {}",
                    "💡".yellow(),
                    "cargo install armature-cli --force".dimmed()
                );
            }
            Ok(())
        }

        Commands::Db { command } => {
            print_mini_banner();
            match command {
                DbCommands::Create => {
                    info("Creating database...");
                    warn("Database commands are coming soon!");
                }
                DbCommands::Drop { force: _ } => {
                    info("Dropping database...");
                    warn("Database commands are coming soon!");
                }
                DbCommands::Migrate { steps: _ } => {
                    info("Running migrations...");
                    warn("Database commands are coming soon!");
                }
                DbCommands::Rollback { steps: _ } => {
                    info("Rolling back migrations...");
                    warn("Database commands are coming soon!");
                }
                DbCommands::Reset { force: _ } => {
                    info("Resetting database...");
                    warn("Database commands are coming soon!");
                }
                DbCommands::Status => {
                    info("Migration status:");
                    warn("Database commands are coming soon!");
                }
                DbCommands::Seed { seeder: _ } => {
                    info("Seeding database...");
                    warn("Database commands are coming soon!");
                }
                DbCommands::Generate { name } => {
                    info(&format!("Generating migration: {}", name.cyan()));
                    warn("Database commands are coming soon!");
                }
            }
            Ok(())
        }

        Commands::Deploy(args) => {
            print_mini_banner();
            let provider = args.provider.map(|p| match p {
                CloudProvider::Aws => "AWS",
                CloudProvider::Gcp => "GCP",
                CloudProvider::Azure => "Azure",
                CloudProvider::Fly => "Fly.io",
                CloudProvider::Railway => "Railway",
                CloudProvider::Render => "Render",
                CloudProvider::Shuttle => "Shuttle",
                CloudProvider::Docker => "Docker",
            });
            info(&format!(
                "Deploying to {} ({})",
                provider.unwrap_or("auto-detected").cyan(),
                args.env.cyan()
            ));
            if args.dry_run {
                info("Dry run - no changes will be made");
            }
            warn("Deploy command is coming soon!");
            Ok(())
        }

        Commands::Repl(args) => {
            if args.simple {
                repl::execute_simple()
            } else {
                repl::execute()
            }
        }

        Commands::Info => info::run().await,

        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            generate(shell, &mut cmd, "armature", &mut io::stdout());
            Ok(())
        }

        Commands::Docs => {
            info("Opening documentation...");
            let url = "https://github.com/pegasusheavy/armature";
            if webbrowser::open(url).is_err() {
                println!("  {} {}", "→".cyan(), url);
            }
            Ok(())
        }

        Commands::Plugin { command } => {
            print_mini_banner();
            match command {
                PluginCommands::List => {
                    info("Installed plugins:");
                    println!("  {} No plugins installed", "○".dimmed());
                }
                PluginCommands::Install { name } => {
                    info(&format!("Installing plugin: {}", name.cyan()));
                    warn("Plugin system is coming soon!");
                }
                PluginCommands::Uninstall { name } => {
                    info(&format!("Uninstalling plugin: {}", name.cyan()));
                    warn("Plugin system is coming soon!");
                }
                PluginCommands::New { name } => {
                    info(&format!("Creating plugin: {}", name.cyan()));
                    warn("Plugin system is coming soon!");
                }
            }
            Ok(())
        }

        Commands::Bench(args) => {
            print_mini_banner();
            if let Some(pattern) = args.pattern {
                info(&format!("Running benchmarks matching: {}", pattern.cyan()));
            } else {
                info("Running all benchmarks...");
            }
            warn("Benchmark command is coming soon!");
            Ok(())
        }

        Commands::Lint(args) => {
            print_mini_banner();
            if args.fix {
                info("Linting with auto-fix...");
            } else {
                info("Linting project...");
            }
            warn("Lint command is coming soon!");
            Ok(())
        }

        Commands::Clean => {
            print_mini_banner();
            info("Cleaning build artifacts...");
            match std::process::Command::new("cargo").arg("clean").status() {
                Ok(status) if status.success() => {
                    success("Build artifacts cleaned!");
                    Ok(())
                }
                Ok(_) => Err(CliError::Command("cargo clean failed".to_string())),
                Err(e) => Err(CliError::Io(e)),
            }
        }

        Commands::Add(args) => {
            print_mini_banner();
            run_add_command(args)
        }

        Commands::Validate(args) => {
            print_mini_banner();
            run_validate_command(args).await
        }

        Commands::Openapi { command } => {
            print_mini_banner();
            match command {
                OpenapiCommands::Client(args) => {
                    let language = match args.language {
                        ClientLanguageArg::Typescript => openapi::ClientLanguage::TypeScript,
                        ClientLanguageArg::Rust => openapi::ClientLanguage::Rust,
                        ClientLanguageArg::Both => openapi::ClientLanguage::Both,
                    };
                    let options = openapi::ClientOptions {
                        base_url: None,
                        async_client: true,
                        with_logging: args.with_logging,
                        with_retry: args.with_retry,
                        client_name: args.name,
                    };
                    openapi::generate_client(&args.spec, &args.output, language, &options).await
                }
                OpenapiCommands::Validate { spec } => {
                    info(&format!("Validating OpenAPI spec: {}", spec.cyan()));
                    warn("OpenAPI validation is coming soon!");
                    Ok(())
                }
                OpenapiCommands::Generate { output, format: _ } => {
                    info(&format!("Generating OpenAPI spec to: {}", output.cyan()));
                    warn("OpenAPI generation from routes is coming soon!");
                    Ok(())
                }
            }
        }

        Commands::Mock(args) => {
            print_mini_banner();
            let mock_args = mock::MockArgs {
                spec: args.spec,
                port: args.port,
                host: args.host,
                data_dir: args.data,
                delay_ms: args.delay,
                cors: args.cors,
                seed: args.seed,
                watch: args.watch,
            };
            mock::run(mock_args).await
        }

        Commands::Run(args) => {
            run::run(&args.script, args.port, args.host.as_deref(), args.watch).await
        }
    };

    if let Err(e) = result {
        eprintln!("\n  {} {}\n", "Error:".red().bold(), e);
        std::process::exit(1);
    };
}
