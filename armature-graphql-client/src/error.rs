//! GraphQL client error types.

use thiserror::Error;

/// Result type for GraphQL client operations.
pub type Result<T> = std::result::Result<T, GraphQLError>;

/// GraphQL client errors.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum GraphQLError {
    /// HTTP request failed.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// GraphQL errors returned by the server.
    #[error("GraphQL errors: {0:?}")]
    GraphQL(Vec<crate::GraphQLResponseError>),

    /// WebSocket error (for subscriptions).
    #[error("WebSocket error: {0}")]
    WebSocket(String),

    /// Connection error.
    #[error("Connection error: {0}")]
    Connection(String),

    /// Timeout error.
    #[error("Request timed out")]
    Timeout,

    /// Invalid URL.
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    /// Subscription error.
    #[error("Subscription error: {0}")]
    Subscription(String),

    /// Configuration error.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Parse error.
    #[error("Parse error: {0}")]
    Parse(String),

    /// Authentication error.
    #[error("Authentication error: {0}")]
    Auth(String),
}

impl GraphQLError {
    /// Check if this is a network error.
    pub fn is_network_error(&self) -> bool {
        matches!(
            self,
            Self::Http(_) | Self::Connection(_) | Self::WebSocket(_)
        )
    }

    /// Check if this is a GraphQL error (server-side).
    pub fn is_graphql_error(&self) -> bool {
        matches!(self, Self::GraphQL(_))
    }

    /// Check if this is a timeout error.
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout)
    }

    /// Get GraphQL errors if this is a GraphQL error.
    pub fn graphql_errors(&self) -> Option<&[crate::GraphQLResponseError]> {
        match self {
            Self::GraphQL(errors) => Some(errors),
            _ => None,
        }
    }
}
