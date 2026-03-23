//! Error types for armature-app.

use std::path::PathBuf;
use thiserror::Error;

/// Result type for app operations.
pub type Result<T> = std::result::Result<T, AppError>;

/// Errors that can occur when building or running a Rhai application.
#[derive(Debug, Error)]
pub enum AppError {
    /// Script file not found.
    #[error("Script not found: {path}")]
    ScriptNotFound { path: PathBuf },

    /// Script compilation error.
    #[error("Compilation error in {path}: {message}")]
    Compilation { path: PathBuf, message: String },

    /// Script runtime error.
    #[error("Runtime error: {message}")]
    Runtime { message: String },

    /// Application was not created in the script.
    #[error(
        "Script did not create an application — call Application::create(module) and app.listen(port)"
    )]
    NoApplication,

    /// No listen port was configured.
    #[error("No listen port configured — call app.listen(port)")]
    NoPort,

    /// Service not found during handler execution.
    #[error("Service not found: {name}")]
    ServiceNotFound { name: String },

    /// Method not found on a service.
    #[error("Method `{method}` not found on service `{service}`")]
    MethodNotFound { service: String, method: String },

    /// Guard rejected the request.
    #[error("Guard `{name}` rejected the request")]
    GuardRejected { name: String },

    /// Builder error during app assembly.
    #[error("Builder error: {message}")]
    Builder { message: String },

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Rhai engine error.
    #[error("Rhai error: {0}")]
    Rhai(String),

    /// Armature core error.
    #[error("Framework error: {0}")]
    Core(#[from] armature_core::Error),
}

impl From<Box<rhai::EvalAltResult>> for AppError {
    fn from(err: Box<rhai::EvalAltResult>) -> Self {
        AppError::Rhai(err.to_string())
    }
}

impl From<rhai::ParseError> for AppError {
    fn from(err: rhai::ParseError) -> Self {
        AppError::Rhai(err.to_string())
    }
}
