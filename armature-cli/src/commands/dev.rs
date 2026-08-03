//! Development server command with file watching.

use colored::Colorize;
use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::error::{CliError, CliResult};
use crate::generators::{find_project_root, has_cargo_watch};

/// Run development server with file watching.
pub async fn run(port: u16, host: &str, cargo_args: &[String]) -> CliResult<()> {
    let project_root = find_project_root()?;

    println!();
    println!(
        "{}",
        "  ╔═══════════════════════════════════════════════════════════╗".bright_cyan()
    );
    println!(
        "{}",
        "  ║           🚀 Armature Development Server                  ║".bright_cyan()
    );
    println!(
        "{}",
        "  ╚═══════════════════════════════════════════════════════════╝".bright_cyan()
    );
    println!();

    // Check if cargo-watch is available, use it if so (more efficient)
    if has_cargo_watch() {
        println!("  {} Using cargo-watch for file watching", "→".green());
        run_with_cargo_watch(port, host, cargo_args, &project_root).await
    } else {
        println!("  {} Using built-in file watcher", "→".yellow());
        println!(
            "  {} Install cargo-watch for better performance: {}",
            "💡".yellow(),
            "cargo install cargo-watch".dimmed()
        );
        run_with_builtin_watcher(port, host, cargo_args, &project_root).await
    }
}

/// Run using cargo-watch (preferred method).
async fn run_with_cargo_watch(
    port: u16,
    host: &str,
    cargo_args: &[String],
    project_root: &Path,
) -> CliResult<()> {
    // cargo-watch takes the command to run as a single `-x` string, so extra
    // cargo arguments have to be folded into it rather than appended to the
    // cargo-watch invocation itself.
    let exec = if cargo_args.is_empty() {
        "run".to_string()
    } else {
        format!("run {}", cargo_args.join(" "))
    };

    let args = vec![
        "watch".to_string(),
        "-x".to_string(),
        exec,
        "-w".to_string(),
        "src".to_string(),
        "-c".to_string(), // Clear screen
        "-q".to_string(), // Quiet cargo-watch output
    ];

    println!(
        "  {} Starting server on http://{}:{}",
        "→".green(),
        host,
        port
    );
    println!("  {} Watching for changes in src/", "→".green());
    println!("  {} Press {} to stop", "→".dimmed(), "Ctrl+C".yellow());
    println!();

    // Environment variables are passed directly to the child process via Command::env()
    // No need to modify the current process environment
    let status = Command::new("cargo")
        .args(&args)
        .current_dir(project_root)
        .env("PORT", port.to_string())
        .env("HOST", host)
        .status()?;

    if !status.success() {
        return Err(CliError::Command(
            "cargo-watch exited with error".to_string(),
        ));
    }

    Ok(())
}

/// Run using built-in file watcher.
async fn run_with_builtin_watcher(
    port: u16,
    host: &str,
    cargo_args: &[String],
    project_root: &Path,
) -> CliResult<()> {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    // Handle Ctrl+C
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .map_err(|e| CliError::Command(format!("Failed to set Ctrl+C handler: {}", e)))?;

    println!(
        "  {} Starting server on http://{}:{}",
        "→".green(),
        host,
        port
    );
    println!("  {} Watching for changes in src/", "→".green());
    println!("  {} Press {} to stop", "→".dimmed(), "Ctrl+C".yellow());
    println!();

    // Initial build and run
    let mut child = start_server(project_root, port, host, cargo_args)?;

    // Set up file watcher
    let (tx, rx) = std::sync::mpsc::channel();

    let mut debouncer = new_debouncer(Duration::from_millis(500), tx)
        .map_err(|e| CliError::Watch(e.to_string()))?;

    debouncer
        .watcher()
        .watch(&project_root.join("src"), RecursiveMode::Recursive)?;

    println!("  {} Waiting for changes...", "→".dimmed());
    println!();

    // Watch loop
    while running.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(events)) => {
                // Filter for Rust file changes
                let rust_changes: Vec<_> = events
                    .iter()
                    .filter(|e| e.path.extension().map(|ext| ext == "rs").unwrap_or(false))
                    .collect();

                if !rust_changes.is_empty() {
                    println!();
                    println!("  {} File changed, rebuilding...", "↻".yellow().bold());

                    // Kill the current server
                    let _ = child.kill();
                    let _ = child.wait();

                    // Clear screen (optional)
                    print!("\x1B[2J\x1B[1;1H");

                    // Restart server
                    child = start_server(project_root, port, host, cargo_args)?;

                    println!("  {} Waiting for changes...", "→".dimmed());
                }
            }
            Ok(Err(e)) => {
                eprintln!("  {} Watch error: {}", "⚠".yellow(), e);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Check if child process is still running
                match child.try_wait() {
                    Ok(Some(status)) => {
                        if !status.success() {
                            println!(
                                "  {} Server crashed, waiting for changes to restart...",
                                "⚠".red()
                            );
                        }
                    }
                    Ok(None) => {
                        // Still running
                    }
                    Err(e) => {
                        eprintln!("  {} Error checking server status: {}", "⚠".yellow(), e);
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                break;
            }
        }
    }

    // Cleanup
    println!();
    println!("  {} Shutting down...", "→".yellow());
    let _ = child.kill();
    let _ = child.wait();

    Ok(())
}

/// Start the server process.
fn start_server(
    project_root: &Path,
    port: u16,
    host: &str,
    cargo_args: &[String],
) -> CliResult<Child> {
    let mut args = vec!["run".to_string()];
    args.extend(cargo_args.iter().cloned());

    let child = Command::new("cargo")
        .args(&args)
        .current_dir(project_root)
        .env("PORT", port.to_string())
        .env("HOST", host)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;

    Ok(child)
}
