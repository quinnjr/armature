//! Centralized Error Transformation
//!
//! This module provides a comprehensive error transformation system for handling,
//! formatting, and customizing error responses in a centralized manner.
//!
//! # Features
//!
//! - Configurable error response formats (JSON, plain text, HTML)
//! - Error context and metadata attachment
//! - Sensitive data filtering
//! - Custom error transformers
//! - Error logging integration
//! - Problem Details (RFC 7807) support
//!
//! # Examples
//!
//! ## Basic Usage
//!
//! ```ignore
//! use armature_core::error_transform::{ErrorTransformer, ErrorConfig, ResponseFormat};
//!
//! let transformer = ErrorTransformer::new()
//!     .format(ResponseFormat::Json)
//!     .include_stack_trace(false)
//!     .filter_sensitive_data(true);
//!
//! let response = transformer.transform(&error, &request);
//! ```
//!
//! ## Custom Transformer
//!
//! ```ignore
//! let transformer = ErrorTransformer::new()
//!     .with_transformer(|error, ctx| {
//!         // Custom transformation logic
//!         Some(ErrorResponse::new(error.status_code())
//!             .message("Custom error message"))
//!     });
//! ```

use crate::{Error, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

/// Compiled PII/secret redaction patterns used by [`ErrorTransformer`].
///
/// These regexes are compiled exactly once at first use rather than on every
/// error transformation. Each entry pairs a case-insensitive compiled pattern
/// with its replacement string. Behaviour is identical to compiling the same
/// patterns per-call; only the compilation cost is amortized.
static SENSITIVE_PATTERNS: LazyLock<Vec<(regex::Regex, &'static str)>> = LazyLock::new(|| {
    let patterns: [(&str, &str); 9] = [
        // Passwords
        (r"password[=:]\s*\S+", "password=[FILTERED]"),
        (r"pwd[=:]\s*\S+", "pwd=[FILTERED]"),
        // API keys
        (r"api[_-]?key[=:]\s*\S+", "api_key=[FILTERED]"),
        (r"apikey[=:]\s*\S+", "apikey=[FILTERED]"),
        // Tokens
        (r"token[=:]\s*\S+", "token=[FILTERED]"),
        (r"bearer\s+\S+", "Bearer [FILTERED]"),
        // Secrets
        (r"secret[=:]\s*\S+", "secret=[FILTERED]"),
        // Credit cards (simple pattern)
        (
            r"\b\d{4}[- ]?\d{4}[- ]?\d{4}[- ]?\d{4}\b",
            "[CARD FILTERED]",
        ),
        // SSN
        (r"\b\d{3}[- ]?\d{2}[- ]?\d{4}\b", "[SSN FILTERED]"),
    ];

    patterns
        .into_iter()
        .filter_map(|(pattern, replacement)| {
            regex::Regex::new(&format!("(?i){pattern}"))
                .ok()
                .map(|re| (re, replacement))
        })
        .collect()
});

// ============================================================================
// Error Response Types
// ============================================================================

/// Format for error responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResponseFormat {
    /// JSON format (default) - Simple JSON with status, message, etc.
    #[default]
    Json,
    /// Plain text format
    PlainText,
    /// HTML format - Styled error page
    Html,
    /// RFC 7807 Problem Details JSON
    ProblemDetails,
    /// JSON:API error format (https://jsonapi.org/format/#errors)
    JsonApi,
    /// GraphQL error format (https://spec.graphql.org)
    GraphQL,
    /// Google/gRPC-style error format
    Google,
    /// AWS-style error format
    Aws,
    /// Azure-style error format
    Azure,
}

/// A structured error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// HTTP status code
    pub status: u16,
    /// Error code (application-specific)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Human-readable error message
    pub message: String,
    /// Detailed error description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    /// Error type/category
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    /// Request path that caused the error
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Timestamp of the error
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// Request ID for tracing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Additional metadata
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub metadata: HashMap<String, serde_json::Value>,
    /// Validation errors (for 422 responses)
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub validation_errors: Vec<ValidationError>,
    /// Stack trace (if enabled)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_trace: Option<String>,
}

impl ErrorResponse {
    /// Create a new error response.
    pub fn new(status: u16) -> Self {
        Self {
            status,
            code: None,
            message: String::new(),
            details: None,
            error_type: None,
            path: None,
            timestamp: Some(httpdate::fmt_http_date(std::time::SystemTime::now())),
            request_id: None,
            metadata: HashMap::new(),
            validation_errors: Vec::new(),
            stack_trace: None,
        }
    }

    /// Set the error message.
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// Set the error code.
    pub fn code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Set the details.
    pub fn details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    /// Set the error type.
    pub fn error_type(mut self, error_type: impl Into<String>) -> Self {
        self.error_type = Some(error_type.into());
        self
    }

    /// Set the request path.
    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Set the request ID.
    pub fn request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        if let Ok(json_value) = serde_json::to_value(value) {
            self.metadata.insert(key.into(), json_value);
        }
        self
    }

    /// Add a validation error.
    pub fn with_validation_error(mut self, error: ValidationError) -> Self {
        self.validation_errors.push(error);
        self
    }

    /// Add multiple validation errors.
    pub fn with_validation_errors(mut self, errors: Vec<ValidationError>) -> Self {
        self.validation_errors.extend(errors);
        self
    }

    /// Set the stack trace.
    pub fn stack_trace(mut self, trace: impl Into<String>) -> Self {
        self.stack_trace = Some(trace.into());
        self
    }

    /// Convert to JSON string.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| {
            format!(
                r#"{{"status":{},"message":"{}"}}"#,
                self.status, self.message
            )
        })
    }

    /// Convert to plain text.
    pub fn to_plain_text(&self) -> String {
        let mut text = format!("Error {}: {}", self.status, self.message);
        if let Some(ref details) = self.details {
            text.push_str(&format!("\nDetails: {}", details));
        }
        if !self.validation_errors.is_empty() {
            text.push_str("\nValidation Errors:");
            for err in &self.validation_errors {
                text.push_str(&format!("\n  - {}: {}", err.field, err.message));
            }
        }
        text
    }

    /// Convert to HTML.
    pub fn to_html(&self) -> String {
        let mut html = format!(
            r#"<!DOCTYPE html>
<html>
<head>
    <title>Error {}</title>
    <style>
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; padding: 40px; background: #f5f5f5; }}
        .error {{ background: white; border-radius: 8px; padding: 24px; max-width: 600px; margin: 0 auto; box-shadow: 0 2px 8px rgba(0,0,0,0.1); }}
        h1 {{ color: #e53935; margin-top: 0; }}
        .status {{ color: #666; font-size: 14px; }}
        .details {{ background: #f5f5f5; padding: 12px; border-radius: 4px; margin-top: 16px; }}
        .validation {{ margin-top: 16px; }}
        .validation li {{ color: #d32f2f; }}
    </style>
</head>
<body>
    <div class="error">
        <p class="status">Error {}</p>
        <h1>{}</h1>"#,
            self.status,
            self.status,
            html_escape(&self.message)
        );

        if let Some(ref details) = self.details {
            html.push_str(&format!(
                r#"<div class="details">{}</div>"#,
                html_escape(details)
            ));
        }

        if !self.validation_errors.is_empty() {
            html.push_str(r#"<div class="validation"><h3>Validation Errors</h3><ul>"#);
            for err in &self.validation_errors {
                html.push_str(&format!(
                    "<li><strong>{}</strong>: {}</li>",
                    html_escape(&err.field),
                    html_escape(&err.message)
                ));
            }
            html.push_str("</ul></div>");
        }

        html.push_str("</div></body></html>");
        html
    }

    /// Convert to Problem Details (RFC 7807).
    pub fn to_problem_details(&self) -> String {
        let problem = ProblemDetails {
            type_uri: self
                .error_type
                .clone()
                .unwrap_or_else(|| "about:blank".to_string()),
            title: self.message.clone(),
            status: self.status,
            detail: self.details.clone(),
            instance: self.path.clone(),
            extensions: self.metadata.clone(),
        };
        serde_json::to_string_pretty(&problem).unwrap_or_else(|_| self.to_json())
    }

    /// Convert to JSON:API error format.
    /// https://jsonapi.org/format/#errors
    pub fn to_json_api(&self) -> String {
        let error = JsonApiError {
            id: self.request_id.clone(),
            status: self.status.to_string(),
            code: self.code.clone(),
            title: Some(self.message.clone()),
            detail: self.details.clone(),
            source: if !self.validation_errors.is_empty() {
                Some(JsonApiErrorSource {
                    pointer: self
                        .validation_errors
                        .first()
                        .map(|e| format!("/data/attributes/{}", e.field)),
                    parameter: None,
                    header: None,
                })
            } else {
                None
            },
            meta: if self.metadata.is_empty() {
                None
            } else {
                Some(self.metadata.clone())
            },
        };

        let response = JsonApiErrorResponse {
            errors: std::iter::once(error)
                .chain(self.validation_errors.iter().skip(1).map(|v| JsonApiError {
                    id: None,
                    status: self.status.to_string(),
                    code: self.code.clone(),
                    title: Some(v.message.clone()),
                    detail: v.rule.clone(),
                    source: Some(JsonApiErrorSource {
                        pointer: Some(format!("/data/attributes/{}", v.field)),
                        parameter: None,
                        header: None,
                    }),
                    meta: None,
                }))
                .collect(),
        };

        serde_json::to_string_pretty(&response).unwrap_or_else(|_| self.to_json())
    }

    /// Convert to GraphQL error format.
    /// https://spec.graphql.org/October2021/#sec-Errors
    pub fn to_graphql(&self) -> String {
        let errors: Vec<GraphQLError> = if self.validation_errors.is_empty() {
            vec![GraphQLError {
                message: self.message.clone(),
                locations: None,
                path: self
                    .path
                    .as_ref()
                    .map(|p| vec![serde_json::Value::String(p.clone())]),
                extensions: Some(GraphQLErrorExtensions {
                    code: self.code.clone().or(self.error_type.clone()),
                    status: Some(self.status),
                    timestamp: self.timestamp.clone(),
                    details: self.details.clone(),
                }),
            }]
        } else {
            self.validation_errors
                .iter()
                .map(|v| GraphQLError {
                    message: v.message.clone(),
                    locations: None,
                    path: Some(vec![serde_json::Value::String(v.field.clone())]),
                    extensions: Some(GraphQLErrorExtensions {
                        code: v.rule.clone(),
                        status: Some(self.status),
                        timestamp: None,
                        details: None,
                    }),
                })
                .collect()
        };

        let response = GraphQLErrorResponse { data: None, errors };

        serde_json::to_string_pretty(&response).unwrap_or_else(|_| self.to_json())
    }

    /// Convert to Google/gRPC-style error format.
    pub fn to_google(&self) -> String {
        let error = GoogleError {
            error: GoogleErrorBody {
                code: self.status,
                message: self.message.clone(),
                status: self
                    .error_type
                    .clone()
                    .unwrap_or_else(|| google_status_from_http(self.status)),
                details: self
                    .validation_errors
                    .iter()
                    .map(|v| GoogleErrorDetail {
                        type_url: "type.googleapis.com/google.rpc.BadRequest.FieldViolation"
                            .to_string(),
                        field: v.field.clone(),
                        description: v.message.clone(),
                    })
                    .collect(),
            },
        };

        serde_json::to_string_pretty(&error).unwrap_or_else(|_| self.to_json())
    }

    /// Convert to AWS-style error format.
    pub fn to_aws(&self) -> String {
        let error = AwsError {
            __type: self
                .error_type
                .clone()
                .unwrap_or_else(|| format!("{}Exception", aws_error_type(self.status))),
            message: self.message.clone(),
            code: self.code.clone(),
            request_id: self.request_id.clone(),
        };

        serde_json::to_string_pretty(&error).unwrap_or_else(|_| self.to_json())
    }

    /// Convert to Azure-style error format.
    pub fn to_azure(&self) -> String {
        let inner_errors: Vec<AzureInnerError> = self
            .validation_errors
            .iter()
            .map(|v| AzureInnerError {
                code: v
                    .rule
                    .clone()
                    .unwrap_or_else(|| "ValidationError".to_string()),
                message: v.message.clone(),
                target: Some(v.field.clone()),
            })
            .collect();

        let error = AzureError {
            error: AzureErrorBody {
                code: self.code.clone().unwrap_or_else(|| {
                    self.error_type
                        .clone()
                        .unwrap_or_else(|| "Error".to_string())
                }),
                message: self.message.clone(),
                target: self.path.clone(),
                details: if inner_errors.is_empty() {
                    None
                } else {
                    Some(inner_errors)
                },
                innererror: self.details.as_ref().map(|d| AzureInnerErrorInfo {
                    code: self.error_type.clone(),
                    message: Some(d.clone()),
                }),
            },
        };

        serde_json::to_string_pretty(&error).unwrap_or_else(|_| self.to_json())
    }

    /// Convert to HttpResponse.
    pub fn into_http_response(self, format: ResponseFormat) -> HttpResponse {
        let (body, content_type) = match format {
            ResponseFormat::Json => (self.to_json(), "application/json"),
            ResponseFormat::PlainText => (self.to_plain_text(), "text/plain; charset=utf-8"),
            ResponseFormat::Html => (self.to_html(), "text/html; charset=utf-8"),
            ResponseFormat::ProblemDetails => {
                (self.to_problem_details(), "application/problem+json")
            }
            ResponseFormat::JsonApi => (self.to_json_api(), "application/vnd.api+json"),
            ResponseFormat::GraphQL => (self.to_graphql(), "application/json"),
            ResponseFormat::Google => (self.to_google(), "application/json"),
            ResponseFormat::Aws => (self.to_aws(), "application/x-amz-json-1.1"),
            ResponseFormat::Azure => (self.to_azure(), "application/json"),
        };

        HttpResponse::new(self.status)
            .with_header("Content-Type".to_string(), content_type.to_string())
            .with_body(body.into_bytes())
    }
}

impl Default for ErrorResponse {
    fn default() -> Self {
        Self::new(500).message("Internal Server Error")
    }
}

/// A validation error for a specific field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    /// Field name that failed validation
    pub field: String,
    /// Validation error message
    pub message: String,
    /// Validation rule that failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    /// Invalid value (sanitized)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
}

impl ValidationError {
    /// Create a new validation error.
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
            rule: None,
            value: None,
        }
    }

    /// Set the rule that failed.
    pub fn rule(mut self, rule: impl Into<String>) -> Self {
        self.rule = Some(rule.into());
        self
    }

    /// Set the invalid value.
    pub fn value(mut self, value: impl Serialize) -> Self {
        self.value = serde_json::to_value(value).ok();
        self
    }
}

/// RFC 7807 Problem Details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProblemDetails {
    /// A URI reference that identifies the problem type
    #[serde(rename = "type")]
    pub type_uri: String,
    /// A short, human-readable summary of the problem
    pub title: String,
    /// The HTTP status code
    pub status: u16,
    /// A human-readable explanation specific to this occurrence
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// A URI reference that identifies the specific occurrence
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// Additional properties
    #[serde(flatten)]
    pub extensions: HashMap<String, serde_json::Value>,
}

// ============================================================================
// JSON:API Error Format (https://jsonapi.org/format/#errors)
// ============================================================================

/// JSON:API error response wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonApiErrorResponse {
    /// Array of error objects
    pub errors: Vec<JsonApiError>,
}

/// JSON:API error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonApiError {
    /// Unique identifier for this occurrence
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// HTTP status code as string
    pub status: String,
    /// Application-specific error code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Short, human-readable summary
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Human-readable explanation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Object containing references to source of error
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<JsonApiErrorSource>,
    /// Non-standard meta-information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<HashMap<String, serde_json::Value>>,
}

/// JSON:API error source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonApiErrorSource {
    /// JSON Pointer to the value in request document
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    /// String indicating which query parameter caused the error
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<String>,
    /// Name of request header that caused the error
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
}

// ============================================================================
// GraphQL Error Format (https://spec.graphql.org/October2021/#sec-Errors)
// ============================================================================

/// GraphQL error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQLErrorResponse {
    /// Response data (null for errors)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    /// Array of errors
    pub errors: Vec<GraphQLError>,
}

/// GraphQL error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQLError {
    /// Error message
    pub message: String,
    /// Locations in GraphQL document
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locations: Option<Vec<GraphQLErrorLocation>>,
    /// Path to response field which experienced the error
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<Vec<serde_json::Value>>,
    /// Implementation-specific extensions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<GraphQLErrorExtensions>,
}

/// GraphQL error location.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQLErrorLocation {
    /// Line number (1-indexed)
    pub line: u32,
    /// Column number (1-indexed)
    pub column: u32,
}

/// GraphQL error extensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQLErrorExtensions {
    /// Error code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// HTTP status code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// Additional details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

// ============================================================================
// Google/gRPC Error Format
// ============================================================================

/// Google-style error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleError {
    /// Error body
    pub error: GoogleErrorBody,
}

/// Google error body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleErrorBody {
    /// HTTP status code
    pub code: u16,
    /// Error message
    pub message: String,
    /// gRPC status (INVALID_ARGUMENT, NOT_FOUND, etc.)
    pub status: String,
    /// Error details
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub details: Vec<GoogleErrorDetail>,
}

/// Google error detail (field violation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleErrorDetail {
    /// Type URL
    #[serde(rename = "@type")]
    pub type_url: String,
    /// Field name
    pub field: String,
    /// Description
    pub description: String,
}

/// Map HTTP status to gRPC status string.
fn google_status_from_http(status: u16) -> String {
    match status {
        400 => "INVALID_ARGUMENT",
        401 => "UNAUTHENTICATED",
        403 => "PERMISSION_DENIED",
        404 => "NOT_FOUND",
        409 => "ALREADY_EXISTS",
        429 => "RESOURCE_EXHAUSTED",
        499 => "CANCELLED",
        500 => "INTERNAL",
        501 => "UNIMPLEMENTED",
        503 => "UNAVAILABLE",
        504 => "DEADLINE_EXCEEDED",
        _ => "UNKNOWN",
    }
    .to_string()
}

// ============================================================================
// AWS Error Format
// ============================================================================

/// AWS-style error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwsError {
    /// Error type (e.g., "ValidationException")
    #[serde(rename = "__type")]
    pub __type: String,
    /// Error message
    pub message: String,
    /// Error code (optional)
    #[serde(rename = "Code", skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Request ID
    #[serde(rename = "RequestId", skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// Map HTTP status to AWS error type prefix.
fn aws_error_type(status: u16) -> String {
    match status {
        400 => "Validation",
        401 => "UnauthorizedAccess",
        403 => "AccessDenied",
        404 => "ResourceNotFound",
        409 => "Conflict",
        429 => "Throttling",
        500 => "InternalService",
        503 => "ServiceUnavailable",
        _ => "Service",
    }
    .to_string()
}

// ============================================================================
// Azure Error Format
// ============================================================================

/// Azure-style error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AzureError {
    /// Error body
    pub error: AzureErrorBody,
}

/// Azure error body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AzureErrorBody {
    /// Error code
    pub code: String,
    /// Error message
    pub message: String,
    /// Target of the error
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Array of details about specific errors
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Vec<AzureInnerError>>,
    /// Inner error for additional details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub innererror: Option<AzureInnerErrorInfo>,
}

/// Azure inner error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AzureInnerError {
    /// Error code
    pub code: String,
    /// Error message
    pub message: String,
    /// Target
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// Azure inner error info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AzureInnerErrorInfo {
    /// Error code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Error message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// ============================================================================
// Error Context
// ============================================================================

/// Context information about an error occurrence.
#[derive(Debug, Clone)]
pub struct ErrorContext {
    /// The original request
    pub request: HttpRequest,
    /// Request ID
    pub request_id: Option<String>,
    /// User ID (if authenticated)
    pub user_id: Option<String>,
    /// Additional context data
    pub data: HashMap<String, serde_json::Value>,
}

impl ErrorContext {
    /// Create error context from a request.
    pub fn from_request(request: HttpRequest) -> Self {
        // Lookup is case-insensitive, so the second spelling was redundant.
        let request_id = request.headers.get("x-request-id").map(str::to_owned);

        Self {
            request,
            request_id,
            user_id: None,
            data: HashMap::new(),
        }
    }

    /// Set the user ID.
    pub fn user_id(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    /// Add context data.
    pub fn with_data(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        if let Ok(json_value) = serde_json::to_value(value) {
            self.data.insert(key.into(), json_value);
        }
        self
    }
}

// ============================================================================
// Error Transformer
// ============================================================================

/// Type alias for custom transformer functions.
pub type TransformerFn = Arc<dyn Fn(&Error, &ErrorContext) -> Option<ErrorResponse> + Send + Sync>;

/// Type alias for error filter functions.
pub type FilterFn = Arc<dyn Fn(&Error) -> Error + Send + Sync>;

/// Type alias for error logging functions.
pub type LoggerFn = Arc<dyn Fn(&Error, &ErrorContext, &ErrorResponse) + Send + Sync>;

/// Centralized error transformer.
pub struct ErrorTransformer {
    /// Response format
    format: ResponseFormat,
    /// Whether to include stack traces
    include_stack_trace: bool,
    /// Whether to filter sensitive data
    filter_sensitive: bool,
    /// Custom transformers (checked in order)
    transformers: Vec<TransformerFn>,
    /// Error filters
    filters: Vec<FilterFn>,
    /// Loggers
    loggers: Vec<LoggerFn>,
    /// Whether to include request path
    include_path: bool,
    /// Whether to include timestamp
    include_timestamp: bool,
    /// Custom error codes mapping
    error_codes: HashMap<String, String>,
    /// Default error messages for production
    production_mode: bool,
}

impl ErrorTransformer {
    /// Create a new error transformer with default settings.
    pub fn new() -> Self {
        Self {
            format: ResponseFormat::Json,
            include_stack_trace: false,
            filter_sensitive: true,
            transformers: Vec::new(),
            filters: Vec::new(),
            loggers: Vec::new(),
            include_path: true,
            include_timestamp: true,
            error_codes: HashMap::new(),
            production_mode: true,
        }
    }

    /// Set the response format.
    pub fn format(mut self, format: ResponseFormat) -> Self {
        self.format = format;
        self
    }

    /// Enable or disable stack trace inclusion.
    pub fn include_stack_trace(mut self, include: bool) -> Self {
        self.include_stack_trace = include;
        self
    }

    /// Enable or disable sensitive data filtering.
    pub fn filter_sensitive_data(mut self, filter: bool) -> Self {
        self.filter_sensitive = filter;
        self
    }

    /// Enable or disable path inclusion.
    pub fn include_path(mut self, include: bool) -> Self {
        self.include_path = include;
        self
    }

    /// Enable or disable timestamp inclusion.
    pub fn include_timestamp(mut self, include: bool) -> Self {
        self.include_timestamp = include;
        self
    }

    /// Set production mode (hides internal error details).
    pub fn production_mode(mut self, production: bool) -> Self {
        self.production_mode = production;
        self
    }

    /// Add a custom transformer.
    pub fn with_transformer<F>(mut self, transformer: F) -> Self
    where
        F: Fn(&Error, &ErrorContext) -> Option<ErrorResponse> + Send + Sync + 'static,
    {
        self.transformers.push(Arc::new(transformer));
        self
    }

    /// Add an error filter.
    pub fn with_filter<F>(mut self, filter: F) -> Self
    where
        F: Fn(&Error) -> Error + Send + Sync + 'static,
    {
        self.filters.push(Arc::new(filter));
        self
    }

    /// Add a logger.
    pub fn with_logger<F>(mut self, logger: F) -> Self
    where
        F: Fn(&Error, &ErrorContext, &ErrorResponse) + Send + Sync + 'static,
    {
        self.loggers.push(Arc::new(logger));
        self
    }

    /// Map an error type to a code.
    pub fn map_error_code(
        mut self,
        error_type: impl Into<String>,
        code: impl Into<String>,
    ) -> Self {
        self.error_codes.insert(error_type.into(), code.into());
        self
    }

    /// Transform an error into an HTTP response.
    pub fn transform(&self, error: &Error, request: &HttpRequest) -> HttpResponse {
        let context = ErrorContext::from_request(request.clone());
        self.transform_with_context(error, &context)
    }

    /// Transform an error with full context.
    pub fn transform_with_context(&self, error: &Error, context: &ErrorContext) -> HttpResponse {
        // Apply filters
        let filtered_error = self.apply_filters(error);

        // Try custom transformers first
        for transformer in &self.transformers {
            if let Some(response) = transformer(&filtered_error, context) {
                return self.finalize_response(response, &filtered_error, context);
            }
        }

        // Default transformation
        let response = self.default_transform(&filtered_error, context);
        self.finalize_response(response, &filtered_error, context)
    }

    /// Apply error filters.
    fn apply_filters(&self, error: &Error) -> Error {
        let mut current = if self.filter_sensitive {
            self.filter_error_message(error)
        } else {
            self.clone_error(error)
        };

        // Apply user-registered filters in order
        for filter in &self.filters {
            current = filter(&current);
        }

        current
    }

    /// Clone an error (simplified).
    fn clone_error(&self, error: &Error) -> Error {
        map_error_message(error, |msg| msg.to_string())
    }

    /// Filter sensitive information from error messages.
    fn filter_error_message(&self, error: &Error) -> Error {
        map_error_message(error, |msg| self.filter_sensitive_string(msg))
    }

    /// Filter sensitive strings.
    ///
    /// Uses the module-level [`SENSITIVE_PATTERNS`] regexes, which are compiled
    /// once on first use rather than on every call.
    fn filter_sensitive_string(&self, s: &str) -> String {
        let mut result = s.to_string();

        for (regex, replacement) in SENSITIVE_PATTERNS.iter() {
            result = regex.replace_all(&result, *replacement).to_string();
        }

        result
    }

    /// Default error transformation.
    fn default_transform(&self, error: &Error, context: &ErrorContext) -> ErrorResponse {
        let status = error.status_code();
        let error_type = self.get_error_type(error);

        let mut response = ErrorResponse::new(status).error_type(&error_type);

        // Set message
        if self.production_mode && error.is_server_error() {
            response = response.message("An internal server error occurred");
        } else {
            response = response.message(error.to_string());
        }

        // Add error code if mapped
        if let Some(code) = self.error_codes.get(&error_type) {
            response = response.code(code);
        }

        // Add path
        if self.include_path {
            response = response.path(context.request.path_str());
        }

        // Add request ID
        if let Some(ref request_id) = context.request_id {
            response = response.request_id(request_id);
        }

        // Add timestamp
        if !self.include_timestamp {
            response.timestamp = None;
        }

        response
    }

    /// Get the error type string.
    fn get_error_type(&self, error: &Error) -> String {
        match error {
            Error::BadRequest(_) => "BAD_REQUEST".to_string(),
            Error::Unauthorized(_) => "UNAUTHORIZED".to_string(),
            Error::Forbidden(_) => "FORBIDDEN".to_string(),
            Error::NotFound(_) => "NOT_FOUND".to_string(),
            Error::Validation(_) => "VALIDATION_ERROR".to_string(),
            Error::Internal(_) => "INTERNAL_ERROR".to_string(),
            Error::Conflict(_) => "CONFLICT".to_string(),
            Error::TooManyRequests(_) => "RATE_LIMITED".to_string(),
            Error::ServiceUnavailable(_) => "SERVICE_UNAVAILABLE".to_string(),
            Error::RequestTimeout(_) => "TIMEOUT".to_string(),
            _ => "ERROR".to_string(),
        }
    }

    /// Finalize the response and log.
    fn finalize_response(
        &self,
        response: ErrorResponse,
        error: &Error,
        context: &ErrorContext,
    ) -> HttpResponse {
        // Call loggers
        for logger in &self.loggers {
            logger(error, context, &response);
        }

        response.into_http_response(self.format)
    }
}

impl Default for ErrorTransformer {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ErrorTransformer {
    fn clone(&self) -> Self {
        Self {
            format: self.format,
            include_stack_trace: self.include_stack_trace,
            filter_sensitive: self.filter_sensitive,
            transformers: self.transformers.clone(),
            filters: self.filters.clone(),
            loggers: self.loggers.clone(),
            include_path: self.include_path,
            include_timestamp: self.include_timestamp,
            error_codes: self.error_codes.clone(),
            production_mode: self.production_mode,
        }
    }
}

// ============================================================================
// Preset Configurations
// ============================================================================

impl ErrorTransformer {
    /// Development configuration (verbose, includes stack traces).
    pub fn development() -> Self {
        Self::new()
            .production_mode(false)
            .include_stack_trace(true)
            .filter_sensitive_data(false)
            .with_logger(|error, ctx, _response| {
                eprintln!(
                    "[DEV ERROR] {} {} - {:?}",
                    ctx.request.method, ctx.request.path, error
                );
            })
    }

    /// Production configuration (safe, minimal information).
    pub fn production() -> Self {
        Self::new()
            .production_mode(true)
            .include_stack_trace(false)
            .filter_sensitive_data(true)
            .with_logger(|error, ctx, response| {
                // Log to tracing
                if error.is_server_error() {
                    crate::logging::error!(
                        status = response.status,
                        path = %ctx.request.path,
                        request_id = ctx.request_id,
                        error = %error,
                        "Server error occurred"
                    );
                } else {
                    crate::logging::warn!(
                        status = response.status,
                        path = %ctx.request.path,
                        request_id = ctx.request_id,
                        "Client error occurred"
                    );
                }
            })
    }

    /// API configuration (Problem Details format).
    pub fn api() -> Self {
        Self::new()
            .format(ResponseFormat::ProblemDetails)
            .production_mode(true)
            .filter_sensitive_data(true)
    }
}

// ============================================================================
// Error Response Builder
// ============================================================================

/// Builder for creating standardized error responses.
pub struct ErrorResponseBuilder {
    transformer: ErrorTransformer,
}

impl ErrorResponseBuilder {
    /// Create a new error response builder.
    pub fn new() -> Self {
        Self {
            transformer: ErrorTransformer::new(),
        }
    }

    /// Use a specific transformer.
    pub fn with_transformer(mut self, transformer: ErrorTransformer) -> Self {
        self.transformer = transformer;
        self
    }

    /// Create a bad request (400) response.
    pub fn bad_request(message: impl Into<String>) -> HttpResponse {
        let error = Error::BadRequest(message.into());
        ErrorTransformer::new().transform(&error, &HttpRequest::new("", ""))
    }

    /// Create an unauthorized (401) response.
    pub fn unauthorized(message: impl Into<String>) -> HttpResponse {
        let error = Error::Unauthorized(message.into());
        ErrorTransformer::new().transform(&error, &HttpRequest::new("", ""))
    }

    /// Create a forbidden (403) response.
    pub fn forbidden(message: impl Into<String>) -> HttpResponse {
        let error = Error::Forbidden(message.into());
        ErrorTransformer::new().transform(&error, &HttpRequest::new("", ""))
    }

    /// Create a not found (404) response.
    pub fn not_found(message: impl Into<String>) -> HttpResponse {
        let error = Error::NotFound(message.into());
        ErrorTransformer::new().transform(&error, &HttpRequest::new("", ""))
    }

    /// Create an internal server error (500) response.
    pub fn internal_error(message: impl Into<String>) -> HttpResponse {
        let error = Error::Internal(message.into());
        ErrorTransformer::new().transform(&error, &HttpRequest::new("", ""))
    }

    /// Create a validation error (422) response with field errors.
    pub fn validation_error(errors: Vec<ValidationError>) -> HttpResponse {
        let response = ErrorResponse::new(422)
            .message("Validation failed")
            .error_type("VALIDATION_ERROR")
            .with_validation_errors(errors);

        response.into_http_response(ResponseFormat::Json)
    }
}

impl Default for ErrorResponseBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Escape a string for safe interpolation into HTML text content.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

/// Apply `f` to the error's message payload, rebuilding the same variant so
/// the status code is preserved. `Io` carries no `String` payload and maps to
/// `Internal`, which shares its 500 status.
fn map_error_message(error: &Error, f: impl FnOnce(&str) -> String) -> Error {
    macro_rules! map_variants {
        ($($variant:ident),* $(,)?) => {
            match error {
                $(Error::$variant(msg) => Error::$variant(f(msg)),)*
                other => Error::Internal(f(&other.to_string())),
            }
        };
    }
    map_variants!(
        Http,
        RouteNotFound,
        MethodNotAllowed,
        DependencyInjection,
        ProviderNotFound,
        Serialization,
        Deserialization,
        Validation,
        Internal,
        Forbidden,
        BadRequest,
        Unauthorized,
        PaymentRequired,
        NotFound,
        NotAcceptable,
        ProxyAuthenticationRequired,
        RequestTimeout,
        Conflict,
        Gone,
        LengthRequired,
        PreconditionFailed,
        PayloadTooLarge,
        UriTooLong,
        UnsupportedMediaType,
        RangeNotSatisfiable,
        ExpectationFailed,
        ImATeapot,
        MisdirectedRequest,
        UnprocessableEntity,
        Locked,
        FailedDependency,
        TooEarly,
        UpgradeRequired,
        PreconditionRequired,
        TooManyRequests,
        RequestHeaderFieldsTooLarge,
        UnavailableForLegalReasons,
        NotImplemented,
        BadGateway,
        ServiceUnavailable,
        GatewayTimeout,
        HttpVersionNotSupported,
        VariantAlsoNegotiates,
        InsufficientStorage,
        LoopDetected,
        NotExtended,
        NetworkAuthenticationRequired,
    )
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_response_new() {
        let response = ErrorResponse::new(404).message("Not found");
        assert_eq!(response.status, 404);
        assert_eq!(response.message, "Not found");
    }

    #[test]
    fn test_error_response_builder() {
        let response = ErrorResponse::new(400)
            .message("Bad request")
            .code("ERR_001")
            .details("Invalid input")
            .error_type("VALIDATION")
            .path("/api/users")
            .request_id("req-123");

        assert_eq!(response.status, 400);
        assert_eq!(response.code, Some("ERR_001".to_string()));
        assert_eq!(response.details, Some("Invalid input".to_string()));
    }

    #[test]
    fn test_validation_error() {
        let err = ValidationError::new("email", "Invalid email format")
            .rule("email")
            .value("not-an-email");

        assert_eq!(err.field, "email");
        assert_eq!(err.message, "Invalid email format");
        assert_eq!(err.rule, Some("email".to_string()));
    }

    #[test]
    fn test_error_response_with_validation_errors() {
        let response = ErrorResponse::new(422)
            .message("Validation failed")
            .with_validation_error(ValidationError::new("name", "Required"))
            .with_validation_error(ValidationError::new("email", "Invalid"));

        assert_eq!(response.validation_errors.len(), 2);
    }

    #[test]
    fn test_error_response_to_json() {
        let response = ErrorResponse::new(404).message("Not found");
        let json = response.to_json();
        assert!(json.contains("404"));
        assert!(json.contains("Not found"));
    }

    #[test]
    fn test_error_response_to_plain_text() {
        let response = ErrorResponse::new(404).message("Resource not found");
        let text = response.to_plain_text();
        assert!(text.contains("404"));
        assert!(text.contains("Resource not found"));
    }

    #[test]
    fn test_error_response_to_html() {
        let response = ErrorResponse::new(500).message("Server error");
        let html = response.to_html();
        assert!(html.contains("500"));
        assert!(html.contains("Server error"));
        assert!(html.contains("<html>"));
    }

    #[test]
    fn test_error_transformer_default() {
        let transformer = ErrorTransformer::new();
        let error = Error::NotFound("User not found".to_string());
        let request = HttpRequest::new("GET", "/users/123".to_string());

        let response = transformer.transform(&error, &request);
        assert_eq!(response.status, 404);
    }

    #[test]
    fn test_error_transformer_production_mode() {
        let transformer = ErrorTransformer::new().production_mode(true);
        let error = Error::Internal("Database connection failed".to_string());
        let request = HttpRequest::new("GET", "/api".to_string());

        let response = transformer.transform(&error, &request);
        assert_eq!(response.status, 500);
        // In production mode, internal errors should hide details
        let body = String::from_utf8(response.body.to_vec()).unwrap();
        assert!(body.contains("internal server error"));
    }

    #[test]
    fn test_error_transformer_with_custom_transformer() {
        let transformer = ErrorTransformer::new().with_transformer(|error, _ctx| {
            if matches!(error, Error::NotFound(_)) {
                Some(ErrorResponse::new(404).message("Custom not found message"))
            } else {
                None
            }
        });

        let error = Error::NotFound("test".to_string());
        let request = HttpRequest::new("GET", "/test".to_string());

        let response = transformer.transform(&error, &request);
        let body = String::from_utf8(response.body.to_vec()).unwrap();
        assert!(body.contains("Custom not found message"));
    }

    #[test]
    fn test_sensitive_data_filtering() {
        let transformer = ErrorTransformer::new().filter_sensitive_data(true);
        let error = Error::BadRequest("Invalid password=secret123 in request".to_string());
        let request = HttpRequest::new("POST", "/login".to_string());

        let response = transformer.transform(&error, &request);
        let body = String::from_utf8(response.body.to_vec()).unwrap();
        assert!(!body.contains("secret123"));
        assert!(body.contains("[FILTERED]"));
    }

    #[test]
    fn test_error_context() {
        let mut request = HttpRequest::new("GET", "/api".to_string());
        request
            .headers
            .insert("X-Request-Id", "req-456".to_string());

        let context = ErrorContext::from_request(request)
            .user_id("user-123")
            .with_data("operation", "fetch_users");

        assert_eq!(context.request_id, Some("req-456".to_string()));
        assert_eq!(context.user_id, Some("user-123".to_string()));
    }

    #[test]
    fn test_problem_details_format() {
        let response = ErrorResponse::new(404)
            .message("User not found")
            .error_type("https://api.example.com/errors/not-found")
            .path("/users/123");

        let json = response.to_problem_details();
        assert!(json.contains("type"));
        assert!(json.contains("title"));
        assert!(json.contains("status"));
    }

    #[test]
    fn test_response_format_content_type() {
        let response = ErrorResponse::new(400).message("Bad request");

        let http_response = response.clone().into_http_response(ResponseFormat::Json);
        assert_eq!(
            http_response.headers.get("Content-Type"),
            Some(&"application/json".to_string())
        );

        let http_response = response
            .clone()
            .into_http_response(ResponseFormat::PlainText);
        assert_eq!(
            http_response.headers.get("Content-Type"),
            Some(&"text/plain; charset=utf-8".to_string())
        );

        let http_response = response
            .clone()
            .into_http_response(ResponseFormat::ProblemDetails);
        assert_eq!(
            http_response.headers.get("Content-Type"),
            Some(&"application/problem+json".to_string())
        );

        let http_response = response.clone().into_http_response(ResponseFormat::JsonApi);
        assert_eq!(
            http_response.headers.get("Content-Type"),
            Some(&"application/vnd.api+json".to_string())
        );

        let http_response = response.clone().into_http_response(ResponseFormat::Aws);
        assert_eq!(
            http_response.headers.get("Content-Type"),
            Some(&"application/x-amz-json-1.1".to_string())
        );
    }

    #[test]
    fn test_json_api_format() {
        let response = ErrorResponse::new(422)
            .message("Validation failed")
            .code("VALIDATION_ERROR")
            .with_validation_error(ValidationError::new("email", "Invalid format"));

        let json = response.to_json_api();
        assert!(json.contains("errors"));
        assert!(json.contains("422"));
        assert!(json.contains("email"));
    }

    #[test]
    fn test_graphql_format() {
        let response = ErrorResponse::new(400)
            .message("Bad request")
            .code("BAD_REQUEST")
            .path("/graphql");

        let json = response.to_graphql();
        assert!(json.contains("errors"));
        assert!(json.contains("Bad request"));
        assert!(json.contains("extensions"));
    }

    #[test]
    fn test_google_format() {
        let response = ErrorResponse::new(400)
            .message("Invalid argument")
            .with_validation_error(ValidationError::new("name", "Required"));

        let json = response.to_google();
        assert!(json.contains("error"));
        assert!(json.contains("INVALID_ARGUMENT"));
        assert!(json.contains("400"));
    }

    #[test]
    fn test_aws_format() {
        let response = ErrorResponse::new(400)
            .message("Validation error")
            .request_id("req-123");

        let json = response.to_aws();
        assert!(json.contains("__type"));
        assert!(json.contains("ValidationException"));
        assert!(json.contains("req-123"));
    }

    #[test]
    fn test_azure_format() {
        let response = ErrorResponse::new(400)
            .message("Bad request")
            .code("InvalidInput")
            .path("/api/users")
            .with_validation_error(ValidationError::new("email", "Invalid"));

        let json = response.to_azure();
        assert!(json.contains("error"));
        assert!(json.contains("InvalidInput"));
        assert!(json.contains("details"));
    }

    #[test]
    fn test_error_code_mapping() {
        let transformer = ErrorTransformer::new()
            .map_error_code("NOT_FOUND", "ERR_404")
            .map_error_code("VALIDATION_ERROR", "ERR_422");

        assert!(transformer.error_codes.contains_key("NOT_FOUND"));
        assert_eq!(
            transformer.error_codes.get("NOT_FOUND"),
            Some(&"ERR_404".to_string())
        );
    }

    #[test]
    fn test_preset_development() {
        let transformer = ErrorTransformer::development();
        assert!(!transformer.production_mode);
        assert!(transformer.include_stack_trace);
    }

    #[test]
    fn test_preset_production() {
        let transformer = ErrorTransformer::production();
        assert!(transformer.production_mode);
        assert!(!transformer.include_stack_trace);
        assert!(transformer.filter_sensitive);
    }

    #[test]
    fn test_preset_api() {
        let transformer = ErrorTransformer::api();
        assert_eq!(transformer.format, ResponseFormat::ProblemDetails);
    }

    #[test]
    fn test_transform_preserves_status_of_unlisted_variants() {
        let transformer = ErrorTransformer::new();
        let request = HttpRequest::new("GET", "/test");

        let response = transformer.transform(&Error::TooManyRequests("slow down".into()), &request);
        assert_eq!(response.status, 429);

        let response = transformer.transform(&Error::Conflict("dup".into()), &request);
        assert_eq!(response.status, 409);

        let response = transformer.transform(&Error::ServiceUnavailable("down".into()), &request);
        assert_eq!(response.status, 503);
    }

    #[test]
    fn test_user_registered_filters_are_applied() {
        let transformer =
            ErrorTransformer::new().with_filter(|_| Error::NotFound("filtered by user".into()));
        let request = HttpRequest::new("GET", "/test");

        let response = transformer.transform(&Error::Internal("original".into()), &request);
        assert_eq!(response.status, 404);
        let body = String::from_utf8(response.into_body_bytes().to_vec()).unwrap();
        assert!(body.contains("filtered by user"));
    }

    #[test]
    fn test_to_html_escapes_message_and_details() {
        let response = ErrorResponse::new(400)
            .message("<script>alert(1)</script>")
            .details("a & b <img>");
        let html = response.to_html();
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("a &amp; b &lt;img&gt;"));
    }

    #[test]
    fn test_filter_sensitive_string_redacts_pii_and_secrets() {
        let transformer = ErrorTransformer::new().filter_sensitive_data(true);
        let input = "card 4111 1111 1111 1111 ssn 123-45-6789 \
                     email user@example.com token=abcdef123 password=hunter2 \
                     Bearer sk_live_secret";
        let filtered = transformer.filter_sensitive_string(input);

        // Card and SSN are redacted.
        assert!(filtered.contains("[CARD FILTERED]"));
        assert!(!filtered.contains("4111 1111 1111 1111"));
        assert!(filtered.contains("[SSN FILTERED]"));
        assert!(!filtered.contains("123-45-6789"));

        // Token / password / bearer secrets are redacted.
        assert!(!filtered.contains("abcdef123"));
        assert!(!filtered.contains("hunter2"));
        assert!(!filtered.contains("sk_live_secret"));
        assert!(filtered.contains("[FILTERED]"));

        // Non-secret email address is preserved (not part of the redaction set).
        assert!(filtered.contains("user@example.com"));
    }

    #[test]
    fn test_filter_sensitive_string_stable_across_calls() {
        // Exercises the once-compiled LazyLock regexes across repeated calls.
        let transformer = ErrorTransformer::new().filter_sensitive_data(true);
        let input = "password=secret123 token=deadbeef";
        let first = transformer.filter_sensitive_string(input);
        let second = transformer.filter_sensitive_string(input);
        assert_eq!(first, second);
        assert!(!first.contains("secret123"));
        assert!(!first.contains("deadbeef"));
    }
}
