//! Data models

use armature::armature_validation::{self, IsEmail, MinLength, NotEmpty, Validate};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// =============================================================================
// User Models
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub name: String,
    pub role: UserRole,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Admin,
    User,
    Guest,
}

impl Default for UserRole {
    fn default() -> Self {
        Self::User
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserResponse {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub role: UserRole,
    pub created_at: DateTime<Utc>,
}

impl From<&User> for UserResponse {
    fn from(user: &User) -> Self {
        Self {
            id: user.id,
            email: user.email.clone(),
            name: user.name.clone(),
            role: user.role,
            created_at: user.created_at,
        }
    }
}

// =============================================================================
// Auth Models
// =============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub user: UserResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenClaims {
    pub sub: String, // User ID
    pub email: String,
    pub role: UserRole,
    pub exp: usize,
    pub iat: usize,
}

// =============================================================================
// API Response Models
// =============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<PaginationMeta>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Vec<ValidationError>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PaginationMeta {
    pub page: u32,
    pub per_page: u32,
    pub total: u64,
    pub total_pages: u32,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
            meta: None,
        }
    }

    pub fn success_with_meta(data: T, meta: PaginationMeta) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
            meta: Some(meta),
        }
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>) -> ApiResponse<()> {
        ApiResponse {
            success: false,
            data: None,
            error: Some(ApiError {
                code: code.into(),
                message: message.into(),
                details: None,
            }),
            meta: None,
        }
    }

    pub fn validation_error(errors: Vec<ValidationError>) -> ApiResponse<()> {
        ApiResponse {
            success: false,
            data: None,
            error: Some(ApiError {
                code: "VALIDATION_ERROR".to_string(),
                message: "Validation failed".to_string(),
                details: Some(errors),
            }),
            meta: None,
        }
    }
}

// =============================================================================
// Health Models
// =============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub timestamp: DateTime<Utc>,
    pub uptime_seconds: u64,
}

// =============================================================================
// Request Validation
// =============================================================================
//
// Wires `LoginRequest`/`RegisterRequest` into `armature-validation`'s
// `Validate` trait so controllers can reject malformed input with structured,
// field-level errors (see `ApiResponse::validation_error`).

impl Validate for LoginRequest {
    fn validate(&self) -> Result<(), Vec<armature_validation::ValidationError>> {
        let mut errors = Vec::new();

        if let Err(e) = NotEmpty::validate(&self.email, "email") {
            errors.push(e);
        }
        if let Err(e) = NotEmpty::validate(&self.password, "password") {
            errors.push(e);
        }

        if errors.is_empty() { Ok(()) } else { Err(errors) }
    }
}

impl Validate for RegisterRequest {
    fn validate(&self) -> Result<(), Vec<armature_validation::ValidationError>> {
        let mut errors = Vec::new();

        if let Err(e) = NotEmpty::validate(&self.email, "email") {
            errors.push(e);
        } else if let Err(e) = IsEmail::validate(&self.email, "email") {
            errors.push(e);
        }

        if let Err(e) = NotEmpty::validate(&self.password, "password") {
            errors.push(e);
        } else if let Err(e) = MinLength(8).validate(&self.password, "password") {
            errors.push(e);
        }

        if let Err(e) = NotEmpty::validate(&self.name, "name") {
            errors.push(e);
        }

        if errors.is_empty() { Ok(()) } else { Err(errors) }
    }
}

/// Adapts an `armature-validation` error into this API's own serializable
/// [`ValidationError`] shape.
impl From<armature_validation::ValidationError> for ValidationError {
    fn from(e: armature_validation::ValidationError) -> Self {
        Self {
            field: e.field,
            message: e.message,
        }
    }
}

