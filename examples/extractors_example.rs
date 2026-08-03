//! Example: Request Parameter Extractors
//!
//! This example demonstrates how to use the extractor types and macros
//! to cleanly extract data from HTTP requests.
//!
//! Run with: `cargo run --example extractors_example`

#![allow(dead_code, unused_imports)]

use armature::prelude::*;
use bytes::Bytes;
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

#[tokio::main]
async fn main() {
    println!("🔧 Armature Request Extractors Example");
    println!("======================================\n");

    demonstrate_body_extraction();
    demonstrate_query_extraction();
    demonstrate_path_extraction();
    demonstrate_header_extraction();
    demonstrate_combined_extraction();
    demonstrate_decorator_syntax().await;
}

fn demonstrate_body_extraction() {
    println!("1️⃣  Body Extraction");
    println!("-------------------");

    // Simulate a POST request with JSON body
    let mut request = HttpRequest::new("POST", "/users".to_string());
    request.body = Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "name": "Alice Smith",
            "email": "alice@example.com",
            "role": "admin"
        }))
        .unwrap(),
    );

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
    let mut request = HttpRequest::new("GET", "/users".to_string());
    request.push_query_param("page", "2");
    request.push_query_param("limit", "25");
    request.push_query_param("sort", "created_at");
    request.push_query_param("order", "desc");
    request.push_query_param("search", "alice");

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
    let mut request = HttpRequest::new("GET", "/users/123/posts/456".to_string());
    request.push_param("user_id", "123");
    request.push_param("post_id", "456");

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

    let mut request = HttpRequest::new("GET", "/api/protected".to_string());
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

    let mut request = HttpRequest::new("PUT", "/users/42".to_string());

    // Set path parameter
    request.push_param("id", "42");

    // Set query parameter
    request.push_query_param("notify", "true");

    // Set body
    request.body = Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "name": "Alice Updated",
            "email": "alice.new@example.com"
        }))
        .unwrap(),
    );

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
    println!("  • MethodExtractor - HTTP method helper");
    println!();
    println!("Helper macros:");
    println!("  • body!(req, Type)           - Extract JSON body");
    println!("  • query!(req, Type)          - Extract query params");
    println!("  • path!(req, \"name\", Type)   - Extract path param");
    println!("  • header!(req, \"name\")       - Extract header");
}

/// A real, compiled controller using parameter decorators.
///
/// This block is not illustrative text: `#[routes]` expands it into registered
/// handlers, and `demonstrate_decorator_syntax` below dispatches real requests
/// through a real `Router` and prints what came back. If the decorator codegen
/// broke, this example would stop compiling.
#[controller("/users")]
#[derive(Default, Clone)]
struct DecoratorUserController;

#[routes]
impl DecoratorUserController {
    /// `#[query]` fills a whole struct from the query string.
    #[get("/list")]
    async fn list(#[query] filters: Query<UserListQuery>) -> Result<HttpResponse, Error> {
        let page = filters.page.unwrap_or(1);
        let limit = filters.limit.unwrap_or(10);
        HttpResponse::ok().with_json(&UserList {
            users: vec![User {
                id: 1,
                name: "Alice".to_string(),
                email: "alice@example.com".to_string(),
                role: "admin".to_string(),
            }],
            total: 1,
            page,
            limit,
        })
    }

    /// `#[param("id")]` extracts and parses a single path parameter.
    #[get("/by-id/:id")]
    async fn get_one(#[param("id")] user_id: Path<u32>) -> Result<HttpResponse, Error> {
        HttpResponse::ok().with_json(&User {
            id: *user_id,
            name: "Alice".to_string(),
            email: "alice@example.com".to_string(),
            role: "admin".to_string(),
        })
    }

    /// Decorators compose: a JSON body and a header in one signature.
    #[post("/create")]
    async fn create(
        #[body] body: Body<CreateUserDto>,
        #[header("authorization")] auth: Header,
    ) -> Result<HttpResponse, Error> {
        HttpResponse::created().with_json(&serde_json::json!({
            "name": body.name,
            "email": body.email,
            "authorized_by": auth.value(),
        }))
    }

    /// Field-level extraction: individual query params, no DTO struct needed.
    ///
    /// `#[query("name")]` parses the value into the parameter's type via
    /// `FromStr` and is **required** - a missing parameter is a 400. Use
    /// `#[query]` with a struct of `Option<_>` fields (as `list` above does)
    /// when a parameter may be absent.
    #[get("/search/run")]
    async fn search(
        #[query("q")] term: String,
        #[query("page")] page: u32,
    ) -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok()
            .with_body(format!("searching for {term:?} on page {page}").into_bytes()))
    }
}

#[module(controllers: [DecoratorUserController])]
#[derive(Default)]
struct DecoratorModule;

/// Build a router from the macro-generated controller registration.
fn decorator_router() -> Router {
    let container = Container::new();
    let mut router = Router::new();
    let module = DecoratorModule;
    for reg in module.controllers() {
        let instance = (reg.factory)(&container).expect("controller factory");
        (reg.route_registrar)(&container, &mut router, instance).expect("route registrar");
    }
    router
}

async fn demonstrate_decorator_syntax() {
    println!("6️⃣  NestJS-Style Decorator Syntax");
    println!("----------------------------------");
    println!("   Parameters carry the extractor; the handler receives values.");
    println!("   Every response below came out of a real Router dispatch.");
    println!();

    let router = decorator_router();

    // #[query] - whole-struct extraction
    let resp = router
        .route(HttpRequest::new("GET", "/users/list?page=2&limit=5"))
        .await
        .expect("GET /users/list must dispatch");
    println!("   #[query] filters: Query<UserListQuery>");
    println!("     GET /users/list?page=2&limit=5 -> {}", show(&resp));

    // #[param("id")] - single path parameter, parsed to u32
    let resp = router
        .route(HttpRequest::new("GET", "/users/by-id/42"))
        .await
        .expect("GET /users/by-id/42 must dispatch");
    println!("   #[param(\"id\")] user_id: Path<u32>");
    println!("     GET /users/by-id/42 -> {}", show(&resp));

    // #[body] + #[header] together
    let mut req = HttpRequest::new("POST", "/users/create");
    req.headers.insert("content-type", "application/json");
    req.headers.insert("authorization", "Bearer token-123");
    req.body = Bytes::from_static(br#"{"name":"Bob","email":"bob@example.com"}"#);
    let resp = router
        .route(req)
        .await
        .expect("POST /users/create must dispatch");
    println!("   #[body] body: Body<CreateUserDto>, #[header(\"authorization\")] auth: Header");
    println!("     POST /users/create -> {}", show(&resp));

    // #[query("q")] - field-level extraction, no DTO
    let resp = router
        .route(HttpRequest::new(
            "GET",
            "/users/search/run?q=armature&page=3",
        ))
        .await
        .expect("GET /users/search/run must dispatch");
    println!("   #[query(\"q\")] term: String, #[query(\"page\")] page: u32");
    println!(
        "     GET /users/search/run?q=armature&page=3 -> {}",
        show(&resp)
    );

    println!();
    println!("   Available decorator attributes:");
    println!("     • #[body]              - Extract entire JSON body");
    println!("     • #[body(\"field\")]     - Extract specific field from body");
    println!("     • #[query]             - Extract all query parameters as a struct");
    println!("     • #[query(\"field\")]    - Extract single query parameter");
    println!("     • #[param(\"name\")]     - Extract single path parameter");
    println!("     • #[path(\"name\")]      - Alias for #[param]");
    println!("     • #[header(\"name\")]    - Extract single header");
    println!("     • #[headers]           - Extract all headers");
    println!("     • #[raw_body]          - Extract raw body bytes");
    println!();
}

/// Render a dispatched response as `status body` for the printouts above.
fn show(resp: &HttpResponse) -> String {
    format!(
        "{} {}",
        resp.status,
        String::from_utf8_lossy(resp.body_ref())
    )
}
