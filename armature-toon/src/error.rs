//! Error types for TOON operations.

use thiserror::Error;

/// Errors that can occur during TOON operations.
#[derive(Debug, Error)]
pub enum ToonError {
    /// Serialization error.
    #[error("TOON serialization error: {0}")]
    SerializeError(String),

    /// Deserialization error.
    #[error("TOON deserialization error: {0}")]
    DeserializeError(String),

    /// UTF-8 encoding error.
    #[error("UTF-8 encoding error: {0}")]
    Utf8Error(String),

    /// IO error.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

impl From<serde_toon::Error> for ToonError {
    fn from(err: serde_toon::Error) -> Self {
        use serde_toon::Error as E;
        let msg = err.to_string();
        match err {
            // Serialization-only failure: a Rust type could not be encoded.
            E::UnsupportedType(_) => ToonError::SerializeError(msg),
            // Parse / deserialization failures — reading TOON text into a value.
            // These are produced by the deserializer and must not be
            // mislabelled as serialization errors.
            E::Syntax { .. }
            | E::TypeMismatch { .. }
            | E::IndentationError { .. }
            | E::InvalidFormat { .. }
            | E::UnexpectedEof { .. }
            | E::Io(_)
            | E::Custom(_)
            | E::Message(_) => ToonError::DeserializeError(msg),
        }
    }
}
