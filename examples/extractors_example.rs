//! Example: Request Parameter Extractors
//!
//! This example demonstrates how to use the extractor types and macros
//! to cleanly extract data from HTTP requests.
//!
//! Run with: `cargo run --example extractors_example`

#![allow(dead_code, unused_imports)]

use armature::prelude::*;
use serde::{Deserialize, Serialize};

// ========== Data Transfer Objects ==========

/// Request body for creating a user
#[derive(Debug, Deserialize, Serialize)]
struct CreateUserDto {
    name: String,
    email: String,
    role: Option<String>,
}

/// Request body for updating a user
#[derive(Debug, Deserialize, Serialize)]
struct UpdateUserDto {
    name: Option<String>,
    email: Option<String>,
    role: Option<String>,
}

/// Query parameters for listing users
#[derive(Debug, Deserialize, Default)]
struct UserListQuery {
    page: Option<u32>,
    limit: Option<u32>,
    sort: Option<String>,
    order: Option<String>,
    search: Option<String>,
}

/// Path parameters for user-post routes
#[derive(Debug, Deserialize)]
struct UserPostParams {
    user_id: u32,
    post_id: u32,
}

// ========== Response types ==========

#[derive(Debug, Serialize)]
struct User {
    id: u32,
    name: String,
    email: String,
    role: String,
}

#[derive(Debug, Serialize)]
struct UserList {
    users: Vec<User>,
    total: u32,
    page: u32,
    limit: u32,
}

fn main() {
    println!("🔧 Armature Request Extractors Example");
    println!("======================================\n");

    demonstrate_body_extraction();
    demonstrate_query_extraction();
    demonstrate_path_extraction();
    demonstrate_header_extraction();
    demonstrate_combined_extraction();
    demonstrate_decorator_syntax();
}

fn demonstrate_body_extraction() {
    println!("1️⃣  Body Extraction");
    println!("-------------------");

    // Simulate a POST request with JSON body
    let mut request = HttpRequest::new("POST".to_string(), "/users".to_string());
    request.body = serde_json::to_vec(&serde_json::json!({
        "name": "Alice Smith",
        "email": "alice@example.com",
        "role": "admin"
    }))
    .unwrap();

    // Method 1: Using the Body extractor type
    let body: Body<CreateUserDto> = Body::from_request(&request).unwrap();
    println!("   Extracted body using Body<T>:");
    println!("      Name:  {}", body.name);
    println!("      Email: {}", body.email);
    println!("      Role:  {:?}", body.role);

    // Method 2: Using the body! macro
    let dto = body!(request, CreateUserDto).unwrap();
    println!("   Extracted body using body! macro:");
    println!("      Name:  {}", dto.name);
    println!();
}

fn demonstrate_query_extraction() {
    println!("2️⃣  Query Parameter Extraction");
    println!("-------------------------------");

    // Simulate a GET request with query parameters
    let mut request = HttpRequest::new("GET".to_string(), "/users".to_string());
    request
        .query_params
        .insert("page".to_string(), "2".to_string());
    request
        .query_params
        .insert("limit".to_string(), "25".to_string());
    request
        .query_params
        .insert("sort".to_string(), "created_at".to_string());
    request
        .query_params
        .insert("order".to_string(), "desc".to_string());
    request
        .query_params
        .insert("search".to_string(), "alice".to_string());

    // Method 1: Using the Query extractor type
    let query: Query<UserListQuery> = Query::from_request(&request).unwrap();
    println!("   Extracted query using Query<T>:");
    println!("      Page:   {:?}", query.page);
    println!("      Limit:  {:?}", query.limit);
    println!("      Sort:   {:?}", query.sort);
    println!("      Order:  {:?}", query.order);
    println!("      Search: {:?}", query.search);

    // Method 2: Using the query! macro
    let filters = query!(request, UserListQuery).unwrap();
    println!("   Extracted query using query! macro:");
    println!("      Page: {:?}", filters.page);
    println!();
}

fn demonstrate_path_extraction() {
    println!("3️⃣  Path Parameter Extraction");
    println!("------------------------------");

    // Simulate a GET request with path parameters
    let mut request = HttpRequest::new("GET".to_string(), "/users/123/posts/456".to_string());
    request
        .path_params
        .insert("user_id".to_string(), "123".to_string());
    request
        .path_params
        .insert("post_id".to_string(), "456".to_string());

    // Method 1: Extract single path parameter with Path<T>
    let user_id: Path<u32> = Path::from_request(&request, "user_id").unwrap();
    let post_id: Path<u32> = Path::from_request(&request, "post_id").unwrap();
    println!("   Extracted path params using Path<T>:");
    println!("      User ID: {}", *user_id);
    println!("      Post ID: {}", *post_id);

    // Method 2: Using the path! macro
    let uid = path!(request, "user_id", u32).unwrap();
    let pid = path!(request, "post_id", u32).unwrap();
    println!("   Extracted path params using path! macro:");
    println!("      User ID: {}", uid);
    println!("      Post ID: {}", pid);

    // Method 3: Extract all path params into a struct
    let params: PathParams<UserPostParams> = PathParams::from_request(&request).unwrap();
    println!("   Extracted all path params using PathParams<T>:");
    println!("      User ID: {}", params.user_id);
    println!("      Post ID: {}", params.post_id);
    println!();
}

fn demonstrate_header_extraction() {
    println!("4️⃣  Header Extraction");
    println!("---------------------");

    let mut request = HttpRequest::new("GET".to_string(), "/api/protected".to_string());
    request.headers.insert(
        "Authorization",
        "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9".to_string(),
    );
    request
        .headers
        .insert("X-Request-ID", "req-12345".to_string());
    request
        .headers
        .insert("Content-Type", "application/json".to_string());

    // Method 1: Extract single header
    let auth: Header = Header::from_request(&request, "Authorization").unwrap();
    println!("   Extracted header using Header:");
    println!("      Name:  {}", auth.name());
    println!("      Value: {}", auth.value());

    // Method 2: Using header! macro
    let request_id = header!(request, "X-Request-ID").unwrap();
    println!("   Extracted header using header! macro:");
    println!("      X-Request-ID: {}", request_id);

    // Method 3: Optional header (doesn't error if missing)
    let custom = Header::optional(&request, "X-Custom-Header");
    println!("   Optional header (X-Custom-Header): {:?}", custom);

    // Method 4: Extract all headers
    let headers: Headers = Headers::from_request(&request).unwrap();
    println!("   All headers:");
    for (name, value) in headers.iter() {
        println!("      {}: {}", name, value);
    }

    // Method 5: ContentType helper
    let ct: ContentType = ContentType::from_request(&request).unwrap();
    println!("   Content-Type helpers:");
    println!("      Is JSON: {}", ct.is_json());
    println!("      Is Form: {}", ct.is_form());
    println!();
}

fn demonstrate_combined_extraction() {
    println!("5️⃣  Combined Extraction Example");
    println!("--------------------------------");
    println!("   Simulating a PUT /users/:id request with body and query...");

    let mut request = HttpRequest::new("PUT".to_string(), "/users/42".to_string());

    // Set path parameter
    request
        .path_params
        .insert("id".to_string(), "42".to_string());

    // Set query parameter
    request
        .query_params
        .insert("notify".to_string(), "true".to_string());

    // Set body
    request.body = serde_json::to_vec(&serde_json::json!({
        "name": "Alice Updated",
        "email": "alice.new@example.com"
    }))
    .unwrap();

    // Set headers
    request
        .headers
        .insert("Authorization", "Bearer token123".to_string());
    request
        .headers
        .insert("Content-Type", "application/json".to_string());

    // Extract everything in a handler-like function
    fn handle_update(req: &HttpRequest) -> Result<(), Error> {
        // Extract path parameter
        let user_id = path!(req, "id", u32)?;

        // Extract body
        let update_data = body!(req, UpdateUserDto)?;

        // Extract optional header
        let auth = Header::optional(req, "Authorization");

        println!("   Handler extracted:");
        println!("      User ID: {}", user_id);
        println!("      Update:  {:?}", update_data);
        println!("      Auth:    {:?}", auth.map(|h| h.into_value()));

        Ok(())
    }

    handle_update(&request).unwrap();
    println!();

    println!("✅ Extractors ready for use!");
    println!();
    println!("Available extractors:");
    println!("  • Body<T>      - JSON request body");
    println!("  • Query<T>     - URL query parameters");
    println!("  • Path<T>      - Single path parameter");
    println!("  • PathParams<T> - All path parameters as struct");
    println!("  • Header       - Single header value");
    println!("  • Headers      - All headers");
    println!("  • RawBody      - Raw request body bytes");
    println!("  • Form<T>      - URL-encoded form data");
    println!("  • ContentType  - Content-Type header helper");
    println!("  • Method       - HTTP method helper");
    println!();
    println!("Helper macros:");
    println!("  • body!(req, Type)           - Extract JSON body");
    println!("  • query!(req, Type)          - Extract query params");
    println!("  • path!(req, \"name\", Type)   - Extract path param");
    println!("  • header!(req, \"name\")       - Extract header");
}

fn demonstrate_decorator_syntax() {
    println!("6️⃣  NestJS-Style Decorator Syntax");
    println!("----------------------------------");
    println!("   Armature supports NestJS-style parameter decorators!");
    println!();
    println!("   Instead of manually extracting parameters:");
    println!();
    println!("     #[post(\"/users\")]");
    println!("     async fn create_user(req: HttpRequest) -> Result<HttpResponse, Error> {{");
    println!("         let body: Body<CreateUser> = Body::from_request(&req)?;");
    println!("         let auth = header!(req, \"Authorization\")?;");
    println!("         // ...");
    println!("     }}");
    println!();
    println!("   You can use decorator attributes directly on parameters:");
    println!();
    println!("     #[post(\"/users\")]");
    println!("     async fn create_user(");
    println!("         #[body] body: Body<CreateUser>,");
    println!("         #[header(\"Authorization\")] auth: Header,");
    println!("     ) -> Result<HttpResponse, Error> {{");
    println!("         // body and auth are automatically extracted!");
    println!("         // ...");
    println!("     }}");
    println!();
    println!("   Available decorator attributes:");
    println!("     • #[body]              - Extract entire JSON body");
    println!("     • #[body(\"field\")]     - Extract specific field from body");
    println!("     • #[query]             - Extract all query parameters struct");
    println!("     • #[query(\"field\")]    - Extract single query parameter");
    println!("     • #[param(\"name\")]     - Extract single path parameter");
    println!("     • #[path(\"name\")]      - Alias for #[param]");
    println!("     • #[header(\"name\")]    - Extract single header");
    println!("     • #[headers]           - Extract all headers");
    println!("     • #[raw_body]          - Extract raw body bytes");
    println!();
    println!("   ✨ Field-level extraction (NestJS-like):");
    println!();
    println!("     // Extract specific fields - no need for a full DTO struct!");
    println!("     #[post(\"/users\")]");
    println!("     async fn create_user(");
    println!("         #[body(\"name\")] name: String,");
    println!("         #[body(\"email\")] email: String,");
    println!("         #[body(\"age\")] age: u32,");
    println!("     ) -> Result<HttpResponse, Error> {{");
    println!("         println!(\"Creating user: {{}} ({{}})\", name, email);");
    println!("         // ...");
    println!("     }}");
    println!();
    println!("     // Extract specific query params");
    println!("     #[get(\"/search\")]");
    println!("     async fn search(");
    println!("         #[query(\"q\")] search_term: String,");
    println!("         #[query(\"page\")] page: Option<u32>,");
    println!("     ) -> Result<HttpResponse, Error> {{");
    println!("         // search_term and page are extracted directly!");
    println!("         // ...");
    println!("     }}");
    println!();
    println!("   Example controller:");
    println!();
    println!("     #[controller(\"/users\")]");
    println!("     struct UserController;");
    println!();
    println!("     impl UserController {{");
    println!("         #[get(\"\")]");
    println!("         async fn list(");
    println!("             #[query] filters: Query<UserFilters>,");
    println!("         ) -> Result<HttpResponse, Error> {{");
    println!("             // filters.page, filters.limit, etc.");
    println!("             HttpResponse::ok().with_json(&UserList {{ ... }})");
    println!("         }}");
    println!();
    println!("         #[post(\"\")]");
    println!("         async fn create(");
    println!("             #[body] body: Body<CreateUserDto>,");
    println!("             #[header(\"Authorization\")] auth: Header,");
    println!("         ) -> Result<HttpResponse, Error> {{");
    println!("             // body.name, body.email, auth.value()");
    println!("             HttpResponse::created().with_json(&User {{ ... }})");
    println!("         }}");
    println!();
    println!("         #[get(\"/:id\")]");
    println!("         async fn get_one(");
    println!("             #[param(\"id\")] user_id: Path<u32>,");
    println!("         ) -> Result<HttpResponse, Error> {{");
    println!("             // *user_id = 123");
    println!("             HttpResponse::ok().with_json(&User {{ ... }})");
    println!("         }}");
    println!();
    println!("         #[put(\"/:id\")]");
    println!("         async fn update(");
    println!("             #[param(\"id\")] user_id: Path<u32>,");
    println!("             #[body] update: Body<UpdateUserDto>,");
    println!("             #[query] options: Query<UpdateOptions>,");
    println!("         ) -> Result<HttpResponse, Error> {{");
    println!("             // All three parameters extracted automatically!");
    println!("             HttpResponse::ok().with_json(&User {{ ... }})");
    println!("         }}");
    println!("     }}");
    println!();
}
