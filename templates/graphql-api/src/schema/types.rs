//! GraphQL types

use async_graphql::{Enum, ID, InputObject, SimpleObject};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// =============================================================================
// User Types
// =============================================================================

/// User object type
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
pub struct User {
    pub id: ID,
    pub name: String,
    pub email: String,
    pub role: UserRole,
    pub created_at: DateTime<Utc>,
}

/// User role enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Enum)]
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

// =============================================================================
// Book Types
// =============================================================================

/// Book object type
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
pub struct Book {
    pub id: ID,
    pub title: String,
    pub description: Option<String>,
    pub isbn: Option<String>,
    pub published_year: Option<i32>,
    pub author_id: String,
    pub created_at: DateTime<Utc>,
}

// =============================================================================
// Input Types
// =============================================================================

/// Input for creating a user
#[derive(Debug, InputObject)]
pub struct CreateUserInput {
    pub name: String,
    pub email: String,
    #[graphql(default)]
    pub role: Option<UserRole>,
}

/// Input for creating a book
#[derive(Debug, InputObject)]
pub struct CreateBookInput {
    pub title: String,
    pub description: Option<String>,
    pub isbn: Option<String>,
    pub published_year: Option<i32>,
    pub author_id: String,
}
