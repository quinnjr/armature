//! Route listing command
//!
//! Lists all routes defined in the application.

use crate::error::CliError;
use std::fs;
use std::path::Path;

/// Route information
#[derive(Debug)]
struct RouteInfo {
    method: String,
    path: String,
    #[allow(dead_code)]
    handler: String,
    middleware: Vec<String>,
    guards: Vec<String>,
}

/// List all routes command
pub fn execute(project_dir: Option<&str>) -> Result<(), CliError> {
    let dir = project_dir.unwrap_or(".");

    println!("🗺️  Armature Routes");
    println!("==================");
    println!();

    // Find routes in the project
    let routes = find_routes(dir)?;

    if routes.is_empty() {
        println!("No routes found.");
        println!();
        println!("Routes are typically defined in:");
        println!("  - src/main.rs");
        println!("  - src/routes.rs");
        println!("  - src/controllers/*.rs");
        return Ok(());
    }

    // Print routes table
    print_routes_table(&routes);

    // Print statistics
    println!();
    println!("Statistics:");
    println!("  Total routes: {}", routes.len());

    let methods: std::collections::HashSet<_> = routes.iter().map(|r| &r.method).collect();
    println!("  HTTP methods: {}", methods.len());

    let with_middleware = routes.iter().filter(|r| !r.middleware.is_empty()).count();
    println!("  Routes with middleware: {}", with_middleware);

    let with_guards = routes.iter().filter(|r| !r.guards.is_empty()).count();
    println!("  Routes with guards: {}", with_guards);

    Ok(())
}

fn find_routes(dir: &str) -> Result<Vec<RouteInfo>, CliError> {
    let mut routes = Vec::new();

    // Search for route definitions in Rust files
    let paths_to_search = vec![
        format!("{}/src/main.rs", dir),
        format!("{}/src/routes.rs", dir),
        format!("{}/src/routes/mod.rs", dir),
    ];

    for path in paths_to_search {
        if Path::new(&path).exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                routes.extend(parse_routes(&content));
            }
        }
    }

    // Search in controllers directory
    let controllers_dir = format!("{}/src/controllers", dir);
    if let Ok(entries) = fs::read_dir(&controllers_dir) {
        for entry in entries.flatten() {
            if let Ok(content) = fs::read_to_string(entry.path()) {
                routes.extend(parse_routes(&content));
            }
        }
    }

    Ok(routes)
}

fn parse_routes(content: &str) -> Vec<RouteInfo> {
    let mut routes = Vec::new();

    // Simple pattern matching for route decorators
    // This is a simplified parser - a real implementation would use syn/quote
    for line in content.lines() {
        let line = line.trim();

        // Match route decorators like #[get("/path")]
        if line.starts_with("#[") && line.contains("(\"") {
            if let Some(route) = parse_route_decorator(line) {
                routes.push(route);
            }
        }
    }

    routes
}

fn parse_route_decorator(line: &str) -> Option<RouteInfo> {
    // Simple regex-like parsing
    let methods = vec!["get", "post", "put", "delete", "patch", "head", "options"];

    for method in methods {
        let pattern = format!("#[{}(\"", method);
        if line.contains(&pattern) {
            if let Some(start) = line.find("(\"") {
                if let Some(end) = line[start..].find("\")") {
                    let path = &line[start + 2..start + end];
                    return Some(RouteInfo {
                        method: method.to_uppercase(),
                        path: path.to_string(),
                        handler: "handler".to_string(),
                        middleware: Vec::new(),
                        guards: Vec::new(),
                    });
                }
            }
        }
    }

    None
}

fn print_routes_table(routes: &[RouteInfo]) {
    // Calculate column widths
    let method_width = routes
        .iter()
        .map(|r| r.method.len())
        .max()
        .unwrap_or(6)
        .max(6);
    let path_width = routes
        .iter()
        .map(|r| r.path.len())
        .max()
        .unwrap_or(4)
        .max(4);

    // Print header
    println!("{:width$}  PATH", "METHOD", width = method_width);
    println!("{}", "-".repeat(method_width + path_width + 2));

    // Print routes
    for route in routes {
        println!(
            "{:width$}  {}",
            route.method,
            route.path,
            width = method_width
        );

        if !route.middleware.is_empty() {
            println!("  └─ Middleware: {}", route.middleware.join(", "));
        }
        if !route.guards.is_empty() {
            println!("  └─ Guards: {}", route.guards.join(", "));
        }
    }
}

/// List routes with filtering
pub fn execute_with_filter(
    project_dir: Option<&str>,
    method: Option<&str>,
) -> Result<(), CliError> {
    let dir = project_dir.unwrap_or(".");

    println!("🗺️  Armature Routes");
    if let Some(m) = method {
        println!("   Filtered by method: {}", m.to_uppercase());
    }
    println!("==================");
    println!();

    let routes = find_routes(dir)?;

    let filtered_routes: Vec<_> = if let Some(m) = method {
        routes
            .into_iter()
            .filter(|r| r.method.eq_ignore_ascii_case(m))
            .collect()
    } else {
        routes
    };

    if filtered_routes.is_empty() {
        println!("No routes found.");
        return Ok(());
    }

    print_routes_table(&filtered_routes);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_route_decorator() {
        let line = r#"#[get("/api/users")]"#;
        let route = parse_route_decorator(line).unwrap();
        assert_eq!(route.method, "GET");
        assert_eq!(route.path, "/api/users");
    }

    #[test]
    fn test_parse_post_route() {
        let line = r#"#[post("/api/users")]"#;
        let route = parse_route_decorator(line).unwrap();
        assert_eq!(route.method, "POST");
        assert_eq!(route.path, "/api/users");
    }
}
