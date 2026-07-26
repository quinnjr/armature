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

impl ToonError {
    /// Map a `serde_toon::Error` produced on the **serialize** path.
    ///
    /// serde's `Custom`/`Message`/`Io` variants carry no direction information,
    /// so the blanket [`From`] impl (which is deserialize-biased) would
    /// mislabel a serialize-time failure as a [`ToonError::DeserializeError`].
    /// Serialize call sites use this constructor instead so those errors are
    /// classified correctly.
    pub(crate) fn from_serialize(err: serde_toon::Error) -> Self {
        ToonError::SerializeError(err.to_string())
    }
}

impl From<serde_toon::Error> for ToonError {
    /// Deserialize-biased conversion.
    ///
    /// This is used on the **deserialize** path (parsing TOON text into a
    /// value). The only unambiguously serialize-side variant
    /// (`UnsupportedType`) is still mapped to [`ToonError::SerializeError`], but
    /// the direction-agnostic variants (`Custom`/`Message`/`Io`) are treated as
    /// deserialization failures here. Serialize call sites must not route
    /// through this impl — they use [`ToonError::from_serialize`] instead.
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
