import { Component } from '@angular/core';
import { CommonModule } from '@angular/common';
import { DocPageComponent, DocPage } from '../../shared/doc-page.component';

@Component({
  selector: 'app-micro-framework-guide',
  standalone: true,
  imports: [CommonModule, DocPageComponent],
  template: `<app-doc-page [page]="page"></app-doc-page>`
})
export class MicroFrameworkGuideComponent {
  page: DocPage = {
    title: 'Micro-Framework Mode',
    subtitle: 'A lightweight, Actix-style API for building web applications without the full module/controller system. Perfect for microservices, simple APIs, and rapid prototyping.',
    icon: '⚡',
    badge: 'New',
    features: [
      {
        icon: '🚀',
        title: 'Minimal Setup',
        description: 'Start with App::new().route().run() — no modules required'
      },
      {
        icon: '🎯',
        title: 'Function Handlers',
        description: 'Simple async functions instead of controller classes'
      },
      {
        icon: '📦',
        title: 'Composable Middleware',
        description: 'Stack middleware with wrap() for logging, CORS, etc.'
      },
      {
        icon: '🔧',
        title: 'Shared State',
        description: 'Type-safe state via Data<T> with Arc-based sharing'
      }
    ],
    sections: [
      {
        id: 'quick-start',
        title: 'Quick Start',
        content: `<p>The micro-framework provides a minimal, function-based API for building HTTP services:</p>`,
        codeBlocks: [
          {
            language: 'rust',
            filename: 'main.rs',
            code: `use armature_core::micro::*;
use armature_core::{Error, HttpRequest, HttpResponse};

async fn hello(_req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok().with_body(b"Hello, World!".to_vec()))
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    App::new()
        .route("/", get(hello))
        .run("127.0.0.1:8080")
        .await
}`
          }
        ]
      },
      {
        id: 'routing',
        title: 'Routing',
        content: `<p>Register routes using method helpers that mirror HTTP verbs:</p>`,
        subsections: [
          {
            id: 'method-helpers',
            title: 'Method Helpers',
            content: `<p>Available helpers: <code>get()</code>, <code>post()</code>, <code>put()</code>, <code>delete()</code>, <code>patch()</code>, <code>head()</code>, <code>options()</code>, <code>any()</code></p>`,
            codeBlocks: [
              {
                language: 'rust',
                code: `App::new()
    .route("/", get(index))
    .route("/users", get(list).post(create))
    .route("/users/:id", get(show).put(update).delete(destroy))
    .route("/any-method", any(catch_all))`
              }
            ]
          },
          {
            id: 'path-params',
            title: 'Path Parameters',
            content: `<p>Extract parameters from the URL path using <code>:param</code> syntax:</p>`,
            codeBlocks: [
              {
                language: 'rust',
                code: `async fn get_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    // Extract :id from /users/:id
    let id = req.param("id").unwrap();

    // Multiple params: /users/:user_id/posts/:post_id
    let user_id = req.param("user_id").unwrap();
    let post_id = req.param("post_id").unwrap();

    HttpResponse::json(&User { id: id.parse()? })
}

App::new()
    .route("/users/:id", get(get_user))
    .route("/users/:user_id/posts/:post_id", get(get_post))`
              }
            ]
          },
          {
            id: 'query-params',
            title: 'Query Parameters',
            content: `<p>Access query string parameters:</p>`,
            codeBlocks: [
              {
                language: 'rust',
                code: `async fn search(req: HttpRequest) -> Result<HttpResponse, Error> {
    // GET /search?q=rust&page=1
    let query = req.query("q").unwrap_or(&"".to_string());
    let page = req.query("page")
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(1);

    HttpResponse::json(&SearchResults { query, page })
}`
              }
            ]
          }
        ]
      },
      {
        id: 'middleware',
        title: 'Middleware',
        content: `<p>Middleware wraps handlers and can modify requests/responses. Middleware executes in the order added (first = outermost).</p>`,
        subsections: [
          {
            id: 'adding-middleware',
            title: 'Adding Middleware',
            codeBlocks: [
              {
                language: 'rust',
                code: `App::new()
    .wrap(Logger::default())      // Outermost - runs first
    .wrap(Cors::permissive())     // Runs second
    .wrap(Compress::default())    // Innermost - runs last
    .route("/", get(handler))`
              }
            ]
          },
          {
            id: 'custom-middleware',
            title: 'Custom Middleware',
            content: `<p>Implement the <code>Middleware</code> trait:</p>`,
            codeBlocks: [
              {
                language: 'rust',
                code: `use armature_core::micro::*;
use std::pin::Pin;
use std::future::Future;

struct Timing;

impl Middleware for Timing {
    fn call(
        &self,
        req: HttpRequest,
        next: Next,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>> {
        Box::pin(async move {
            let start = std::time::Instant::now();

            // Call next handler in chain
            let mut response = next(req).await?;

            // Add timing header
            response.headers.insert(
                "X-Response-Time".to_string(),
                format!("{}ms", start.elapsed().as_millis()),
            );

            Ok(response)
        })
    }
}

App::new()
    .wrap(Timing)
    .route("/", get(handler))`
              }
            ]
          }
        ]
      },
      {
        id: 'state',
        title: 'State Management',
        content: `<p>Share state across handlers using <code>Data&lt;T&gt;</code>:</p>`,
        codeBlocks: [
          {
            language: 'rust',
            code: `use armature_core::micro::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct AppState {
    request_count: AtomicU64,
    db_pool: Pool,
}

async fn handler(req: HttpRequest) -> Result<HttpResponse, Error> {
    // State is accessible via request extensions
    Ok(HttpResponse::ok())
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let state = AppState {
        request_count: AtomicU64::new(0),
        db_pool: create_pool().await,
    };

    App::new()
        .data(state)  // Register state
        .route("/", get(handler))
        .run("0.0.0.0:8080")
        .await
}`
          }
        ]
      },
      {
        id: 'scopes',
        title: 'Route Scopes',
        content: `<p>Group routes under a common prefix:</p>`,
        subsections: [
          {
            id: 'basic-scopes',
            title: 'Basic Scopes',
            codeBlocks: [
              {
                language: 'rust',
                code: `App::new()
    .service(
        scope("/api/v1")
            .route("/users", get(list_users).post(create_user))
            .route("/users/:id", get(get_user).delete(delete_user))
            .route("/posts", get(list_posts))
    )
    .route("/health", get(health_check))

// Routes created:
// GET/POST /api/v1/users
// GET/DELETE /api/v1/users/:id
// GET /api/v1/posts
// GET /health`
              }
            ]
          },
          {
            id: 'nested-scopes',
            title: 'Nested Scopes',
            codeBlocks: [
              {
                language: 'rust',
                code: `App::new()
    .service(
        scope("/api")
            .service(
                scope("/v1")
                    .route("/users", get(v1_users))
            )
            .service(
                scope("/v2")
                    .route("/users", get(v2_users))
            )
    )

// Routes: GET /api/v1/users, GET /api/v2/users`
              }
            ]
          },
          {
            id: 'scoped-middleware',
            title: 'Scoped Middleware',
            content: `<p>Apply middleware only to specific scopes:</p>`,
            codeBlocks: [
              {
                language: 'rust',
                code: `App::new()
    .wrap(Logger::default())  // Global middleware
    .service(
        scope("/api")
            .wrap(auth_middleware)  // Only for /api/*
            .route("/users", get(users))
    )
    .route("/public", get(public_page))  // No auth required`
              }
            ]
          }
        ]
      },
      {
        id: 'built-in-middleware',
        title: 'Built-in Middleware',
        subsections: [
          {
            id: 'logger',
            title: 'Logger',
            content: `<p>Logs requests with timing information:</p>`,
            codeBlocks: [
              {
                language: 'rust',
                code: `use armature_core::micro::{Logger, LogFormat};

// Default format
App::new().wrap(Logger::default())

// Custom format
App::new().wrap(Logger::new(LogFormat::Combined))

// Output: INFO Request completed method=GET path=/users status=200 duration_ms=5`
              }
            ]
          },
          {
            id: 'cors',
            title: 'CORS',
            content: `<p>Cross-Origin Resource Sharing configuration:</p>`,
            codeBlocks: [
              {
                language: 'rust',
                code: `use armature_core::micro::Cors;

// Permissive (allow all)
App::new().wrap(Cors::permissive())

// Custom configuration
App::new().wrap(
    Cors::default()
        .allowed_origins(["https://example.com"])
        .allowed_methods(["GET", "POST", "PUT", "DELETE"])
        .allowed_headers(["Content-Type", "Authorization"])
        .allow_credentials(true)
        .max_age(3600)
)`
              }
            ]
          },
          {
            id: 'compress',
            title: 'Compress',
            content: `<p>Adds compression headers:</p>`,
            codeBlocks: [
              {
                language: 'rust',
                code: `use armature_core::micro::{Compress, CompressionLevel};

App::new().wrap(Compress::default())
App::new().wrap(Compress::new(CompressionLevel::Best))`
              }
            ]
          }
        ]
      },
      {
        id: 'json-example',
        title: 'Full JSON API Example',
        content: `<p>A complete REST API example with all features:</p>`,
        codeBlocks: [
          {
            language: 'rust',
            filename: 'main.rs',
            code: `use armature_core::micro::*;
use armature_core::{Error, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct User {
    id: u64,
    name: String,
}

async fn list_users(_req: HttpRequest) -> Result<HttpResponse, Error> {
    HttpResponse::json(&vec![
        User { id: 1, name: "Alice".to_string() },
        User { id: 2, name: "Bob".to_string() },
    ])
}

async fn get_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    let id: u64 = req.param("id")
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Error::validation("Invalid user ID"))?;

    HttpResponse::json(&User { id, name: "Alice".to_string() })
}

async fn create_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    let user: User = req.json()?;
    HttpResponse::created().json(&user)
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    App::new()
        .wrap(Logger::default())
        .wrap(Cors::permissive())
        .service(
            scope("/api/v1")
                .route("/users", get(list_users).post(create_user))
                .route("/users/:id", get(get_user))
        )
        .route("/health", get(|_| async { Ok(HttpResponse::ok()) }))
        .run("0.0.0.0:3000")
        .await
}`
          }
        ]
      },
      {
        id: 'when-to-use',
        title: 'When to Use',
        content: `<p>Choose the right mode for your project:</p>
        <table>
          <thead>
            <tr>
              <th>Use Micro-Framework When</th>
              <th>Use Full Framework When</th>
            </tr>
          </thead>
          <tbody>
            <tr><td>✅ Building microservices</td><td>✅ Large enterprise applications</td></tr>
            <tr><td>✅ Simple REST APIs</td><td>✅ Need dependency injection</td></tr>
            <tr><td>✅ Quick prototypes</td><td>✅ Want decorator-based controllers</td></tr>
            <tr><td>✅ Performance-critical services</td><td>✅ Complex middleware requirements</td></tr>
            <tr><td>✅ Prefer explicit over implicit</td><td>✅ Automatic OpenAPI generation</td></tr>
          </tbody>
        </table>`
      },
      {
        id: 'comparison',
        title: 'Micro vs Full Framework',
        content: `<table>
          <thead>
            <tr>
              <th>Aspect</th>
              <th>Micro-Framework</th>
              <th>Full Framework</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td>Setup</td>
              <td><code>App::new()</code></td>
              <td><code>Application::bootstrap(Module)</code></td>
            </tr>
            <tr>
              <td>Routing</td>
              <td><code>get(handler)</code></td>
              <td><code>#[get("/")]</code> decorator</td>
            </tr>
            <tr>
              <td>DI</td>
              <td>Manual <code>Data&lt;T&gt;</code></td>
              <td><code>#[injectable]</code> auto-wiring</td>
            </tr>
            <tr>
              <td>Middleware</td>
              <td><code>wrap(mw)</code></td>
              <td><code>#[UseGuards]</code>, <code>#[UsePipes]</code></td>
            </tr>
            <tr>
              <td>Best for</td>
              <td>Microservices</td>
              <td>Enterprise apps</td>
            </tr>
          </tbody>
        </table>`
      },
      {
        id: 'performance',
        title: 'Performance',
        content: `<p>Benchmark results (December 2025):</p>
        <table>
          <thead>
            <tr>
              <th>Benchmark</th>
              <th>Time</th>
            </tr>
          </thead>
          <tbody>
            <tr><td>Empty app creation</td><td>25ns</td></tr>
            <tr><td>App with 5 routes</td><td>1.9-4.7µs</td></tr>
            <tr><td>Route (no middleware)</td><td>875ns</td></tr>
            <tr><td>Route (3 middleware)</td><td>1.9µs</td></tr>
            <tr><td>Data access</td><td>&lt;1ns</td></tr>
            <tr><td>JSON handler</td><td>3.7µs</td></tr>
          </tbody>
        </table>`
      },
      {
        id: 'best-practices',
        title: 'Best Practices',
        content: `<ol>
          <li><strong>Use scopes for API versioning</strong> — Group routes by version: <code>/api/v1</code>, <code>/api/v2</code></li>
          <li><strong>Register middleware in correct order</strong> — Logger first (logs all), then CORS (handles preflight), then auth</li>
          <li><strong>Use typed state</strong> — Define proper structs instead of <code>HashMap&lt;String, Any&gt;</code></li>
          <li><strong>Handle errors properly</strong> — Use <code>?</code> operator and proper <code>Error</code> types</li>
          <li><strong>Keep handlers small</strong> — Delegate business logic to service functions</li>
        </ol>`
      }
    ],
    relatedDocs: [
      {
        id: 'di-guide',
        title: 'Dependency Injection',
        description: 'Full DI system for enterprise apps'
      },
      {
        id: 'middleware-guide',
        title: 'Middleware Guide',
        description: 'Advanced middleware patterns'
      },
      {
        id: 'route-groups',
        title: 'Route Groups',
        description: 'Organizing routes in the full framework'
      }
    ],
    seeAlso: [
      { title: 'Getting Started', id: 'readme' },
      { title: 'Configuration', id: 'config-guide' },
      { title: 'Testing Guide', id: 'testing-guide' }
    ]
  };
}

