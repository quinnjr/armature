#![allow(dead_code)]
// Example: running a Rhai application from Rust code.
//
// This shows how to embed the Rhai app runner in a Rust binary.
// Most users will prefer `armature run app.rhai` via the CLI instead.
//
// Run:
//   cargo run --example rhai_app
//
// Then test:
//   curl http://localhost:3001/health
//   curl -H "Authorization: Bearer token" http://localhost:3001/api/tasks
//   curl http://localhost:3001/api/greetings/world

use std::path::Path;

#[tokio::main]
async fn main() {
    println!("Starting Rhai application from examples/rhai_app/app.rhai");
    println!();
    println!("Endpoints:");
    println!("  GET    /health              — health check");
    println!("  GET    /api/tasks           — list tasks   (needs Authorization: Bearer <token>)");
    println!("  GET    /api/tasks/:id       — get task     (needs Authorization: Bearer <token>)");
    println!("  POST   /api/tasks           — create task  (needs Authorization: Bearer <token>)");
    println!("  DELETE /api/tasks/:id       — delete task  (needs Authorization: Bearer <token>)");
    println!("  GET    /api/greetings/:name — greet someone");
    println!();

    let script = Path::new("examples/rhai_app/app.rhai");

    let config = armature_app::RunConfig {
        port: Some(3001), // override script port to avoid conflicts
        host: None,       // use the default (0.0.0.0)
    };

    if let Err(e) = armature_app::run(script, config).await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
