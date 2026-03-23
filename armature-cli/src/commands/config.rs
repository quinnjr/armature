//! Config validation command
//!
//! Validates application configuration files.

use crate::error::CliError;
use std::fs;
use std::path::Path;

/// Config validation result
#[derive(Debug)]
struct ValidationResult {
    file: String,
    valid: bool,
    errors: Vec<String>,
    warnings: Vec<String>,
}

/// Config check command
pub fn execute(project_dir: Option<&str>) -> Result<(), CliError> {
    let dir = project_dir.unwrap_or(".");

    println!("⚙️  Armature Config Validation");
    println!("==============================");
    println!();

    // Find config files
    let config_files = find_config_files(dir)?;

    if config_files.is_empty() {
        println!("❌ No configuration files found.");
        println!();
        println!("Expected files:");
        println!("  - config/default.toml");
        println!("  - config/development.toml");
        println!("  - config/production.toml");
        println!("  - .env");
        println!("  - Cargo.toml");
        return Ok(());
    }

    println!("Found {} configuration file(s)", config_files.len());
    println!();

    // Validate each config file
    let mut results = Vec::new();
    for file in &config_files {
        let result = validate_config_file(file)?;
        results.push(result);
    }

    // Print results
    print_validation_results(&results);

    // Summary
    println!();
    let valid_count = results.iter().filter(|r| r.valid).count();
    let error_count: usize = results.iter().map(|r| r.errors.len()).sum();
    let warning_count: usize = results.iter().map(|r| r.warnings.len()).sum();

    println!("Summary:");
    println!("  ✅ Valid files: {}/{}", valid_count, results.len());
    if error_count > 0 {
        println!("  ❌ Errors: {}", error_count);
    }
    if warning_count > 0 {
        println!("  ⚠️  Warnings: {}", warning_count);
    }

    if valid_count == results.len() && error_count == 0 {
        println!();
        println!("✅ All configuration files are valid!");
        Ok(())
    } else {
        Err(CliError::Validation(
            "Configuration validation failed".to_string(),
        ))
    }
}

fn find_config_files(dir: &str) -> Result<Vec<String>, CliError> {
    let mut files = Vec::new();

    // Standard config locations
    let locations = vec![
        format!("{}/config/default.toml", dir),
        format!("{}/config/development.toml", dir),
        format!("{}/config/production.toml", dir),
        format!("{}/.env", dir),
        format!("{}/.env.example", dir),
        format!("{}/Cargo.toml", dir),
    ];

    for location in locations {
        if Path::new(&location).exists() {
            files.push(location);
        }
    }

    Ok(files)
}

fn validate_config_file(file: &str) -> Result<ValidationResult, CliError> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    // Read file
    let content = fs::read_to_string(file).map_err(CliError::Io)?;

    // Determine file type and validate
    if file.ends_with(".toml") {
        validate_toml(&content, &mut errors, &mut warnings);
    } else if file.ends_with(".env") || file.contains(".env") {
        validate_env(&content, &mut errors, &mut warnings);
    }

    let valid = errors.is_empty();

    Ok(ValidationResult {
        file: file.to_string(),
        valid,
        errors,
        warnings,
    })
}

fn validate_toml(content: &str, errors: &mut Vec<String>, warnings: &mut Vec<String>) {
    // Try to parse as TOML
    match toml::from_str::<toml::Value>(content) {
        Ok(value) => {
            // Check for common required fields
            if let Some(table) = value.as_table() {
                // Check for common sections
                if !table.contains_key("server") && !table.contains_key("package") {
                    warnings.push("Missing 'server' or 'package' section".to_string());
                }

                // Check for database config
                if table.contains_key("database") {
                    if let Some(db) = table.get("database").and_then(|v| v.as_table()) {
                        if !db.contains_key("url") && !db.contains_key("host") {
                            warnings.push("Database config missing 'url' or 'host'".to_string());
                        }
                    }
                }
            }
        }
        Err(e) => {
            errors.push(format!("TOML parse error: {}", e));
        }
    }
}

fn validate_env(content: &str, errors: &mut Vec<String>, warnings: &mut Vec<String>) {
    for (line_num, line) in content.lines().enumerate() {
        let line = line.trim();

        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Check format (KEY=VALUE)
        if !line.contains('=') {
            errors.push(format!(
                "Line {}: Invalid format (expected KEY=VALUE)",
                line_num + 1
            ));
            continue;
        }

        let parts: Vec<&str> = line.splitn(2, '=').collect();
        if parts.len() != 2 {
            errors.push(format!("Line {}: Invalid format", line_num + 1));
            continue;
        }

        let key = parts[0].trim();
        let value = parts[1].trim();

        // Validate key format
        if !key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
        {
            warnings.push(format!(
                "Line {}: Key '{}' should be uppercase",
                line_num + 1,
                key
            ));
        }

        // Check for empty values
        if value.is_empty() {
            warnings.push(format!("Line {}: Empty value for '{}'", line_num + 1, key));
        }

        // Check for unquoted values with spaces
        if value.contains(' ') && !value.starts_with('"') && !value.starts_with('\'') {
            warnings.push(format!(
                "Line {}: Value with spaces should be quoted",
                line_num + 1
            ));
        }
    }
}

fn print_validation_results(results: &[ValidationResult]) {
    for result in results {
        let status = if result.valid { "✅" } else { "❌" };
        println!("{} {}", status, result.file);

        for error in &result.errors {
            println!("  ❌ Error: {}", error);
        }

        for warning in &result.warnings {
            println!("  ⚠️  Warning: {}", warning);
        }

        if result.valid && result.warnings.is_empty() {
            println!("  ✅ Valid");
        }

        println!();
    }
}

/// Validate specific config file
pub fn validate_file(file: &str) -> Result<(), CliError> {
    println!("⚙️  Validating: {}", file);
    println!();

    if !Path::new(file).exists() {
        return Err(CliError::Validation(format!("File not found: {}", file)));
    }

    let result = validate_config_file(file)?;
    print_validation_results(&[result]);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_toml_valid() {
        let content = r#"
[server]
host = "127.0.0.1"
port = 3000
        "#;
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        validate_toml(content, &mut errors, &mut warnings);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_toml_invalid() {
        let content = r#"
[server
host = "127.0.0.1"
        "#;
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        validate_toml(content, &mut errors, &mut warnings);
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_validate_env_valid() {
        let content = r#"
# Database config
DATABASE_URL=postgres://localhost/mydb
PORT=3000
        "#;
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        validate_env(content, &mut errors, &mut warnings);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_env_invalid() {
        let content = r#"
INVALID LINE
PORT=3000
        "#;
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        validate_env(content, &mut errors, &mut warnings);
        assert!(!errors.is_empty());
    }
}
