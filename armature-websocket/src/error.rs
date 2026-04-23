//! Error types for WebSocket operations.

use thiserror::Error;

/// WebSocket error type.
#[derive(Error, Debug)]
pub enum WebSocketError {
    /// Connection error
    #[error("Connection error: {0}")]
    Connection(String),

    /// Protocol error
    #[error("Protocol error: {0}")]
    Protocol(#[from] tungstenite::Error),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Connection not found
    #[error("Connection not found: {0}")]
    ConnectionNotFound(String),

    /// Room not found
    #[error("Room not found: {0}")]
    RoomNotFound(String),

    /// Connection closed
    #[error("Connection closed")]
    ConnectionClosed,

    /// Send error
    #[error("Failed to send message: {0}")]
    Send(String),

    /// Timeout error
    #[error("Operation timed out")]
    Timeout,

    /// TLS error
    #[error("TLS error: {0}")]
    Tls(String),

    /// Invalid URL
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    /// Server error
    #[error("Server error: {0}")]
    Server(String),
}

/// Result type for WebSocket operations.
pub type WebSocketResult<T> = Result<T, WebSocketError>;
