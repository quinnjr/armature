//! Integration tests for the `armature` CLI binary.
//!
//! These exercise project scaffolding, code generation, route listing, OpenAPI
//! client generation, and exit-code behavior end-to-end in throwaway tempdirs.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;

/// Build a `Command` for the `armature` binary rooted in `dir`, with color off.
fn armature_in(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("armature").unwrap();
    cmd.current_dir(dir);
    cmd.arg("--no-color");
    cmd
}

/// Create a minimal Armature-looking project (Cargo.toml + src) inside `dir`.
fn scaffold_project(dir: &Path) {
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\narmature = \"0.1\"\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("src/controllers")).unwrap();
    fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(dir.join("src/controllers/mod.rs"), "pub mod health;\n").unwrap();
}

fn assert_exists(base: &Path, rel: &str) {
    assert!(
        base.join(rel).exists(),
        "expected generated file/dir to exist: {}",
        rel
    );
}

// =============================================================================
// Command tree validity (guards against duplicate clap aliases, which make
// clap's debug asserts panic on EVERY invocation)
// =============================================================================

#[test]
fn command_tree_is_valid() {
    let tmp = tempfile::tempdir().unwrap();
    armature_in(tmp.path()).arg("--version").assert().success();
    // Exercise nested subcommand help to force the whole tree to build.
    armature_in(tmp.path())
        .args(["openapi", "--help"])
        .assert()
        .success();
    armature_in(tmp.path())
        .args(["generate", "--help"])
        .assert()
        .success();
}

// =============================================================================
// Project templates
// =============================================================================

#[test]
fn new_minimal_produces_tree() {
    let tmp = tempfile::tempdir().unwrap();
    armature_in(tmp.path())
        .args(["new", "acme", "--template", "minimal", "--skip-git"])
        .assert()
        .success();

    let root = tmp.path().join("acme");
    for f in [
        "Cargo.toml",
        "src/main.rs",
        "src/controllers/mod.rs",
        "src/controllers/health.rs",
        ".env.example",
        "README.md",
        ".gitignore",
    ] {
        assert_exists(&root, f);
    }
}

#[test]
fn new_full_produces_tree() {
    let tmp = tempfile::tempdir().unwrap();
    armature_in(tmp.path())
        .args(["new", "acme", "--template", "full", "--skip-git"])
        .assert()
        .success();

    let root = tmp.path().join("acme");
    for f in [
        "src/services/mod.rs",
        "src/middleware/mod.rs",
        "src/guards/mod.rs",
        "src/models/mod.rs",
        "Dockerfile",
        "docker-compose.yml",
    ] {
        assert_exists(&root, f);
    }
}

#[test]
fn new_microservice_produces_tree() {
    let tmp = tempfile::tempdir().unwrap();
    armature_in(tmp.path())
        .args(["new", "acme", "--template", "microservice", "--skip-git"])
        .assert()
        .success();

    let root = tmp.path().join("acme");
    assert_exists(&root, "src/handlers/mod.rs");
    assert_exists(&root, "src/jobs/mod.rs");
    assert_exists(&root, "Dockerfile");
}

#[test]
fn new_graphql_produces_tree() {
    let tmp = tempfile::tempdir().unwrap();
    armature_in(tmp.path())
        .args(["new", "acme", "--template", "graphql", "--skip-git"])
        .assert()
        .success();

    let root = tmp.path().join("acme");
    assert_exists(&root, "src/graphql/mod.rs");
    let cargo = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(
        cargo.contains("async-graphql"),
        "graphql template Cargo.toml must depend on async-graphql, got:\n{cargo}"
    );
}

#[test]
fn new_grpc_produces_tree() {
    let tmp = tempfile::tempdir().unwrap();
    armature_in(tmp.path())
        .args(["new", "acme", "--template", "grpc", "--skip-git"])
        .assert()
        .success();

    let root = tmp.path().join("acme");
    assert_exists(&root, "src/grpc/mod.rs");
    assert_exists(&root, "proto/service.proto");
    let cargo = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(
        cargo.contains("tonic"),
        "grpc template Cargo.toml must depend on tonic, got:\n{cargo}"
    );
}

#[test]
fn new_lambda_produces_tree() {
    let tmp = tempfile::tempdir().unwrap();
    armature_in(tmp.path())
        .args(["new", "acme", "--template", "lambda", "--skip-git"])
        .assert()
        .success();

    let root = tmp.path().join("acme");
    let cargo = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(
        cargo.contains("lambda_http") || cargo.contains("lambda_runtime"),
        "lambda template Cargo.toml must depend on a lambda crate, got:\n{cargo}"
    );
    let main = fs::read_to_string(root.join("src/main.rs")).unwrap();
    assert!(
        main.contains("lambda"),
        "lambda main.rs should reference lambda runtime"
    );
}

#[test]
fn new_cloudrun_produces_tree() {
    let tmp = tempfile::tempdir().unwrap();
    armature_in(tmp.path())
        .args(["new", "acme", "--template", "cloudrun", "--skip-git"])
        .assert()
        .success();

    let root = tmp.path().join("acme");
    assert_exists(&root, "Dockerfile");
    // Cloud Run deployment descriptor.
    assert!(
        root.join("service.yaml").exists() || root.join("cloudbuild.yaml").exists(),
        "cloudrun template must emit a Cloud Run deploy descriptor"
    );
}

#[test]
fn invalid_template_creates_nothing() {
    // clap rejects an unknown --template value before any work happens.
    let tmp = tempfile::tempdir().unwrap();
    armature_in(tmp.path())
        .args(["new", "acme", "--template", "bogus", "--skip-git"])
        .assert()
        .failure();

    assert!(
        !tmp.path().join("acme").exists(),
        "no project directory must be created for an invalid template"
    );
}

#[test]
fn new_database_docker_ci_emits_config() {
    let tmp = tempfile::tempdir().unwrap();
    armature_in(tmp.path())
        .args([
            "new",
            "acme",
            "--template",
            "minimal",
            "--database",
            "postgres",
            "--docker",
            "--ci",
            "--skip-git",
        ])
        .assert()
        .success();

    let root = tmp.path().join("acme");
    let cargo = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(
        cargo.contains("sqlx") || cargo.contains("postgres"),
        "postgres database should add a db dependency, got:\n{cargo}"
    );
    assert_exists(&root, "Dockerfile");
    assert_exists(&root, ".github/workflows/ci.yml");
}

// =============================================================================
// generate --fields
// =============================================================================

#[test]
fn generate_dto_injects_fields() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());

    armature_in(tmp.path())
        .args(["g", "dto", "user", "--fields", "name:string,email:string"])
        .assert()
        .success();

    let dto = fs::read_to_string(tmp.path().join("src/dto/user.rs")).unwrap();
    assert!(
        dto.contains("pub name: String"),
        "DTO must contain the requested name field, got:\n{dto}"
    );
    assert!(
        dto.contains("pub email: String"),
        "DTO must contain the requested email field, got:\n{dto}"
    );
}

// =============================================================================
// routes
// =============================================================================

fn scaffold_routes_project(dir: &Path) {
    scaffold_project(dir);
    fs::write(
        dir.join("src/controllers/users.rs"),
        r#"
#[middleware(LoggerMiddleware)]
#[guard(AuthGuard)]
#[get("/api/users")]
pub async fn list() {}

#[post("/api/users")]
pub async fn create() {}
"#,
    )
    .unwrap();
}

#[test]
fn routes_formats_differ() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_routes_project(tmp.path());

    let json = armature_in(tmp.path())
        .args(["routes", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json = String::from_utf8(json).unwrap();
    assert!(json.contains('['), "json format should emit a JSON array");

    let markdown = armature_in(tmp.path())
        .args(["routes", "--format", "markdown"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let markdown = String::from_utf8(markdown).unwrap();
    assert!(
        markdown.contains('|'),
        "markdown format should emit a table with pipes"
    );

    let yaml = armature_in(tmp.path())
        .args(["routes", "--format", "yaml"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let yaml = String::from_utf8(yaml).unwrap();
    assert!(
        yaml.contains("method:") || yaml.contains("- method"),
        "yaml format should emit yaml keys"
    );

    assert_ne!(json, markdown, "json and markdown output must differ");
    assert_ne!(json, yaml, "json and yaml output must differ");
}

#[test]
fn routes_reports_middleware_stats() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_routes_project(tmp.path());

    armature_in(tmp.path())
        .args(["routes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Routes with middleware: 1"));
}

#[test]
fn routes_path_filter() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());
    fs::write(
        tmp.path().join("src/controllers/mixed.rs"),
        "#[get(\"/api/users\")]\npub async fn a() {}\n#[get(\"/health\")]\npub async fn b() {}\n",
    )
    .unwrap();

    armature_in(tmp.path())
        .args(["routes", "--path", "users"])
        .assert()
        .success()
        .stdout(predicate::str::contains("/api/users"))
        .stdout(predicate::str::contains("/health").not());
}

// =============================================================================
// validate exit code
// =============================================================================

#[test]
fn validate_fails_on_missing_src() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();

    // --config-only avoids invoking cargo; the missing src still fails validation.
    armature_in(tmp.path())
        .args(["validate", "--config-only"])
        .assert()
        .failure();
}

// =============================================================================
// openapi client generation
// =============================================================================

fn write_petstore_spec(dir: &Path) -> std::path::PathBuf {
    let spec = r#"openapi: 3.0.0
info:
  title: Pet Store
  version: 1.0.0
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: array
                items:
                  $ref: '#/components/schemas/Pet'
components:
  schemas:
    Pet:
      type: object
      properties:
        id:
          type: integer
        name:
          type: string
"#;
    let path = dir.join("openapi.yaml");
    fs::write(&path, spec).unwrap();
    path
}

#[test]
fn openapi_rust_client_honors_logging_and_retry() {
    let tmp = tempfile::tempdir().unwrap();
    write_petstore_spec(tmp.path());

    armature_in(tmp.path())
        .args([
            "openapi",
            "client",
            "openapi.yaml",
            "--language",
            "rust",
            "--output",
            "out",
            "--with-logging",
            "--with-retry",
        ])
        .assert()
        .success();

    let client = fs::read_to_string(tmp.path().join("out/client.rs")).unwrap();
    assert!(
        client.contains("retry") || client.contains("Retry"),
        "with-retry must emit retry code, got:\n{client}"
    );
    assert!(
        client.to_lowercase().contains("log")
            || client.contains("eprintln")
            || client.contains("tracing"),
        "with-logging must emit logging code"
    );
}

#[test]
fn openapi_ts_client_honors_logging_and_retry_and_base_url() {
    let tmp = tempfile::tempdir().unwrap();
    write_petstore_spec(tmp.path());

    armature_in(tmp.path())
        .args([
            "openapi",
            "client",
            "openapi.yaml",
            "--language",
            "typescript",
            "--output",
            "out",
            "--with-logging",
            "--with-retry",
            "--base-url",
            "https://api.example.com",
        ])
        .assert()
        .success();

    let client = fs::read_to_string(tmp.path().join("out/client.ts")).unwrap();
    assert!(
        client.contains("retry") || client.contains("Retry"),
        "with-retry must emit retry code in TS"
    );
    assert!(
        client.contains("console."),
        "with-logging must emit console logging in TS"
    );
    assert!(
        client.contains("https://api.example.com"),
        "base-url must be baked into the generated TS client"
    );
}

// =============================================================================
// Removed `db` subcommand
// =============================================================================

#[test]
fn db_subcommand_removed() {
    let tmp = tempfile::tempdir().unwrap();
    armature_in(tmp.path())
        .args(["db", "migrate"])
        .assert()
        .failure();
}

// =============================================================================
// Ignored smoke test: cargo-check a generated project
// =============================================================================

#[test]
#[ignore = "compiles a generated project; slow and needs network for crates.io"]
fn generated_minimal_project_cargo_checks() {
    let tmp = tempfile::tempdir().unwrap();
    armature_in(tmp.path())
        .args(["new", "acme", "--template", "minimal", "--skip-git"])
        .assert()
        .success();

    let root = tmp.path().join("acme");
    let status = std::process::Command::new("cargo")
        .arg("check")
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "generated minimal project should cargo-check"
    );
}
