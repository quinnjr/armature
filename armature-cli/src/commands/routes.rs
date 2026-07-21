//! Route listing command
//!
//! Lists all routes defined in the application, with optional filtering by HTTP
//! method or path pattern and selectable output formats.

use crate::error::CliError;
use serde::Serialize;
use std::fs;
use std::path::Path;

/// Route information
#[derive(Debug, Serialize)]
struct RouteInfo {
    method: String,
    path: String,
    handler: String,
    middleware: Vec<String>,
    guards: Vec<String>,
}

/// Output format for the routes command.
#[derive(Debug, Clone, Copy)]
pub enum RouteFormat {
    Table,
    Json,
    Yaml,
    Markdown,
}

/// List all routes (table format, no filters). Retained for backwards compatibility.
pub fn execute(project_dir: Option<&str>) -> Result<(), CliError> {
    run(project_dir, None, None, RouteFormat::Table, false)
}

/// List routes filtered by HTTP method (table format).
pub fn execute_with_filter(
    project_dir: Option<&str>,
    method: Option<&str>,
) -> Result<(), CliError> {
    run(project_dir, method, None, RouteFormat::Table, false)
}

/// List routes with full filtering and format control.
pub fn run(
    project_dir: Option<&str>,
    method: Option<&str>,
    path: Option<&str>,
    format: RouteFormat,
    show_middleware: bool,
) -> Result<(), CliError> {
    let dir = project_dir.unwrap_or(".");

    let mut routes = find_routes(dir)?;

    // Apply method filter.
    if let Some(m) = method {
        routes.retain(|r| r.method.eq_ignore_ascii_case(m));
    }

    // Apply path pattern filter (substring match).
    if let Some(p) = path {
        routes.retain(|r| r.path.contains(p));
    }

    match format {
        RouteFormat::Table => render_table(&routes, method, path, show_middleware),
        RouteFormat::Json => render_json(&routes)?,
        RouteFormat::Yaml => render_yaml(&routes)?,
        RouteFormat::Markdown => render_markdown(&routes, show_middleware),
    }

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
        if Path::new(&path).exists()
            && let Ok(content) = fs::read_to_string(&path)
        {
            routes.extend(parse_routes(&content));
        }
    }

    // Search in controllers directory (deterministic order for stable output).
    let controllers_dir = format!("{}/src/controllers", dir);
    if let Ok(entries) = fs::read_dir(&controllers_dir) {
        let mut files: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        files.sort();
        for file in files {
            if let Ok(content) = fs::read_to_string(&file) {
                routes.extend(parse_routes(&content));
            }
        }
    }

    Ok(routes)
}

fn parse_routes(content: &str) -> Vec<RouteInfo> {
    let mut routes = Vec::new();

    // `#[middleware(...)]` / `#[guard(...)]` attributes precede the route
    // decorator they apply to; accumulate them until the next route is seen.
    let mut pending_middleware: Vec<String> = Vec::new();
    let mut pending_guards: Vec<String> = Vec::new();

    for line in content.lines() {
        let line = line.trim();

        if let Some(names) = parse_attr_list(line, "middleware") {
            pending_middleware.extend(names);
            continue;
        }
        if let Some(names) = parse_attr_list(line, "guard") {
            pending_guards.extend(names);
            continue;
        }

        // Match route decorators like #[get("/path")].
        if line.starts_with("#[")
            && line.contains("(\"")
            && let Some(mut route) = parse_route_decorator(line)
        {
            route.middleware = std::mem::take(&mut pending_middleware);
            route.guards = std::mem::take(&mut pending_guards);
            routes.push(route);
        }
    }

    routes
}

/// Parse `#[attr(A, B)]` into `["A", "B"]`, or `None` if the line is not that attribute.
fn parse_attr_list(line: &str, attr: &str) -> Option<Vec<String>> {
    let prefix = format!("#[{}(", attr);
    let rest = line.strip_prefix(&prefix)?;
    let inner = rest.strip_suffix(")]").unwrap_or(rest);
    let names: Vec<String> = inner
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Some(names)
}

fn parse_route_decorator(line: &str) -> Option<RouteInfo> {
    let methods = vec!["get", "post", "put", "delete", "patch", "head", "options"];

    for method in methods {
        let pattern = format!("#[{}(\"", method);
        if line.contains(&pattern)
            && let Some(start) = line.find("(\"")
            && let Some(end) = line[start..].find("\")")
        {
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

    None
}

// =============================================================================
// Renderers
// =============================================================================

fn no_routes_message() {
    println!("No routes found.");
    println!();
    println!("Routes are typically defined in:");
    println!("  - src/main.rs");
    println!("  - src/routes.rs");
    println!("  - src/controllers/*.rs");
}

fn render_table(
    routes: &[RouteInfo],
    method: Option<&str>,
    path: Option<&str>,
    show_middleware: bool,
) {
    println!("🗺️  Armature Routes");
    if let Some(m) = method {
        println!("   Filtered by method: {}", m.to_uppercase());
    }
    if let Some(p) = path {
        println!("   Filtered by path: {}", p);
    }
    println!("==================");
    println!();

    if routes.is_empty() {
        no_routes_message();
        return;
    }

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

    println!("{:width$}  PATH", "METHOD", width = method_width);
    println!("{}", "-".repeat(method_width + path_width + 2));

    for route in routes {
        println!(
            "{:width$}  {}",
            route.method,
            route.path,
            width = method_width
        );

        // Always surface guards; surface middleware when present or requested.
        if show_middleware || !route.middleware.is_empty() {
            if route.middleware.is_empty() {
                println!("  └─ Middleware: (none)");
            } else {
                println!("  └─ Middleware: {}", route.middleware.join(", "));
            }
        }
        if !route.guards.is_empty() {
            println!("  └─ Guards: {}", route.guards.join(", "));
        }
    }

    // Statistics
    println!();
    println!("Statistics:");
    println!("  Total routes: {}", routes.len());

    let methods: std::collections::HashSet<_> = routes.iter().map(|r| &r.method).collect();
    println!("  HTTP methods: {}", methods.len());

    let with_middleware = routes.iter().filter(|r| !r.middleware.is_empty()).count();
    println!("  Routes with middleware: {}", with_middleware);

    let with_guards = routes.iter().filter(|r| !r.guards.is_empty()).count();
    println!("  Routes with guards: {}", with_guards);
}

fn render_json(routes: &[RouteInfo]) -> Result<(), CliError> {
    let json = serde_json::to_string_pretty(routes)
        .map_err(|e| CliError::Command(format!("Failed to serialize routes to JSON: {}", e)))?;
    println!("{}", json);
    Ok(())
}

fn render_yaml(routes: &[RouteInfo]) -> Result<(), CliError> {
    let yaml = serde_yaml::to_string(routes)
        .map_err(|e| CliError::Command(format!("Failed to serialize routes to YAML: {}", e)))?;
    print!("{}", yaml);
    Ok(())
}

fn render_markdown(routes: &[RouteInfo], show_middleware: bool) {
    println!("# Routes");
    println!();
    if routes.is_empty() {
        println!("_No routes found._");
        return;
    }

    if show_middleware {
        println!("| Method | Path | Middleware | Guards |");
        println!("| ------ | ---- | ---------- | ------ |");
        for r in routes {
            println!(
                "| {} | {} | {} | {} |",
                r.method,
                r.path,
                if r.middleware.is_empty() {
                    "-".to_string()
                } else {
                    r.middleware.join(", ")
                },
                if r.guards.is_empty() {
                    "-".to_string()
                } else {
                    r.guards.join(", ")
                },
            );
        }
    } else {
        println!("| Method | Path |");
        println!("| ------ | ---- |");
        for r in routes {
            println!("| {} | {} |", r.method, r.path);
        }
    }
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

    #[test]
    fn test_parse_attr_list() {
        assert_eq!(
            parse_attr_list("#[middleware(LoggerMiddleware)]", "middleware"),
            Some(vec!["LoggerMiddleware".to_string()])
        );
        assert_eq!(
            parse_attr_list("#[middleware(A, B)]", "middleware"),
            Some(vec!["A".to_string(), "B".to_string()])
        );
        assert_eq!(parse_attr_list("#[get(\"/x\")]", "middleware"), None);
    }

    #[test]
    fn parses_middleware_and_guards_onto_next_route() {
        let content = r#"
#[middleware(LoggerMiddleware)]
#[guard(AuthGuard)]
#[get("/api/users")]
pub async fn list() {}

#[post("/api/users")]
pub async fn create() {}
"#;
        let routes = parse_routes(content);
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].middleware, vec!["LoggerMiddleware".to_string()]);
        assert_eq!(routes[0].guards, vec!["AuthGuard".to_string()]);
        // The attributes must not leak onto the second route.
        assert!(routes[1].middleware.is_empty());
        assert!(routes[1].guards.is_empty());
    }
}
