//! gRPC error types.

use thiserror::Error;
use tonic::Status;

/// Result type for gRPC operations.
pub type Result<T> = std::result::Result<T, GrpcError>;

/// gRPC errors.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum GrpcError {
    /// Transport error.
    #[error("Transport error: {0}")]
    Transport(#[from] tonic::transport::Error),

    /// Status error from gRPC call.
    #[error("gRPC status: {0}")]
    Status(#[from] Status),

    /// Connection error.
    #[error("Connection error: {0}")]
    Connection(String),

    /// Configuration error.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Timeout error.
    #[error("Request timed out")]
    Timeout,

    /// Authentication error.
    #[error("Authentication failed: {0}")]
    Auth(String),

    /// Interceptor error.
    #[error("Interceptor error: {0}")]
    Interceptor(String),

    /// Server error.
    #[error("Server error: {0}")]
    Server(String),
}

impl GrpcError {
    /// Convert to a tonic Status.
    pub fn to_status(&self) -> Status {
        match self {
            Self::Transport(e) => Status::unavailable(e.to_string()),
            Self::Status(s) => s.clone(),
            Self::Connection(msg) => Status::unavailable(msg),
            Self::Config(msg) => Status::invalid_argument(msg),
            Self::Timeout => Status::deadline_exceeded("Request timed out"),
            Self::Auth(msg) => Status::unauthenticated(msg),
            Self::Interceptor(msg) => Status::internal(msg),
            Self::Server(msg) => Status::internal(msg),
        }
    }

    /// Create from a tonic Code and message.
    pub fn from_code(code: tonic::Code, message: impl Into<String>) -> Self {
        Self::Status(Status::new(code, message))
    }
}

impl From<GrpcError> for Status {
    fn from(err: GrpcError) -> Self {
        err.to_status()
    }
}
