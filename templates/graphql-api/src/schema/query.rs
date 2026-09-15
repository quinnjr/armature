//! GraphQL Query resolvers

use async_graphql::{ID, Object, Result};

use crate::services::{BookService, UserService};

use super::types::{Book, User};

/// Root query object
pub struct QueryRoot {
    user_service: UserService,
    book_service: BookService,
}

impl QueryRoot {
    pub fn new(user_service: UserService, book_service: BookService) -> Self {
        Self {
            user_service,
            book_service,
        }
    }
}

#[Object]
impl QueryRoot {
    // =========================================================================
    // User Queries
    // =========================================================================

    /// Get all users
    async fn users(&self) -> Vec<User> {
        self.user_service.list()
    }

    /// Get a specific user by ID
    async fn user(&self, id: ID) -> Result<User> {
        self.user_service
            .find_by_id(&id)
            .ok_or_else(|| "User not found".into())
    }

    /// Get a user by email
    async fn user_by_email(&self, email: String) -> Result<User> {
        self.user_service
            .find_by_email(&email)
            .ok_or_else(|| "User not found".into())
    }

    // =========================================================================
    // Book Queries
    // =========================================================================

    /// Get all books
    async fn books(&self) -> Vec<Book> {
        self.book_service.list()
    }

    /// Get a specific book by ID
    async fn book(&self, id: ID) -> Result<Book> {
        self.book_service
            .find_by_id(&id)
            .ok_or_else(|| "Book not found".into())
    }

    /// Get books by author
    async fn books_by_author(&self, author_id: ID) -> Vec<Book> {
        self.book_service.find_by_author(&author_id)
    }

    /// Search books by title
    async fn search_books(&self, query: String) -> Vec<Book> {
        self.book_service.search(&query)
    }
}
