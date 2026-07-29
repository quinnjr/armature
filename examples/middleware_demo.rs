// Comprehensive Middleware System Example
//
// Middleware is attached to routes using the framework's real mechanism for
// doing so, the `#[middleware(...)]` / `#[use_middleware(...)]` attribute
// macros - *not* by building a `MiddlewareChain` by hand, which has no effect
// on the running server since `Application` has no API to accept one
// directly.
//
//   - `#[middleware(...)]` on the `ApiController` struct applies request ID,
//     logging, CORS, security headers, and compression to every route on the
//     controller.
//   - `#[use_middleware(...)]` on an individual handler layers additional,
//     route-specific middleware on top: API key auth + rate limiting on
//     `/api/protected`, a 5s request timeout on `/api/slow`, and a 1MB body
//     size limit on `/api/upload`.

use armature::prelude::*;
use armature::{
    BodySizeLimitMiddleware, CompressionMiddleware, CorsMiddleware, Error, HttpResponse,
    LoggerMiddleware, Middleware, RequestIdMiddleware, SecurityHeadersMiddleware,
    TimeoutMiddleware,
};
use armature_proc_macro::{middleware, use_middleware};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

// ========== Custom Middleware ==========

/// API Key validation middleware
struct ApiKeyMiddleware {
    valid_keys: Vec<String>,
}

impl ApiKeyMiddleware {
    fn new(keys: Vec<String>) -> Self {
        Self { valid_keys: keys }
    }
}

#[async_trait::async_trait]
impl Middleware for ApiKeyMiddleware {
    async fn handle(
        &self,
        req: HttpRequest,
        next: Box<
            dyn FnOnce(
                    HttpRequest,
                )
                    -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>
                + Send,
        >,
    ) -> Result<HttpResponse, Error> {
        if let Some(api_key) = req.headers.get("x-api-key")
            && self.valid_keys.contains(api_key)
        {
            println!("✓ Valid API key: {}", api_key);
            return next(req).await;
        }
        Err(Error::Unauthorized(
            "Invalid or missing API key".to_string(),
        ))
    }
}

/// Rate limiting middleware (simple in-memory version)
struct RateLimitMiddleware {
    requests_per_minute: u32,
}

impl RateLimitMiddleware {
    fn new(limit: u32) -> Self {
        Self {
            requests_per_minute: limit,
        }
    }
}

#[async_trait::async_trait]
impl Middleware for RateLimitMiddleware {
    async fn handle(
        &self,
        req: HttpRequest,
        next: Box<
            dyn FnOnce(
                    HttpRequest,
                )
                    -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>
                + Send,
        >,
    ) -> Result<HttpResponse, Error> {
        let client_ip = req
            .headers
            .get("x-forwarded-for")
            .or_else(|| req.headers.get("x-real-ip"))
            .map(|s| s.as_str())
            .unwrap_or("unknown");

        println!(
            "Rate limit check for IP: {} (limit: {}/min)",
            client_ip, self.requests_per_minute
        );
        next(req).await
    }
}

/// Request timing middleware
struct TimingMiddleware;

#[async_trait::async_trait]
impl Middleware for TimingMiddleware {
    async fn handle(
        &self,
        req: HttpRequest,
        next: Box<
            dyn FnOnce(
                    HttpRequest,
                )
                    -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>
                + Send,
        >,
    ) -> Result<HttpResponse, Error> {
        let start = std::time::Instant::now();
        let result = next(req).await;
        let duration = start.elapsed();
        println!("⏱  Request completed in {:?}", duration);
        result
    }
}

// ========== DTOs ==========

#[derive(Debug, Serialize, Deserialize)]
struct ApiResponse {
    message: String,
    data: Option<serde_json::Value>,
}

// ========== Services ==========

#[injectable]
#[derive(Clone, Default)]
struct DataService;

impl DataService {
    fn get_data(&self) -> ApiResponse {
        ApiResponse {
            message: "Data retrieved successfully".to_string(),
            data: Some(serde_json::json!({
                "items": ["item1", "item2", "item3"]
            })),
        }
    }
}

// ========== Controllers ==========

#[controller("/api")]
#[middleware(
    RequestIdMiddleware,
    LoggerMiddleware::new(),
    CorsMiddleware::new()
        .allow_origin("*")
        .allow_credentials(false),
    SecurityHeadersMiddleware::new(),
    CompressionMiddleware::new()
)]
#[derive(Default, Clone)]
struct ApiController;

#[routes]
impl ApiController {
    #[get("/public")]
    async fn get_public_data() -> Result<HttpResponse, Error> {
        HttpResponse::json(&ApiResponse {
            message: "Public data - controller-wide middleware only, no API key required"
                .to_string(),
            data: None,
        })
    }

    #[use_middleware(
        ApiKeyMiddleware::new(vec![
            "secret-key-123".to_string(),
            "admin-key-456".to_string(),
        ]),
        RateLimitMiddleware::new(60),
        TimingMiddleware
    )]
    #[get("/protected")]
    async fn get_protected_data(_req: HttpRequest) -> Result<HttpResponse, Error> {
        let service = DataService;
        HttpResponse::json(&service.get_data())
    }

    #[use_middleware(TimeoutMiddleware::new(5))]
    #[get("/slow")]
    async fn get_slow_data(_req: HttpRequest) -> Result<HttpResponse, Error> {
        // Sleeps longer than the 5s `TimeoutMiddleware` configured above, so
        // the middleware aborts the request with a 408 before this ever
        // returns a response.
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        Ok(HttpResponse::ok().with_body(b"This should never be reached".to_vec()))
    }

    #[use_middleware(BodySizeLimitMiddleware::new(1024 * 1024))]
    #[post("/upload")]
    async fn upload(req: HttpRequest) -> Result<HttpResponse, Error> {
        let size = req.body.len();
        Ok(HttpResponse::ok().with_body(format!("Uploaded {} bytes", size).into_bytes()))
    }

    // An explicit OPTIONS route so a preflight request actually matches and
    // reaches the controller-wide `CorsMiddleware`, which short-circuits
    // OPTIONS requests with a 204 response before this body ever runs.
    #[options("/protected")]
    async fn protected_preflight() -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }
}

// ========== Module ==========

#[module(
    providers: [DataService],
    controllers: [ApiController]
)]
#[derive(Default, Clone)]
struct AppModule;

#[tokio::main]
async fn main() {
    println!("🔧 Armature Middleware System Demo");
    println!("===================================\n");

    // Middleware is actually wired up via `#[middleware(...)]` on
    // `ApiController` and `#[use_middleware(...)]` on individual handlers
    // above - this is just printing what is really in effect.
    println!("Controller-wide middleware (every /api/* route):");
    println!("  1. Request ID - Assigns unique ID to each request");
    println!("  2. Logger - Logs request/response details");
    println!("  3. CORS - Handles cross-origin requests");
    println!("  4. Security Headers - Adds security headers");
    println!("  5. Compression - Compresses large responses");
    println!();
    println!("Route-specific middleware:");
    println!("  /api/protected - API Key auth, rate limit (60/min), timing");
    println!("  /api/slow      - Request timeout (5s)");
    println!("  /api/upload    - Body size limit (max 1MB)");
    println!();

    println!("Server running on http://localhost:3014");
    println!();
    println!("API Endpoints:");
    println!();
    println!("1. Public (controller-wide middleware only, no API key):");
    println!("   curl http://localhost:3014/api/public");
    println!();
    println!("2. Protected (requires API key - omit it to see a 401):");
    println!("   curl -i http://localhost:3014/api/protected");
    println!("   curl -i http://localhost:3014/api/protected \\");
    println!("     -H \"x-api-key: secret-key-123\"");
    println!();
    println!("3. CORS preflight:");
    println!("   curl -X OPTIONS http://localhost:3014/api/protected");
    println!();
    println!("4. Test timeout (handler sleeps 10s, middleware times out at 5s):");
    println!("   curl -i http://localhost:3014/api/slow");
    println!();
    println!("5. Test body size limit:");
    println!("   curl -X POST http://localhost:3014/api/upload \\");
    println!("     -d \"$(head -c 2000 </dev/urandom | base64)\"");
    println!();

    let app = Application::create::<AppModule>().await;

    if let Err(e) = app.listen(3014).await {
        eprintln!("Server error: {}", e);
    }
}
