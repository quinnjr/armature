//! Utility functions for code generation.

use heck::{ToKebabCase, ToPascalCase, ToSnakeCase};
use std::path::{Path, PathBuf};

use crate::error::{CliError, CliResult};

/// Convert a name to various case formats.
pub struct NameCases {
    pub pascal: String,
    pub snake: String,
    pub kebab: String,
    #[allow(dead_code)]
    pub original: String,
}

impl NameCases {
    /// Create name cases from the original name.
    pub fn from(name: &str) -> Self {
        // Handle path-like names (e.g., "api/users")
        let base_name = name.split('/').next_back().unwrap_or(name);

        Self {
            pascal: base_name.to_pascal_case(),
            snake: base_name.to_snake_case(),
            kebab: base_name.to_kebab_case(),
            original: name.to_string(),
        }
    }
}

/// Find the project root by looking for Cargo.toml.
pub fn find_project_root() -> CliResult<PathBuf> {
    let mut current = std::env::current_dir()?;

    loop {
        if current.join("Cargo.toml").exists() {
            // Check if it's an Armature project by looking for armature in dependencies
            let cargo_toml = std::fs::read_to_string(current.join("Cargo.toml"))?;
            if cargo_toml.contains("armature") {
                return Ok(current);
            }
        }

        if !current.pop() {
            return Err(CliError::NotInProject);
        }
    }
}

/// Get the source directory for the project.
pub fn get_src_dir() -> CliResult<PathBuf> {
    let root = find_project_root()?;
    Ok(root.join("src"))
}

/// Ensure a directory exists, creating it if necessary.
pub fn ensure_dir(path: &Path) -> CliResult<()> {
    if !path.exists() {
        std::fs::create_dir_all(path).map_err(|e| {
            CliError::Command(format!(
                "failed to create directory {}: {}",
                path.display(),
                e
            ))
        })?;
    }
    Ok(())
}

/// Write a file, checking if it already exists.
pub fn write_file(path: &Path, content: &str, overwrite: bool) -> CliResult<()> {
    if path.exists() && !overwrite {
        return Err(CliError::FileExists(path.display().to_string()));
    }

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }

    std::fs::write(path, content)
        .map_err(|e| CliError::Command(format!("failed to write {}: {}", path.display(), e)))?;
    Ok(())
}

/// Update the mod.rs file to include a new module.
pub fn update_mod_file(dir: &Path, module_name: &str) -> CliResult<()> {
    let mod_file = dir.join("mod.rs");
    let mod_line = format!("pub mod {};\n", module_name);

    if mod_file.exists() {
        let content = std::fs::read_to_string(&mod_file).map_err(|e| {
            CliError::Command(format!("failed to read {}: {}", mod_file.display(), e))
        })?;
        if !content.contains(&format!("mod {};", module_name)) {
            let new_content = format!("{}{}", content, mod_line);
            std::fs::write(&mod_file, new_content).map_err(|e| {
                CliError::Command(format!("failed to write {}: {}", mod_file.display(), e))
            })?;
        }
    } else {
        std::fs::write(&mod_file, mod_line).map_err(|e| {
            CliError::Command(format!("failed to write {}: {}", mod_file.display(), e))
        })?;
    }

    Ok(())
}

/// Check if cargo-watch is installed.
pub fn has_cargo_watch() -> bool {
    which::which("cargo-watch").is_ok()
}
