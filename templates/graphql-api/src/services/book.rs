//! Book service

use armature::prelude::*;
use async_graphql::ID;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::schema::{Book, CreateBookInput};

/// Book service for managing books
///
/// `#[injectable]` makes this resolvable from the DI container, but nothing in
/// this template resolves it that way: `build_schema` in `main.rs` constructs
/// it directly and the schema owns it for the process lifetime. The attribute
/// is here so you can move it into the container without rewriting the type.
#[injectable]
#[derive(Clone)]
pub struct BookService {
    books: Arc<RwLock<HashMap<String, Book>>>,
}

impl BookService {
    pub fn new() -> Self {
        let mut books = HashMap::new();

        // Add some sample books
        let sample_books = vec![
            Book {
                id: ID::from("1"),
                title: "The Rust Programming Language".to_string(),
                description: Some("The official book on Rust".to_string()),
                isbn: Some("978-1718500440".to_string()),
                published_year: Some(2019),
                author_id: "1".to_string(),
                created_at: Utc::now(),
            },
            Book {
                id: ID::from("2"),
                title: "Zero To Production In Rust".to_string(),
                description: Some("Building real-world backend applications".to_string()),
                isbn: Some("978-1234567890".to_string()),
                published_year: Some(2022),
                author_id: "1".to_string(),
                created_at: Utc::now(),
            },
            Book {
                id: ID::from("3"),
                title: "GraphQL in Action".to_string(),
                description: Some("Learn GraphQL by building real projects".to_string()),
                isbn: Some("978-1617295683".to_string()),
                published_year: Some(2021),
                author_id: "2".to_string(),
                created_at: Utc::now(),
            },
        ];

        for book in sample_books {
            books.insert(book.id.to_string(), book);
        }

        Self {
            books: Arc::new(RwLock::new(books)),
        }
    }

    /// List all books
    pub fn list(&self) -> Vec<Book> {
        self.books.read().unwrap().values().cloned().collect()
    }

    /// Find book by ID
    pub fn find_by_id(&self, id: &ID) -> Option<Book> {
        self.books.read().unwrap().get(&id.to_string()).cloned()
    }

    /// Find books by author
    pub fn find_by_author(&self, author_id: &ID) -> Vec<Book> {
        self.books
            .read()
            .unwrap()
            .values()
            .filter(|b| b.author_id == author_id.to_string())
            .cloned()
            .collect()
    }

    /// Search books by title
    pub fn search(&self, query: &str) -> Vec<Book> {
        let query_lower = query.to_lowercase();
        self.books
            .read()
            .unwrap()
            .values()
            .filter(|b| b.title.to_lowercase().contains(&query_lower))
            .cloned()
            .collect()
    }

    /// Create a new book
    pub fn create(&self, input: CreateBookInput) -> Book {
        let id = uuid::Uuid::new_v4().to_string();

        let book = Book {
            id: ID::from(&id),
            title: input.title,
            description: input.description,
            isbn: input.isbn,
            published_year: input.published_year,
            author_id: input.author_id,
            created_at: Utc::now(),
        };

        self.books.write().unwrap().insert(id, book.clone());
        book
    }

    /// Update a book's title
    pub fn update_title(&self, id: &ID, title: String) -> Option<Book> {
        let mut books = self.books.write().unwrap();
        let book = books.get_mut(&id.to_string())?;
        book.title = title;
        Some(book.clone())
    }

    /// Delete a book
    pub fn delete(&self, id: &ID) -> bool {
        self.books
            .write()
            .unwrap()
            .remove(&id.to_string())
            .is_some()
    }
}

impl Default for BookService {
    fn default() -> Self {
        Self::new()
    }
}
