#![allow(
    dead_code,
    unused_imports,
    clippy::default_constructed_unit_structs,
    clippy::needless_borrow,
    clippy::unnecessary_lazy_evaluations
)]
// GraphQL API example with Armature

use armature::prelude::*;
use armature_graphql::{EmptySubscription, ID, Object, Result as GqlResult, Schema, SimpleObject};
use serde::{Deserialize, Serialize};

// ========== Domain Models ==========

#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
struct Book {
    id: ID,
    title: String,
    author: String,
    year: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
struct Author {
    id: ID,
    name: String,
    books: Vec<Book>,
}

// ========== Services ==========

#[injectable]
#[derive(Default, Clone)]
struct BookService;

impl BookService {
    fn get_all_books(&self) -> Vec<Book> {
        vec![
            Book {
                id: ID::from("1"),
                title: "The Rust Programming Language".to_string(),
                author: "Steve Klabnik".to_string(),
                year: 2018,
            },
            Book {
                id: ID::from("2"),
                title: "Programming Rust".to_string(),
                author: "Jim Blandy".to_string(),
                year: 2021,
            },
            Book {
                id: ID::from("3"),
                title: "Rust in Action".to_string(),
                author: "Tim McNamara".to_string(),
                year: 2021,
            },
        ]
    }

    fn get_book_by_id(&self, id: &str) -> Option<Book> {
        self.get_all_books()
            .into_iter()
            .find(|b| b.id.as_str() == id)
    }

    fn create_book(&self, title: String, author: String, year: i32) -> Book {
        Book {
            id: ID::from("4"),
            title,
            author,
            year,
        }
    }

    fn get_authors(&self) -> Vec<Author> {
        vec![
            Author {
                id: ID::from("1"),
                name: "Steve Klabnik".to_string(),
                books: vec![self.get_book_by_id("1").unwrap()],
            },
            Author {
                id: ID::from("2"),
                name: "Jim Blandy".to_string(),
                books: vec![self.get_book_by_id("2").unwrap()],
            },
        ]
    }
}

// ========== GraphQL Schema ==========

struct QueryRoot {
    book_service: BookService,
}

#[Object]
impl QueryRoot {
    /// Get all books
    async fn books(&self) -> Vec<Book> {
        self.book_service.get_all_books()
    }

    /// Get a book by ID
    async fn book(&self, id: ID) -> GqlResult<Book> {
        self.book_service
            .get_book_by_id(id.as_str())
            .ok_or_else(|| "Book not found".into())
    }

    /// Get all authors
    async fn authors(&self) -> Vec<Author> {
        self.book_service.get_authors()
    }

    /// Search books by title
    async fn search_books(&self, query: String) -> Vec<Book> {
        self.book_service
            .get_all_books()
            .into_iter()
            .filter(|b| b.title.to_lowercase().contains(&query.to_lowercase()))
            .collect()
    }
}

struct MutationRoot {
    book_service: BookService,
}

#[Object]
impl MutationRoot {
    /// Create a new book
    async fn create_book(&self, title: String, author: String, year: i32) -> Book {
        self.book_service.create_book(title, author, year)
    }

    /// Update a book (simplified)
    async fn update_book(&self, id: ID, title: String) -> GqlResult<Book> {
        let mut book = self
            .book_service
            .get_book_by_id(id.as_str())
            .ok_or_else(|| "Book not found")?;
        book.title = title;
        Ok(book)
    }

    /// Delete a book (returns success status)
    async fn delete_book(&self, id: ID) -> bool {
        self.book_service.get_book_by_id(id.as_str()).is_some()
    }
}

// ========== Controllers ==========

#[controller("/graphql")]
#[derive(Default, Clone)]
struct GraphQLController;

#[routes]
impl GraphQLController {
    #[post("")]
    async fn execute(req: HttpRequest) -> Result<HttpResponse, Error> {
        let book_service = BookService::default();

        let query = QueryRoot {
            book_service: book_service.clone(),
        };
        let mutation = MutationRoot {
            book_service: book_service.clone(),
        };

        let schema = Schema::build(query, mutation, EmptySubscription).finish();

        // Parse GraphQL request
        #[derive(Deserialize)]
        struct GraphQLRequest {
            query: String,
            #[serde(default)]
            variables: Option<serde_json::Value>,
            #[serde(default)]
            operation_name: Option<String>,
        }

        let gql_req: GraphQLRequest = req.json()?;

        // Build async-graphql request
        let mut request = async_graphql::Request::new(gql_req.query);
        if let Some(vars) = gql_req.variables {
            request = request.variables(async_graphql::Variables::from_json(vars));
        }
        if let Some(op_name) = gql_req.operation_name {
            request = request.operation_name(op_name);
        }

        // Execute query
        let response = schema.execute(request).await;

        // Convert to JSON response
        let json =
            serde_json::to_value(&response).map_err(|e| Error::Serialization(e.to_string()))?;

        HttpResponse::ok().with_json(&json)
    }

    #[get("/schema")]
    async fn get_schema() -> Result<HttpResponse, Error> {
        let book_service = BookService::default();
        let query = QueryRoot {
            book_service: book_service.clone(),
        };
        let mutation = MutationRoot { book_service };
        let schema = Schema::build(query, mutation, EmptySubscription).finish();
        let sdl = schema.sdl();

        Ok(HttpResponse::ok()
            .with_header("Content-Type".to_string(), "text/plain".to_string())
            .with_body(sdl.into_bytes()))
    }
}

#[controller("/playground")]
#[derive(Default, Clone)]
struct PlaygroundController;

#[routes]
impl PlaygroundController {
    #[get("")]
    async fn playground() -> Result<HttpResponse, Error> {
        let html = armature_graphql::graphiql_html("/graphql");
        Ok(HttpResponse::ok()
            .with_header("Content-Type".to_string(), "text/html".to_string())
            .with_body(html.into_bytes()))
    }
}

// ========== Module ==========

#[module(
    providers: [BookService],
    controllers: [GraphQLController, PlaygroundController]
)]
#[derive(Default, Clone)]
struct AppModule;

// ========== Application ==========

#[tokio::main]
async fn main() {
    println!("📚 Armature GraphQL API Example");
    println!("================================\n");

    println!("GraphQL endpoint: http://localhost:3007/graphql");
    println!("GraphQL playground: http://localhost:3007/playground");
    println!();
    println!("Example queries:");
    println!();
    println!("1. Get all books:");
    println!("   query {{ books {{ id title author year }} }}");
    println!();
    println!("2. Get book by ID:");
    println!("   query {{ book(id: \"1\") {{ id title author year }} }}");
    println!();
    println!("3. Search books:");
    println!("   query {{ searchBooks(query: \"Rust\") {{ id title }} }}");
    println!();
    println!("4. Create a book:");
    println!(
        "   mutation {{ createBook(title: \"New Book\", author: \"John Doe\", year: 2024) {{ id title }} }}"
    );
    println!();
    println!("5. Get all authors:");
    println!("   query {{ authors {{ id name books {{ title }} }} }}");
    println!();

    let app = Application::create::<AppModule>().await;

    if let Err(e) = app.listen(3007).await {
        eprintln!("Server error: {}", e);
    }
}
