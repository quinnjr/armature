//! GraphQL Mutation resolvers

use async_graphql::{ID, Object, Result};

use crate::services::{BookService, UserService};

use super::types::{Book, CreateBookInput, CreateUserInput, User};

/// Root mutation object
pub struct MutationRoot {
    user_service: UserService,
    book_service: BookService,
}

impl MutationRoot {
    pub fn new(user_service: UserService, book_service: BookService) -> Self {
        Self {
            user_service,
            book_service,
        }
    }
}

#[Object]
impl MutationRoot {
    // =========================================================================
    // User Mutations
    // =========================================================================

    /// Create a new user
    async fn create_user(&self, name: String, email: String) -> Result<User> {
        if self.user_service.find_by_email(&email).is_some() {
            return Err("Email already exists".into());
        }
        Ok(self.user_service.create(CreateUserInput {
            name,
            email,
            role: None,
        }))
    }

    /// Update a user's name
    async fn update_user_name(&self, id: ID, name: String) -> Result<User> {
        self.user_service
            .update_name(&id, name)
            .ok_or_else(|| "User not found".into())
    }

    /// Delete a user
    async fn delete_user(&self, id: ID) -> bool {
        self.user_service.delete(&id)
    }

    // =========================================================================
    // Book Mutations
    // =========================================================================

    /// Create a new book
    async fn create_book(&self, input: CreateBookInput) -> Result<Book> {
        // Verify author exists
        if self
            .user_service
            .find_by_id(&ID::from(&input.author_id))
            .is_none()
        {
            return Err("Author not found".into());
        }
        Ok(self.book_service.create(input))
    }

    /// Update a book's title
    async fn update_book_title(&self, id: ID, title: String) -> Result<Book> {
        self.book_service
            .update_title(&id, title)
            .ok_or_else(|| "Book not found".into())
    }

    /// Delete a book
    async fn delete_book(&self, id: ID) -> bool {
        self.book_service.delete(&id)
    }
}
