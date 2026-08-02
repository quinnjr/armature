//! Error types for Armature CLI.

use std::fmt;

/// Result type for CLI operations.
pub type CliResult<T> = Result<T, CliError>;

/// CLI error types.
#[derive(Debug)]
pub enum CliError {
    /// IO error (file operations, etc.)
    Io(std::io::Error),

    /// Template rendering error
    Template(String),

    /// Project configuration error
    Config(String),

    /// Command execution error
    Command(String),

    /// File already exists
    FileExists(String),

    /// Project not found (not in an Armature project directory)
    NotInProject,

    /// Invalid argument
    InvalidArgument(String),

    /// Watch error
    Watch(String),

    /// Build error
    Build(String),

    /// Validation error
    Validation(String),

    /// Tool error (external tool execution)
    Tool(String),

    /// Subcommand is declared but not implemented yet. Surfaced as a hard
    /// failure so scripts and container start-commands never mistake a no-op
    /// for a successful run.
    Unimplemented(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Io(e) => write!(f, "IO error: {}", e),
            CliError::Template(msg) => write!(f, "Template error: {}", msg),
            CliError::Config(msg) => write!(f, "Configuration error: {}", msg),
            CliError::Command(msg) => write!(f, "Command error: {}", msg),
            CliError::FileExists(path) => write!(f, "File already exists: {}", path),
            CliError::NotInProject => write!(
                f,
                "Not in an Armature project directory. Run this command from your project root."
            ),
            CliError::InvalidArgument(msg) => write!(f, "Invalid argument: {}", msg),
            CliError::Watch(msg) => write!(f, "Watch error: {}", msg),
            CliError::Build(msg) => write!(f, "Build error: {}", msg),
            CliError::Validation(msg) => write!(f, "Validation error: {}", msg),
            CliError::Tool(msg) => write!(f, "Tool error: {}", msg),
            CliError::Unimplemented(cmd) => write!(
                f,
                "`armature {}` is not implemented yet and did nothing. Track progress at https://github.com/quinnjr/armature/issues",
                cmd
            ),
        }
    }
}

impl std::error::Error for CliError {}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::Io(e)
    }
}

impl From<handlebars::RenderError> for CliError {
    fn from(e: handlebars::RenderError) -> Self {
        CliError::Template(e.to_string())
    }
}

impl From<toml::de::Error> for CliError {
    fn from(e: toml::de::Error) -> Self {
        CliError::Config(e.to_string())
    }
}

impl From<notify::Error> for CliError {
    fn from(e: notify::Error) -> Self {
        CliError::Watch(e.to_string())
    }
}
