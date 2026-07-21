//! Interactive Rust REPL command
//!
//! Provides an interactive REPL for testing Armature code snippets.

use crate::error::CliError;
use std::io::{self, Write};
use std::process::{Command, Stdio};

/// REPL command implementation
pub fn execute() -> Result<(), CliError> {
    println!("🦀 Armature Interactive REPL");
    println!("===============================");
    println!("Type Rust code and press Enter to evaluate.");
    println!("Type 'exit' or 'quit' to quit.");
    println!("Type 'help' for more commands.");
    println!();

    // Check if evcxr is installed
    if !is_evcxr_installed() {
        println!("⚠️  REPL requires evcxr_repl to be installed.");
        println!();
        println!("Install with:");
        println!("  cargo install evcxr_repl");
        println!();
        println!("Or use the built-in simple REPL (limited functionality):");
        println!("  armature repl --simple");
        println!();
        return Err(CliError::Tool("evcxr_repl not found".to_string()));
    }

    // Launch evcxr_repl with Armature prelude
    launch_evcxr_repl()
}

/// Execute simple built-in REPL (limited functionality)
pub fn execute_simple() -> Result<(), CliError> {
    println!("🦀 Armature Simple REPL");
    println!("=======================");
    println!("Type Rust expressions and press Enter.");
    println!("Type 'exit' or 'quit' to quit.");
    println!();

    let mut history = Vec::new();

    loop {
        print!(">> ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        if input.is_empty() {
            continue;
        }

        match input {
            "exit" | "quit" => {
                println!("Goodbye!");
                break;
            }
            "help" => {
                print_help();
                continue;
            }
            "history" => {
                print_history(&history);
                continue;
            }
            "clear" => {
                print!("\x1B[2J\x1B[1;1H");
                continue;
            }
            _ => {
                history.push(input.to_string());

                // Simple evaluation (limited - just echoes the input)
                println!("📝 Input: {}", input);
                println!("ℹ️  Note: Simple REPL has limited evaluation.");
                println!("   Install evcxr_repl for full Rust REPL: cargo install evcxr_repl");
                println!();
            }
        }
    }

    Ok(())
}

fn is_evcxr_installed() -> bool {
    Command::new("evcxr")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Build the init script fed to evcxr at launch so the Armature prelude is
/// available in the REPL (adds the dependency and imports the prelude).
fn repl_init_script() -> String {
    let mut script = String::new();
    // Add the armature dependency and pull in the prelude. Kept in sync with
    // the `armature = "0.2"` pin used for generated projects in
    // `commands/new.rs::build_dependencies` / `templates.rs::ProjectData::new`.
    script.push_str(":dep armature = \"0.2\"\n");
    script.push_str("use armature::prelude::*;\n");
    script
}

fn launch_evcxr_repl() -> Result<(), CliError> {
    println!("Starting evcxr REPL with the Armature prelude...");
    println!();

    let mut child = Command::new("evcxr")
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(CliError::Io)?;

    // Feed the init script (":dep" + "use armature::prelude::*;") so the prelude
    // is loaded before handing the session to the user.
    let mut child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| CliError::Tool("failed to open evcxr stdin".to_string()))?;
    child_stdin
        .write_all(repl_init_script().as_bytes())
        .map_err(CliError::Io)?;
    child_stdin.flush().map_err(CliError::Io)?;

    // Forward the user's stdin to the REPL for the interactive session.
    std::thread::spawn(move || {
        let mut user_input = io::stdin();
        let _ = io::copy(&mut user_input, &mut child_stdin);
    });

    let status = child.wait().map_err(CliError::Io)?;

    if !status.success() {
        return Err(CliError::Tool("evcxr_repl exited with error".to_string()));
    }

    Ok(())
}

fn print_help() {
    println!("Available commands:");
    println!("  help     - Show this help message");
    println!("  history  - Show command history");
    println!("  clear    - Clear the screen");
    println!("  exit     - Exit the REPL");
    println!("  quit     - Exit the REPL");
    println!();
}

fn print_history(history: &[String]) {
    if history.is_empty() {
        println!("No command history yet.");
        return;
    }

    println!("Command history:");
    for (i, cmd) in history.iter().enumerate() {
        println!("  {}: {}", i + 1, cmd);
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_evcxr_installed() {
        // This will vary by system
        let _ = is_evcxr_installed();
    }

    #[test]
    fn repl_init_script_injects_prelude() {
        let script = repl_init_script();
        assert!(script.contains(":dep"), "init script must add a dependency");
        assert!(
            script.contains("use armature::prelude::*;"),
            "init script must import the Armature prelude"
        );
    }
}
