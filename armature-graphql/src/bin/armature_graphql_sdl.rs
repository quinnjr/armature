//! `armature-graphql-sdl` — statically analyze async-graphql resolver
//! source and render its SDL, without running the server.
//!
//! Discoverable as `armature graphql-sdl` via armature-cli's
//! external-subcommand mechanism once this binary is on `$PATH`.
//!
//! Recursively scans a directory (or reads a single file) for the
//! standard async-graphql macros — `#[derive(SimpleObject)]`, `#[Object]`,
//! `#[derive(Enum)]`, `#[derive(InputObject)]`, `#[derive(Union)]`,
//! `#[derive(Interface)]`, `#[Scalar]` — and renders SDL text directly
//! from the parsed AST. See `armature_graphql::sdl_static` for the
//! analysis itself.

use armature_graphql::sdl_static::SchemaBuilder;
use clap::Parser;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "armature-graphql-sdl",
    about = "Statically render SDL from async-graphql resolver source (no running server required)"
)]
struct Args {
    /// Directory to scan (recursively) or a single .rs file to analyze
    #[arg(value_name = "PATH", default_value = "src")]
    path: PathBuf,

    /// File to write the rendered SDL to
    #[arg(short, long, value_name = "PATH", default_value = "schema.graphql")]
    output: PathBuf,

    /// Name of the root Query type
    #[arg(long, default_value = "Query")]
    query: String,

    /// Name of the root Mutation type
    #[arg(long, default_value = "Mutation")]
    mutation: String,

    /// Name of the root Subscription type
    #[arg(long, default_value = "Subscription")]
    subscription: String,
}

fn main() -> ExitCode {
    let args = Args::parse();

    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<(), String> {
    let files = collect_rust_files(&args.path)?;
    if files.is_empty() {
        return Err(format!("no .rs files found under {}", args.path.display()));
    }

    let mut builder = SchemaBuilder::new();
    for file in &files {
        let source = std::fs::read_to_string(file)
            .map_err(|e| format!("failed to read {}: {e}", file.display()))?;
        builder
            .add_source(&source)
            .map_err(|e| format!("{}: {e}", file.display()))?;
    }

    if builder.is_empty() {
        return Err(format!(
            "no GraphQL types (SimpleObject/Object/Enum/InputObject/Union/Interface/Scalar) \
             found under {} ({} file(s) scanned)",
            args.path.display(),
            files.len()
        ));
    }

    let sdl = builder.render(&args.query, &args.mutation, &args.subscription);

    std::fs::write(&args.output, &sdl)
        .map_err(|e| format!("failed to write {}: {e}", args.output.display()))?;

    println!(
        "analyzed {} file(s), wrote {} bytes of SDL to {}",
        files.len(),
        sdl.len(),
        args.output.display()
    );
    Ok(())
}

/// Recursively collects `.rs` files under `path` (or returns `[path]` if
/// it's already a file), skipping `target/`, hidden directories, and
/// `node_modules/`. Results are sorted for deterministic output.
fn collect_rust_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if !path.is_dir() {
        return Err(format!("{} does not exist", path.display()));
    }

    let mut files = Vec::new();
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| format!("failed to read directory {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("failed to read directory entry: {e}"))?;
            let entry_path = entry.path();
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();

            if entry_path.is_dir() {
                if file_name.starts_with('.')
                    || file_name == "target"
                    || file_name == "node_modules"
                {
                    continue;
                }
                stack.push(entry_path);
            } else if entry_path.extension().is_some_and(|ext| ext == "rs") {
                files.push(entry_path);
            }
        }
    }

    files.sort();
    Ok(files)
}
