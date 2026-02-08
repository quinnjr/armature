//! Run a Rhai application script.

use colored::Colorize;
use std::path::Path;

use crate::error::{CliError, CliResult};

/// Run a Rhai application script.
pub async fn run(
    script: &str,
    port: Option<u16>,
    host: Option<&str>,
    watch: bool,
) -> CliResult<()> {
    let script_path = Path::new(script);

    if !script_path.exists() {
        return Err(CliError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Script not found: {}", script),
        )));
    }

    println!();
    println!(
        "{}",
        "  ╔═══════════════════════════════════════════════════════════╗".bright_cyan()
    );
    println!(
        "{}",
        "  ║            🦾 Armature Rhai Application                   ║".bright_cyan()
    );
    println!(
        "{}",
        "  ╚═══════════════════════════════════════════════════════════╝".bright_cyan()
    );
    println!();

    println!(
        "  {} Loading script: {}",
        "→".green(),
        script_path.display()
    );

    if watch {
        println!(
            "  {} Watch mode: {}",
            "→".yellow(),
            "not yet implemented".dimmed()
        );
    }

    let config = armature_app::RunConfig {
        port,
        host: host.map(String::from),
    };

    let display_port = port.unwrap_or(3000);
    let display_host = host.unwrap_or("0.0.0.0");

    println!(
        "  {} Starting server on http://{}:{}",
        "→".green(),
        display_host,
        display_port
    );
    println!("  {} Press {} to stop", "→".dimmed(), "Ctrl+C".yellow());
    println!();

    armature_app::run(script_path, config)
        .await
        .map_err(|e| CliError::Command(format!("{}", e)))
}
