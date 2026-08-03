#![allow(dead_code)]
//! TechEmpower Framework Benchmark Server
//!
//! Implements the standard TechEmpower benchmark tests:
//! - JSON serialization: GET /json
//! - Plaintext: GET /plaintext
//! - DB single query: GET /db
//! - DB multiple queries: GET /queries?queries=N
//! - Fortunes: GET /fortunes
//! - DB updates: GET /updates?queries=N
//! - Cached queries: GET /cached-queries?queries=N
//!
//! # Running
//!
//! ```bash
//! cargo run --release --example techempower_server
//! ```
//!
//! # Benchmarking
//!
//! ```bash
//! wrk -t4 -c256 -d15s http://127.0.0.1:8080/json
//! wrk -t4 -c256 -d15s http://127.0.0.1:8080/plaintext
//! ```

use armature::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ============================================================================
// Types
// ============================================================================

/// JSON response for /json endpoint
#[derive(Serialize)]
struct JsonMessage {
    message: &'static str,
}

/// World row from database
#[derive(Clone, Serialize, Deserialize)]
struct World {
    id: i32,
    #[serde(rename = "randomNumber")]
    random_number: i32,
}

/// Fortune row from database
#[derive(Clone, Serialize)]
struct Fortune {
    id: i32,
    message: String,
}

// ============================================================================
// In-Memory Database (for benchmarking without real DB)
// ============================================================================

/// Simple in-memory database for benchmarking without PostgreSQL
struct InMemoryDb {
    worlds: Vec<World>,
    fortunes: Vec<Fortune>,
}

impl InMemoryDb {
    fn new() -> Self {
        // Pre-populate with 10,000 "World" rows
        let worlds: Vec<World> = (1..=10000)
            .map(|id| World {
                id,
                random_number: fastrand::i32(1..=10000),
            })
            .collect();

        let fortunes = vec![
            Fortune { id: 1, message: "fortune: No such file or directory".to_string() },
            Fortune { id: 2, message: "A computer scientist is someone who fixes things that aren't broken.".to_string() },
            Fortune { id: 3, message: "After enough decimal places, nobody gives a damn.".to_string() },
            Fortune { id: 4, message: "A bad random number generator: 1, 1, 1, 1, 1, 4.33e+67, 1, 1, 1".to_string() },
            Fortune { id: 5, message: "A computer program does what you tell it to do, not what you want it to do.".to_string() },
            Fortune { id: 6, message: "Emstrongs's Law: If you make something idiot-proof, someone will make a better idiot.".to_string() },
            Fortune { id: 7, message: "Feature: A bug with seniority.".to_string() },
            Fortune { id: 8, message: "Computers make very fast, very accurate mistakes.".to_string() },
            Fortune { id: 9, message: "<script>alert(\"This should not be displayed in a alarm box.\");</script>".to_string() },
            Fortune { id: 10, message: "フレームワークのベンチマーク".to_string() },
            Fortune { id: 11, message: "Any program that runs right is obsolete.".to_string() },
            Fortune { id: 12, message: "Waste not, compute not.".to_string() },
        ];

        Self { worlds, fortunes }
    }

    fn get_random_world(&self) -> World {
        let id = fastrand::i32(1..=10000);
        let idx = ((id - 1) as usize) % self.worlds.len();
        self.worlds[idx].clone()
    }

    fn update_world(&self, world: &mut World) {
        world.random_number = fastrand::i32(1..=10000);
    }

    fn get_fortunes(&self) -> Vec<Fortune> {
        self.fortunes.clone()
    }
}

// ============================================================================
// Application State
// ============================================================================

struct AppState {
    db: InMemoryDb,
}

// Global state for benchmarking (in production, use DI)
static APP_STATE: std::sync::OnceLock<Arc<AppState>> = std::sync::OnceLock::new();

fn get_state() -> &'static Arc<AppState> {
    APP_STATE.get_or_init(|| {
        Arc::new(AppState {
            db: InMemoryDb::new(),
        })
    })
}

// ============================================================================
// Helpers
// ============================================================================

/// Parse the queries parameter (1-500, default 1)
fn parse_queries(queries: Option<&str>) -> usize {
    queries
        .and_then(|s| s.parse::<usize>().ok())
        .map(|n| n.clamp(1, 500))
        .unwrap_or(1)
}

/// Render fortunes as HTML
fn render_fortunes_html(fortunes: &[Fortune]) -> String {
    let mut html = String::with_capacity(2048);
    html.push_str("<!DOCTYPE html><html><head><title>Fortunes</title></head><body><table><tr><th>id</th><th>message</th></tr>");

    for fortune in fortunes {
        html.push_str("<tr><td>");
        html.push_str(&fortune.id.to_string());
        html.push_str("</td><td>");
        // HTML escape the message
        html.push_str(&html_escape(&fortune.message));
        html.push_str("</td></tr>");
    }

    html.push_str("</table></body></html>");
    html
}

/// Simple HTML escaping
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

// ============================================================================
// Controller
// ============================================================================

#[controller("")]
#[derive(Default, Clone)]
struct TechEmpowerController;

#[routes]
impl TechEmpowerController {
    /// GET /json - JSON serialization test
    ///
    /// Returns: {"message":"Hello, World!"}
    #[get("/json")]
    async fn json_handler() -> Result<HttpResponse, Error> {
        HttpResponse::json(&JsonMessage {
            message: "Hello, World!",
        })
    }

    /// GET /plaintext - Plaintext test
    ///
    /// Returns: Hello, World!
    #[get("/plaintext")]
    async fn plaintext_handler() -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok()
            .with_header("Content-Type".to_string(), "text/plain".to_string())
            .with_body("Hello, World!".as_bytes().to_vec()))
    }

    /// GET /db - Single database query test
    ///
    /// Fetches a single random row from the World table
    #[get("/db")]
    async fn db_handler() -> Result<HttpResponse, Error> {
        let state = get_state();
        let world = state.db.get_random_world();
        HttpResponse::json(&world)
    }

    /// GET /queries - Multiple database queries test
    ///
    /// Fetches N random rows from the World table (1-500, default 1)
    #[get("/queries")]
    async fn queries_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
        let state = get_state();
        let count = parse_queries(req.query_param("queries"));

        let worlds: Vec<World> = (0..count).map(|_| state.db.get_random_world()).collect();

        HttpResponse::json(&worlds)
    }

    /// GET /fortunes - Fortunes template test
    ///
    /// Fetches all Fortune rows, adds a new one, sorts by message, renders HTML
    #[get("/fortunes")]
    async fn fortunes_handler() -> Result<HttpResponse, Error> {
        let state = get_state();
        let mut fortunes = state.db.get_fortunes();

        // Add the additional fortune as per TFB spec
        fortunes.push(Fortune {
            id: 0,
            message: "Additional fortune added at request time.".to_string(),
        });

        // Sort by message
        fortunes.sort_by(|a, b| a.message.cmp(&b.message));

        // Render HTML
        let html = render_fortunes_html(&fortunes);

        Ok(HttpResponse::ok()
            .with_header(
                "Content-Type".to_string(),
                "text/html; charset=utf-8".to_string(),
            )
            .with_body(html.into_bytes()))
    }

    /// GET /updates - Database updates test
    ///
    /// Fetches N random rows, updates each with a new random number, persists
    #[get("/updates")]
    async fn updates_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
        let state = get_state();
        let count = parse_queries(req.query_param("queries"));

        let worlds: Vec<World> = (0..count)
            .map(|_| {
                let mut world = state.db.get_random_world();
                state.db.update_world(&mut world);
                world
            })
            .collect();

        HttpResponse::json(&worlds)
    }

    /// GET /cached-queries - Cached queries test
    ///
    /// Like /queries but results should be cached
    #[get("/cached-queries")]
    async fn cached_queries_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
        // For simplicity, we use the same implementation as queries
        // A real implementation would use armature-cache with Redis/Memcached
        let state = get_state();
        let count = parse_queries(req.query_param("queries"));

        let worlds: Vec<World> = (0..count).map(|_| state.db.get_random_world()).collect();

        HttpResponse::json(&worlds)
    }
}

// ============================================================================
// Module
// ============================================================================

#[module(controllers: [TechEmpowerController])]
#[derive(Default)]
struct AppModule;

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() {
    // Initialize state
    let _ = get_state();

    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║         TechEmpower Framework Benchmark Server             ║");
    println!("╠════════════════════════════════════════════════════════════╣");
    println!("║  Endpoints:                                                ║");
    println!("║    GET /json            - JSON serialization               ║");
    println!("║    GET /plaintext       - Plaintext response               ║");
    println!("║    GET /db              - Single DB query                  ║");
    println!("║    GET /queries?q=N     - Multiple DB queries (1-500)      ║");
    println!("║    GET /fortunes        - Template rendering               ║");
    println!("║    GET /updates?q=N     - DB updates (1-500)               ║");
    println!("║    GET /cached-queries  - Cached queries (1-500)           ║");
    println!("╠════════════════════════════════════════════════════════════╣");
    println!("║  Benchmark with:                                           ║");
    println!("║    wrk -t4 -c256 -d15s http://127.0.0.1:8080/json          ║");
    println!("║    wrk -t4 -c256 -d15s http://127.0.0.1:8080/plaintext     ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    println!();

    let app = Application::create::<AppModule>().await;

    println!("🚀 Server starting on http://127.0.0.1:8080");
    println!("   Press Ctrl+C to stop\n");

    app.listen(8080).await.unwrap();
}
