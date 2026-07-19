//! Error types for OpenSearch operations.

use thiserror::Error;

/// OpenSearch error type.
#[derive(Error, Debug)]
pub enum OpenSearchError {
    /// Connection error.
    #[error("Connection error: {0}")]
    Connection(String),

    /// Authentication error.
    #[error("Authentication failed: {0}")]
    Authentication(String),

    /// Index not found.
    #[error("Index not found: {0}")]
    IndexNotFound(String),

    /// Document not found.
    #[error("Document not found: {index}/{id}")]
    DocumentNotFound {
        /// Index name.
        index: String,
        /// Document ID.
        id: String,
    },

    /// Validation error.
    #[error("Validation error: {0}")]
    Validation(String),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Query error.
    #[error("Query error: {0}")]
    Query(String),

    /// Bulk operation error.
    #[error("Bulk operation failed: {succeeded} succeeded, {failed} failed")]
    BulkError {
        /// Number of successful operations.
        succeeded: usize,
        /// Number of failed operations.
        failed: usize,
        /// Error details.
        errors: Vec<String>,
    },

    /// Index already exists.
    #[error("Index already exists: {0}")]
    IndexExists(String),

    /// Timeout error.
    #[error("Operation timed out")]
    Timeout,

    /// Internal OpenSearch error.
    #[error("OpenSearch error: {0}")]
    Internal(String),

    /// Client error from opensearch crate.
    #[error("Client error: {0}")]
    Client(#[from] opensearch::Error),
}

/// Result type alias for OpenSearch operations.
pub type Result<T> = std::result::Result<T, OpenSearchError>;

/// Extract a human-readable reason from an OpenSearch error response body,
/// e.g. `{"error": {"reason": "..."}}`.
pub(crate) fn error_reason(body: &serde_json::Value) -> String {
    body["error"]["reason"]
        .as_str()
        .unwrap_or("Unknown error")
        .to_string()
}

/// Parse an OpenSearch HTTP response body as JSON and, if the response's
/// status code is not a success, translate it into an
/// [`OpenSearchError::Internal`] carrying the server's `error.reason`.
///
/// This is the common tail of nearly every OpenSearch call: send the
/// request, check the status, and on failure surface the body's reason.
/// Callers that need special-case status handling (e.g. mapping 404 to a
/// domain-specific "not found" error) should inspect
/// `response.status_code()` *before* calling this, since it consumes the
/// response. On success, the parsed JSON body is returned so callers that
/// need data out of it (e.g. `_id`, `deleted`, cluster health) don't have
/// to parse it again.
pub(crate) async fn json_or_error(
    response: opensearch::http::response::Response,
) -> Result<serde_json::Value> {
    let status = response.status_code();
    let body: serde_json::Value = response.json().await?;

    if !status.is_success() {
        return Err(OpenSearchError::Internal(error_reason(&body)));
    }

    Ok(body)
}
