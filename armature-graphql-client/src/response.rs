//! GraphQL response types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// GraphQL response from the server.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GraphQLResponse<T = Value> {
    /// The data returned by the query/mutation.
    #[serde(default)]
    pub data: Option<T>,
    /// Errors returned by the server.
    #[serde(default)]
    pub errors: Option<Vec<GraphQLResponseError>>,
    /// Extensions (for tracing, caching info, etc.).
    #[serde(default)]
    pub extensions: Option<Value>,
}

impl<T> GraphQLResponse<T> {
    /// Check if the response has errors.
    pub fn has_errors(&self) -> bool {
        self.errors.as_ref().is_some_and(|e| !e.is_empty())
    }

    /// Get the data, returning an error if there are GraphQL errors.
    pub fn into_result(self) -> crate::Result<T> {
        if let Some(errors) = self.errors {
            if !errors.is_empty() {
                return Err(crate::GraphQLError::GraphQL(errors));
            }
        }
        self.data
            .ok_or_else(|| crate::GraphQLError::Parse("Response contained no data".to_string()))
    }

    /// Get the data, ignoring any errors.
    pub fn data(self) -> Option<T> {
        self.data
    }

    /// Get the errors.
    pub fn errors(&self) -> Option<&[GraphQLResponseError]> {
        self.errors.as_deref()
    }
}

/// A GraphQL error from the server.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GraphQLResponseError {
    /// Error message.
    pub message: String,
    /// Locations in the query where the error occurred.
    #[serde(default)]
    pub locations: Option<Vec<ErrorLocation>>,
    /// Path to the field that caused the error.
    #[serde(default)]
    pub path: Option<Vec<PathSegment>>,
    /// Additional error extensions.
    #[serde(default)]
    pub extensions: Option<Value>,
}

impl std::fmt::Display for GraphQLResponseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(locations) = &self.locations {
            if !locations.is_empty() {
                write!(f, " at ")?;
                for (i, loc) in locations.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}:{}", loc.line, loc.column)?;
                }
            }
        }
        Ok(())
    }
}

/// Location in the GraphQL query.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ErrorLocation {
    /// Line number (1-indexed).
    pub line: u32,
    /// Column number (1-indexed).
    pub column: u32,
}

/// Path segment in a GraphQL error.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum PathSegment {
    /// Field name.
    Field(String),
    /// Array index.
    Index(usize),
}

impl std::fmt::Display for PathSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Field(name) => write!(f, "{}", name),
            Self::Index(idx) => write!(f, "[{}]", idx),
        }
    }
}

/// Format a path as a string.
pub fn format_path(path: &[PathSegment]) -> String {
    path.iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join(".")
}
