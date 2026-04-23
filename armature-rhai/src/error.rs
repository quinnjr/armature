//! Error types for Rhai integration.

use std::path::PathBuf;
use thiserror::Error;

/// Result type for Rhai operations.
pub type Result<T> = std::result::Result<T, RhaiError>;

/// Errors that can occur during Rhai script execution.
#[derive(Debug, Error)]
pub enum RhaiError {
    /// Script file not found.
    #[error("Script not found: {path}")]
    ScriptNotFound { path: PathBuf },

    /// Script compilation error.
    #[error("Script compilation error in {path}: {message}")]
    CompilationError { path: PathBuf, message: String },

    /// Script runtime error.
    #[error("Script runtime error in {path}: {message}")]
    RuntimeError { path: PathBuf, message: String },

    /// Script execution timeout.
    #[error("Script execution timeout after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },

    /// Script exceeded operation limit.
    #[error("Script exceeded maximum operations ({max_ops})")]
    OperationLimit { max_ops: u64 },

    /// Invalid script output.
    #[error("Invalid script output: expected {expected}, got {actual}")]
    InvalidOutput { expected: String, actual: String },

    /// Script returned an error.
    #[error("Script error: {message}")]
    ScriptError { message: String },

    /// Configuration error.
    #[error("Configuration error: {message}")]
    ConfigError { message: String },

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Rhai parse error.
    #[error("Parse error: {0}")]
    Parse(String),

    /// Rhai box error (runtime).
    #[error("Runtime error: {0}")]
    Runtime(String),

    /// Hot reload watcher error.
    #[cfg(feature = "hot-reload")]
    #[error("Watcher error: {0}")]
    Watcher(#[from] notify::Error),
}

impl From<rhai::ParseError> for RhaiError {
    fn from(err: rhai::ParseError) -> Self {
        RhaiError::Parse(err.to_string())
    }
}

impl From<Box<rhai::EvalAltResult>> for RhaiError {
    fn from(err: Box<rhai::EvalAltResult>) -> Self {
        RhaiError::Runtime(err.to_string())
    }
}

impl RhaiError {
    /// Create a script error.
    pub fn script(message: impl Into<String>) -> Self {
        RhaiError::ScriptError {
            message: message.into(),
        }
    }

    /// Create a configuration error.
    pub fn config(message: impl Into<String>) -> Self {
        RhaiError::ConfigError {
            message: message.into(),
        }
    }

    /// Create a compilation error.
    pub fn compilation(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        RhaiError::CompilationError {
            path: path.into(),
            message: message.into(),
        }
    }

    /// Create a runtime error.
    pub fn runtime(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        RhaiError::RuntimeError {
            path: path.into(),
            message: message.into(),
        }
    }
}
