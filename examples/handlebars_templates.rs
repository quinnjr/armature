#![allow(dead_code)]
//! Handlebars Templating Example
//!
//! This example demonstrates how to use the Handlebars template engine
//! directly with Armature's dependency injection container.
//!
//! No special Armature plugin is needed - just use the handlebars crate
//! and inject it as a service!

use armature_core::handler::from_legacy_handler;
use armature_core::*;
use armature_proc_macro::*;
use handlebars::Handlebars;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// =============================================================================
// Template Constants
// =============================================================================

const LAYOUT_TEMPLATE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{{title}} - Armature</title>
    <style>
        * { box-sizing: border-box; margin: 0; padding: 0; }
        body { font-family: system-ui, -apple-system, sans-serif; line-height: 1.6; background: #f4f4f4; }
        header { background: linear-gradient(135deg, #667eea 0%, #764ba2 100%); color: white; padding: 1rem 2rem; }
        header h1 { font-size: 1.5rem; }
        nav { margin-top: 0.5rem; }
        nav a { color: rgba(255,255,255,0.9); text-decoration: none; margin-right: 1rem; }
        nav a:hover { text-decoration: underline; }
        main { max-width: 800px; margin: 2rem auto; padding: 0 1rem; }
        .card { background: white; border-radius: 8px; padding: 1.5rem; margin-bottom: 1rem; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }
        .badge { display: inline-block; padding: 0.25rem 0.5rem; border-radius: 4px; font-size: 0.75rem; font-weight: 600; }
        .badge-admin { background: #fee2e2; color: #dc2626; }
        .badge-user { background: #dbeafe; color: #2563eb; }
        .badge-active { background: #d1fae5; color: #059669; }
        .badge-inactive { background: #f3f4f6; color: #6b7280; }
        a.btn { display: inline-block; padding: 0.5rem 1rem; background: #667eea; color: white; text-decoration: none; border-radius: 4px; }
        a.btn:hover { background: #5a67d8; }
        ul { margin: 1rem 0; padding-left: 1.5rem; }
        li { margin: 0.5rem 0; }
    </style>
</head>
<body>
    <header>
        <h1>🦾 Armature + Handlebars</h1>
        <nav>
            <a href="/">Home</a>
            <a href="/users">Users</a>
        </nav>
    </header>
    <main>
        {{{body}}}
    </main>
</body>
</html>"#;

const HOME_TEMPLATE: &str = r#"{{#> layout}}
{{#*inline "body"}}
<div class="card">
    <h2>{{title}}</h2>
    <p>{{message}}</p>
    <h3 style="margin-top: 1.5rem;">Key Points:</h3>
    <ul>
        {{#each features}}
        <li>✅ {{this}}</li>
        {{/each}}
    </ul>
    <p style="margin-top: 1.5rem;">
        <a href="/users" class="btn">View Users →</a>
    </p>
</div>
{{/inline}}
{{/layout}}"#;

const USERS_TEMPLATE: &str = r#"{{#> layout}}
{{#*inline "body"}}
<div class="card">
    <h2>{{title}}</h2>
    <p>Total: {{count}} users</p>
</div>
{{#each users}}
<div class="card">
    <h3>{{this.name}}</h3>
    <p>📧 {{this.email}}</p>
    <p>
        <span class="badge badge-{{this.role}}">{{this.role}}</span>
        {{#if this.active}}
        <span class="badge badge-active">Active</span>
        {{else}}
        <span class="badge badge-inactive">Inactive</span>
        {{/if}}
    </p>
    <p style="margin-top: 1rem;">
        <a href="/users/{{this.id}}" class="btn">View Profile →</a>
    </p>
</div>
{{/each}}
{{/inline}}
{{/layout}}"#;

const USER_TEMPLATE: &str = r#"{{#> layout}}
{{#*inline "body"}}
<div class="card">
    <h2>{{user.name}}</h2>
    <table style="margin: 1rem 0; width: 100%;">
        <tr><td><strong>ID:</strong></td><td>{{user.id}}</td></tr>
        <tr><td><strong>Email:</strong></td><td>{{user.email}}</td></tr>
        <tr><td><strong>Role:</strong></td><td><span class="badge badge-{{user.role}}">{{user.role}}</span></td></tr>
        <tr>
            <td><strong>Status:</strong></td>
            <td>
                {{#if user.active}}
                <span class="badge badge-active">Active</span>
                {{else}}
                <span class="badge badge-inactive">Inactive</span>
                {{/if}}
            </td>
        </tr>
    </table>
    <p style="margin-top: 1.5rem;">
        <a href="/users">← Back to Users</a>
    </p>
</div>
{{/inline}}
{{/layout}}"#;

// =============================================================================
// Template Service - Injectable Handlebars wrapper
// =============================================================================

/// A simple injectable template service using Handlebars.
/// This shows how any external library can be wrapped and used with DI.
#[derive(Clone)]
struct TemplateService {
    hbs: Arc<Handlebars<'static>>,
}

impl TemplateService {
    fn new() -> Self {
        let mut hbs = Handlebars::new();

        // Register templates inline (in production, you'd load these from files)
        hbs.register_template_string("layout", LAYOUT_TEMPLATE)
            .expect("Failed to register layout template");
        hbs.register_template_string("home", HOME_TEMPLATE)
            .expect("Failed to register home template");
        hbs.register_template_string("users", USERS_TEMPLATE)
            .expect("Failed to register users template");
        hbs.register_template_string("user", USER_TEMPLATE)
            .expect("Failed to register user template");

        Self { hbs: Arc::new(hbs) }
    }

    /// Render a template to an HTML response
    fn render<T: Serialize>(&self, template: &str, data: &T) -> Result<HttpResponse, Error> {
        let html = self
            .hbs
            .render(template, data)
            .map_err(|e| Error::Internal(format!("Template error: {}", e)))?;

        Ok(HttpResponse::ok().with_body(html.into_bytes()).with_header(
            "Content-Type".to_string(),
            "text/html; charset=utf-8".to_string(),
        ))
    }
}

// =============================================================================
// Data Models
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct User {
    id: u64,
    name: String,
    email: String,
    role: String,
    active: bool,
}

// =============================================================================
// User Service - Business logic
// =============================================================================

#[derive(Clone)]
struct UserService;

impl UserService {
    fn new() -> Self {
        Self
    }

    fn get_all(&self) -> Vec<User> {
        vec![
            User {
                id: 1,
                name: "Alice Smith".to_string(),
                email: "alice@example.com".to_string(),
                role: "admin".to_string(),
                active: true,
            },
            User {
                id: 2,
                name: "Bob Johnson".to_string(),
                email: "bob@example.com".to_string(),
                role: "user".to_string(),
                active: true,
            },
            User {
                id: 3,
                name: "Charlie Brown".to_string(),
                email: "charlie@example.com".to_string(),
                role: "user".to_string(),
                active: false,
            },
        ]
    }

    fn get_by_id(&self, id: u64) -> Option<User> {
        self.get_all().into_iter().find(|u| u.id == id)
    }
}

// =============================================================================
// Controllers - Using injected services
// =============================================================================

/// Home controller with injected template service
#[controller("/")]
#[derive(Clone)]
struct HomeController {
    templates: TemplateService,
}

#[routes]
impl HomeController {
    async fn index(&self, _req: HttpRequest) -> Result<HttpResponse, Error> {
        let data = serde_json::json!({
            "title": "Armature + Handlebars",
            "message": "Server-side templating with dependency injection!",
            "features": [
                "No plugin required - use handlebars directly",
                "Injectable template service",
                "Type-safe template rendering",
                "Works with any template engine"
            ]
        });

        self.templates.render("home", &data)
    }
}

/// User controller with multiple injected services
#[controller("/users")]
#[derive(Clone)]
struct UserController {
    templates: TemplateService,
    users: UserService,
}

#[routes]
impl UserController {
    async fn list(&self, _req: HttpRequest) -> Result<HttpResponse, Error> {
        let users = self.users.get_all();

        let data = serde_json::json!({
            "title": "Users",
            "users": users,
            "count": users.len()
        });

        self.templates.render("users", &data)
    }

    async fn show(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        let id: u64 = req
            .param("id")
            .ok_or_else(|| Error::BadRequest("Missing id".to_string()))?
            .parse()
            .map_err(|_| Error::BadRequest("Invalid id".to_string()))?;

        let user = self
            .users
            .get_by_id(id)
            .ok_or_else(|| Error::NotFound("User not found".to_string()))?;

        let data = serde_json::json!({
            "title": format!("User: {}", user.name),
            "user": user
        });

        self.templates.render("user", &data)
    }
}

// =============================================================================
// Main
// =============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("📝 Handlebars Templating Example");
    println!("================================\n");

    // Create services
    let template_service = TemplateService::new();
    let user_service = UserService::new();

    // Create DI container and register services
    let container = Container::new();

    // Create router with routes
    let mut router = Router::new();

    // Home route
    let home_ctrl = HomeController {
        templates: template_service.clone(),
    };
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/".to_string(),
        handler: from_legacy_handler(Arc::new(move |req: HttpRequest| {
            let ctrl = home_ctrl.clone();
            Box::pin(async move { ctrl.index(req).await })
        })),
        constraints: None,
    });

    // Users list route
    let users_ctrl = UserController {
        templates: template_service.clone(),
        users: user_service.clone(),
    };
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/users".to_string(),
        handler: from_legacy_handler(Arc::new(move |req: HttpRequest| {
            let ctrl = users_ctrl.clone();
            Box::pin(async move { ctrl.list(req).await })
        })),
        constraints: None,
    });

    // User detail route
    let user_ctrl = UserController {
        templates: template_service.clone(),
        users: user_service.clone(),
    };
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/users/:id".to_string(),
        handler: from_legacy_handler(Arc::new(move |req: HttpRequest| {
            let ctrl = user_ctrl.clone();
            Box::pin(async move { ctrl.show(req).await })
        })),
        constraints: None,
    });

    let app = Application::new(container, router);

    println!("✅ Services injected:");
    println!("   • TemplateService (Handlebars wrapper)");
    println!("   • UserService (business logic)");
    println!();
    println!("🚀 Server starting on http://localhost:3000");
    println!();
    println!("Routes:");
    println!("  • GET /          → Home page");
    println!("  • GET /users     → User list");
    println!("  • GET /users/:id → User detail");
    println!();

    app.listen(3000).await?;

    Ok(())
}
