//! Mock server command for frontend development
//!
//! Runs a mock API server that generates fake data from OpenAPI specs.
//!
//! # Usage
//!
//! ```bash
//! # From OpenAPI spec
//! armature mock --spec openapi.yaml
//!
//! # With custom port
//! armature mock --spec api.json --port 4000
//!
//! # With custom data directory
//! armature mock --spec openapi.yaml --data ./mock-data
//!
//! # With response delay (simulate latency)
//! armature mock --spec openapi.yaml --delay 200
//! ```

use crate::error::{CliError, CliResult};
use crate::{info, success};
use armature_core::Router;
use chrono::{DateTime, Utc};
use colored::Colorize;
use openapiv3::{OpenAPI, Operation, PathItem, ReferenceOr, Schema, SchemaKind, Type as OapiType};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Arguments for the mock command
#[derive(Debug, Clone)]
pub struct MockArgs {
    /// Path to OpenAPI spec (JSON or YAML)
    pub spec: String,
    /// Port to run mock server on
    pub port: u16,
    /// Host to bind to
    pub host: String,
    /// Directory containing custom mock data files
    pub data_dir: Option<String>,
    /// Simulated response delay in milliseconds
    pub delay_ms: u64,
    /// Enable CORS headers
    pub cors: bool,
    /// Seed for random data generation (for reproducibility)
    pub seed: Option<u64>,
    /// Watch spec file for changes
    pub watch: bool,
}

impl Default for MockArgs {
    fn default() -> Self {
        Self {
            spec: "openapi.yaml".to_string(),
            port: 3000,
            host: "127.0.0.1".to_string(),
            data_dir: None,
            delay_ms: 0,
            cors: true,
            seed: None,
            watch: false,
        }
    }
}

/// Shared state for the mock server
struct MockState {
    spec: OpenAPI,
    custom_data: HashMap<String, Value>,
    delay_ms: u64,
    cors: bool,
    seed: Option<u64>,
    request_count: std::sync::atomic::AtomicU64,
    start_time: DateTime<Utc>,
}

/// Run the mock server
pub async fn run(args: MockArgs) -> CliResult<()> {
    info(&format!("Loading OpenAPI spec: {}", args.spec.cyan()));

    // Load and parse the OpenAPI spec
    let spec = load_openapi_spec(&args.spec)?;

    info(&format!(
        "Loaded API: {} v{}",
        spec.info.title.cyan(),
        spec.info.version.cyan()
    ));

    // Count endpoints
    let endpoint_count: usize = spec
        .paths
        .paths
        .values()
        .map(|item| match item {
            ReferenceOr::Item(path) => count_operations(path),
            _ => 0,
        })
        .sum();

    info(&format!(
        "Found {} endpoints",
        endpoint_count.to_string().cyan()
    ));

    // Load custom mock data if provided
    let custom_data = if let Some(data_dir) = &args.data_dir {
        load_custom_data(data_dir)?
    } else {
        HashMap::new()
    };

    // Create shared state
    let state = Arc::new(MockState {
        spec,
        custom_data,
        delay_ms: args.delay_ms,
        cors: args.cors,
        seed: args.seed,
        request_count: std::sync::atomic::AtomicU64::new(0),
        start_time: Utc::now(),
    });

    // Print routes
    println!();
    println!("  {}", "Available endpoints:".bright_white());
    print_routes(&state.spec);

    // Build router with mock handlers
    let router = build_mock_router(Arc::clone(&state));

    // Print startup info
    println!();
    success(&format!(
        "Mock server running at http://{}:{}",
        args.host, args.port
    ));
    println!();
    println!(
        "  {} Press {} to stop",
        "→".dimmed(),
        "Ctrl+C".bright_white()
    );
    println!();

    // Start the server using armature-core
    let addr = format!("{}:{}", args.host, args.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(CliError::Io)?;

    let router = Arc::new(router);

    // Simple HTTP server loop
    loop {
        let (stream, client_addr) = listener.accept().await.map_err(CliError::Io)?;
        let router = Arc::clone(&router);
        let state = Arc::clone(&state);

        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, router, state, client_addr).await {
                eprintln!("  {} Connection error: {}", "✗".red(), e);
            }
        });
    }
}

/// Handle a TCP connection
async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    _router: Arc<Router>,
    state: Arc<MockState>,
    _client_addr: std::net::SocketAddr,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    let (reader, mut writer) = stream.split();
    let mut reader = BufReader::new(reader);

    // Read request line
    let mut request_line = String::new();
    reader.read_line(&mut request_line).await?;

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(());
    }

    let method = parts[0].to_string();
    let path = parts[1].to_string();

    // Read headers
    let mut headers = HashMap::new();
    let mut content_length = 0usize;

    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        let line = line.trim();

        if line.is_empty() {
            break;
        }

        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_lowercase();
            let value = value.trim().to_string();

            if key == "content-length" {
                content_length = value.parse().unwrap_or(0);
            }

            headers.insert(key, value);
        }
    }

    // Read body if present
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).await?;
    }

    // Log request
    let method_colored = match method.as_str() {
        "GET" => "GET".green(),
        "POST" => "POST".yellow(),
        "PUT" => "PUT".blue(),
        "DELETE" => "DELETE".red(),
        "PATCH" => "PATCH".magenta(),
        "OPTIONS" => "OPTIONS".cyan(),
        _ => method.as_str().normal(),
    };

    println!(
        "  {} {} {}",
        "→".dimmed(),
        method_colored,
        path.bright_white()
    );

    // Apply delay if configured
    if state.delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(state.delay_ms)).await;
    }

    // Increment request count
    state
        .request_count
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    // Handle CORS preflight
    if method == "OPTIONS" && state.cors {
        let response = "HTTP/1.1 204 No Content\r\n\
            Access-Control-Allow-Origin: *\r\n\
            Access-Control-Allow-Methods: GET, POST, PUT, DELETE, PATCH, OPTIONS\r\n\
            Access-Control-Allow-Headers: Content-Type, Authorization\r\n\
            Content-Length: 0\r\n\
            \r\n";
        writer.write_all(response.as_bytes()).await?;
        return Ok(());
    }

    // Handle special mock endpoints
    if path == "/_mock/stats" {
        let count = state
            .request_count
            .load(std::sync::atomic::Ordering::Relaxed);
        let uptime = (Utc::now() - state.start_time).num_seconds();
        let body = serde_json::to_string_pretty(&json!({
            "requests": count,
            "uptime_seconds": uptime,
            "endpoints": count_all_operations(&state.spec),
        }))?;

        let response = format!(
            "HTTP/1.1 200 OK\r\n\
            Content-Type: application/json\r\n\
            {}\
            Content-Length: {}\r\n\
            \r\n{}",
            cors_headers(state.cors),
            body.len(),
            body
        );
        writer.write_all(response.as_bytes()).await?;
        return Ok(());
    }

    if path == "/_mock/spec" {
        let body = serde_yaml::to_string(&state.spec).unwrap_or_default();

        let response = format!(
            "HTTP/1.1 200 OK\r\n\
            Content-Type: text/yaml\r\n\
            {}\
            Content-Length: {}\r\n\
            \r\n{}",
            cors_headers(state.cors),
            body.len(),
            body
        );
        writer.write_all(response.as_bytes()).await?;
        return Ok(());
    }

    // Check custom data first
    let custom_key = format!("{} {}", method, path);
    if let Some(data) = state.custom_data.get(&custom_key) {
        let body = serde_json::to_string_pretty(data)?;

        let response = format!(
            "HTTP/1.1 200 OK\r\n\
            Content-Type: application/json\r\n\
            {}\
            Content-Length: {}\r\n\
            \r\n{}",
            cors_headers(state.cors),
            body.len(),
            body
        );
        writer.write_all(response.as_bytes()).await?;
        return Ok(());
    }

    // Find matching operation in spec
    let result = find_operation(&state.spec, &method, &path);

    let (status, body) = match result {
        Some((operation, path_params)) => {
            generate_mock_response(&state.spec, operation, &path_params, state.seed)
        }
        None => {
            let err_body = serde_json::to_string_pretty(&json!({
                "error": "Not Found",
                "message": format!("No mock endpoint for {} {}", method, path),
                "hint": "Check your OpenAPI spec or add custom mock data"
            }))?;
            (404, err_body)
        }
    };

    let status_text = match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };

    let response = format!(
        "HTTP/1.1 {} {}\r\n\
        Content-Type: application/json\r\n\
        {}\
        Content-Length: {}\r\n\
        \r\n{}",
        status,
        status_text,
        cors_headers(state.cors),
        body.len(),
        body
    );

    writer.write_all(response.as_bytes()).await?;
    Ok(())
}

/// Generate CORS headers string
fn cors_headers(enabled: bool) -> String {
    if enabled {
        "Access-Control-Allow-Origin: *\r\n".to_string()
    } else {
        String::new()
    }
}

/// Build a router with mock handlers (placeholder for future use)
fn build_mock_router(_state: Arc<MockState>) -> Router {
    Router::new()
}

/// Count operations in a path item
fn count_operations(path: &PathItem) -> usize {
    let mut count = 0;
    if path.get.is_some() {
        count += 1;
    }
    if path.post.is_some() {
        count += 1;
    }
    if path.put.is_some() {
        count += 1;
    }
    if path.delete.is_some() {
        count += 1;
    }
    if path.patch.is_some() {
        count += 1;
    }
    count
}

/// Count all operations in the spec
fn count_all_operations(spec: &OpenAPI) -> usize {
    spec.paths
        .paths
        .values()
        .map(|item| match item {
            ReferenceOr::Item(path) => count_operations(path),
            _ => 0,
        })
        .sum()
}

/// Find operation matching method and path
fn find_operation<'a>(
    spec: &'a OpenAPI,
    method: &str,
    path: &str,
) -> Option<(&'a Operation, HashMap<String, String>)> {
    for (spec_path, item) in &spec.paths.paths {
        let item = match item {
            ReferenceOr::Item(item) => item,
            _ => continue,
        };

        // Check if path matches (with parameter extraction)
        if let Some(params) = match_path(spec_path, path) {
            let operation = match method {
                "GET" => item.get.as_ref(),
                "POST" => item.post.as_ref(),
                "PUT" => item.put.as_ref(),
                "DELETE" => item.delete.as_ref(),
                "PATCH" => item.patch.as_ref(),
                _ => None,
            };

            if let Some(op) = operation {
                return Some((op, params));
            }
        }
    }
    None
}

/// Match a spec path against a request path, extracting parameters
fn match_path(spec_path: &str, request_path: &str) -> Option<HashMap<String, String>> {
    let spec_parts: Vec<&str> = spec_path.split('/').collect();
    let req_parts: Vec<&str> = request_path.split('/').collect();

    if spec_parts.len() != req_parts.len() {
        return None;
    }

    let mut params = HashMap::new();

    for (spec_part, req_part) in spec_parts.iter().zip(req_parts.iter()) {
        if spec_part.starts_with('{') && spec_part.ends_with('}') {
            // This is a parameter
            let param_name = &spec_part[1..spec_part.len() - 1];
            params.insert(param_name.to_string(), req_part.to_string());
        } else if spec_part != req_part {
            return None;
        }
    }

    Some(params)
}

/// Generate mock response for an operation
fn generate_mock_response(
    spec: &OpenAPI,
    operation: &Operation,
    _path_params: &HashMap<String, String>,
    seed: Option<u64>,
) -> (u16, String) {
    // Find successful response (200, 201, etc.)
    let response = operation
        .responses
        .responses
        .iter()
        .find(|(code, _)| {
            matches!(
                code,
                openapiv3::StatusCode::Code(200)
                    | openapiv3::StatusCode::Code(201)
                    | openapiv3::StatusCode::Code(204)
            )
        })
        .or_else(|| operation.responses.responses.iter().next());

    let (status_code, response) = match response {
        Some((code, resp)) => {
            let status = match code {
                openapiv3::StatusCode::Code(c) => *c,
                openapiv3::StatusCode::Range(_) => 200,
            };
            (status, resp)
        }
        None => {
            return (200, json!({"message": "OK"}).to_string());
        }
    };

    // Get response body
    let response = match response {
        ReferenceOr::Item(r) => r,
        ReferenceOr::Reference { reference } => {
            let ref_name = reference
                .strip_prefix("#/components/responses/")
                .unwrap_or(reference);
            if let Some(components) = &spec.components {
                if let Some(ReferenceOr::Item(r)) = components.responses.get(ref_name) {
                    r
                } else {
                    return (status_code, json!({"message": "OK"}).to_string());
                }
            } else {
                return (status_code, json!({"message": "OK"}).to_string());
            }
        }
    };

    // No content
    if status_code == 204 {
        return (204, String::new());
    }

    // Get schema from content
    let schema = response
        .content
        .get("application/json")
        .and_then(|media| media.schema.as_ref());

    match schema {
        Some(schema) => {
            let data = generate_mock_data(spec, schema, seed, 0);
            (
                status_code,
                serde_json::to_string_pretty(&data).unwrap_or_default(),
            )
        }
        None => (status_code, json!({"message": "OK"}).to_string()),
    }
}

/// Generate mock data from a schema
fn generate_mock_data(
    spec: &OpenAPI,
    schema_ref: &ReferenceOr<Schema>,
    seed: Option<u64>,
    depth: usize,
) -> Value {
    // Prevent infinite recursion
    if depth > 10 {
        return json!(null);
    }

    let schema = match schema_ref {
        ReferenceOr::Item(s) => s,
        ReferenceOr::Reference { reference } => {
            let ref_name = reference
                .strip_prefix("#/components/schemas/")
                .unwrap_or(reference);
            if let Some(components) = &spec.components {
                if let Some(ReferenceOr::Item(s)) = components.schemas.get(ref_name) {
                    s
                } else {
                    return json!(null);
                }
            } else {
                return json!(null);
            }
        }
    };

    // Check for example first
    if let Some(example) = &schema.schema_data.example {
        return example.clone();
    }

    match &schema.schema_kind {
        SchemaKind::Type(type_info) => match type_info {
            OapiType::String(string_type) => {
                // Check for format
                let format = match &string_type.format {
                    openapiv3::VariantOrUnknownOrEmpty::Item(f) => match f {
                        openapiv3::StringFormat::DateTime => "date-time",
                        openapiv3::StringFormat::Date => "date",
                        openapiv3::StringFormat::Password => "password",
                        openapiv3::StringFormat::Byte => "byte",
                        openapiv3::StringFormat::Binary => "binary",
                    },
                    openapiv3::VariantOrUnknownOrEmpty::Unknown(s) => s.as_str(),
                    openapiv3::VariantOrUnknownOrEmpty::Empty => "",
                };

                match format {
                    "date-time" => json!(Utc::now().to_rfc3339()),
                    "date" => json!(Utc::now().format("%Y-%m-%d").to_string()),
                    "email" => json!("user@example.com"),
                    "uri" | "url" => json!("https://example.com"),
                    "uuid" => json!("550e8400-e29b-41d4-a716-446655440000"),
                    "hostname" => json!("example.com"),
                    "ipv4" => json!("192.168.1.1"),
                    "ipv6" => json!("::1"),
                    _ => {
                        // Check for enum
                        if !string_type.enumeration.is_empty() {
                            let idx = seed.unwrap_or(0) as usize % string_type.enumeration.len();
                            if let Some(Some(val)) = string_type.enumeration.get(idx) {
                                return json!(val);
                            }
                        }
                        json!("string")
                    }
                }
            }
            OapiType::Number(_) => json!(42.5),
            OapiType::Integer(_) => json!(42),
            OapiType::Boolean(_) => json!(true),
            OapiType::Object(obj_type) => {
                let mut obj = serde_json::Map::new();
                for (prop_name, prop_schema) in &obj_type.properties {
                    let unboxed = match prop_schema {
                        ReferenceOr::Item(boxed) => ReferenceOr::Item(boxed.as_ref().clone()),
                        ReferenceOr::Reference { reference } => ReferenceOr::Reference {
                            reference: reference.clone(),
                        },
                    };
                    let value = generate_mock_data(spec, &unboxed, seed, depth + 1);
                    obj.insert(prop_name.clone(), value);
                }
                json!(obj)
            }
            OapiType::Array(array_type) => {
                if let Some(items) = &array_type.items {
                    let unboxed = match items {
                        ReferenceOr::Item(boxed) => ReferenceOr::Item(boxed.as_ref().clone()),
                        ReferenceOr::Reference { reference } => ReferenceOr::Reference {
                            reference: reference.clone(),
                        },
                    };
                    let item = generate_mock_data(spec, &unboxed, seed, depth + 1);
                    // Return array with 1-3 items
                    let count = (seed.unwrap_or(2) % 3 + 1) as usize;
                    json!(vec![item; count])
                } else {
                    json!([])
                }
            }
        },
        SchemaKind::OneOf { one_of } | SchemaKind::AnyOf { any_of: one_of } => {
            if let Some(first) = one_of.first() {
                generate_mock_data(spec, first, seed, depth + 1)
            } else {
                json!(null)
            }
        }
        SchemaKind::AllOf { all_of } => {
            let mut obj = serde_json::Map::new();
            for schema in all_of {
                if let Value::Object(map) = generate_mock_data(spec, schema, seed, depth + 1) {
                    obj.extend(map);
                }
            }
            json!(obj)
        }
        SchemaKind::Not { .. } => json!(null),
        SchemaKind::Any(_) => json!({}),
    }
}

/// Load OpenAPI spec from file
fn load_openapi_spec(path: &str) -> CliResult<OpenAPI> {
    let content = fs::read_to_string(path).map_err(CliError::Io)?;

    let spec: OpenAPI = if path.ends_with(".yaml") || path.ends_with(".yml") {
        serde_yaml::from_str(&content)
            .map_err(|e| CliError::Config(format!("Invalid YAML: {}", e)))?
    } else {
        serde_json::from_str(&content)
            .map_err(|e| CliError::Config(format!("Invalid JSON: {}", e)))?
    };

    Ok(spec)
}

/// Load custom mock data from directory
fn load_custom_data(dir: &str) -> CliResult<HashMap<String, Value>> {
    let mut data = HashMap::new();
    let path = Path::new(dir);

    if !path.exists() {
        return Ok(data);
    }

    for entry in fs::read_dir(path).map_err(CliError::Io)? {
        let entry = entry.map_err(CliError::Io)?;
        let file_path = entry.path();

        if file_path.extension().map(|e| e == "json").unwrap_or(false) {
            let content = fs::read_to_string(&file_path).map_err(CliError::Io)?;
            let value: Value = serde_json::from_str(&content)
                .map_err(|e| CliError::Config(format!("Invalid JSON in {:?}: {}", file_path, e)))?;

            // File name format: METHOD_path_parts.json (e.g., GET_users.json, POST_users_123.json)
            if let Some(stem) = file_path.file_stem().and_then(|s| s.to_str()) {
                let parts: Vec<&str> = stem.splitn(2, '_').collect();
                if parts.len() == 2 {
                    let method = parts[0];
                    let path = format!("/{}", parts[1].replace('_', "/"));
                    let key = format!("{} {}", method, path);
                    data.insert(key, value);
                }
            }
        }
    }

    if !data.is_empty() {
        info(&format!("Loaded {} custom mock data files", data.len()));
    }

    Ok(data)
}

/// Print available routes
fn print_routes(spec: &OpenAPI) {
    for (path, item) in &spec.paths.paths {
        let item = match item {
            ReferenceOr::Item(item) => item,
            _ => continue,
        };

        let operations = [
            (item.get.as_ref(), "GET"),
            (item.post.as_ref(), "POST"),
            (item.put.as_ref(), "PUT"),
            (item.delete.as_ref(), "DELETE"),
            (item.patch.as_ref(), "PATCH"),
        ];

        for (op, method) in operations {
            if let Some(operation) = op {
                let method_colored = match method {
                    "GET" => "GET".green(),
                    "POST" => "POST".yellow(),
                    "PUT" => "PUT".blue(),
                    "DELETE" => "DELETE".red(),
                    "PATCH" => "PATCH".magenta(),
                    _ => method.normal(),
                };

                let summary = operation
                    .summary
                    .as_deref()
                    .or(operation.operation_id.as_deref())
                    .unwrap_or("");

                println!(
                    "    {} {} {}",
                    method_colored,
                    path.bright_white(),
                    summary.dimmed()
                );
            }
        }
    }

    // Print special endpoints
    println!();
    println!("  {}", "Special endpoints:".dimmed());
    println!(
        "    {} {} {}",
        "GET".green(),
        "/_mock/stats".bright_white(),
        "Server statistics".dimmed()
    );
    println!(
        "    {} {} {}",
        "GET".green(),
        "/_mock/spec".bright_white(),
        "OpenAPI spec".dimmed()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_path() {
        let params = match_path("/users/{id}", "/users/123");
        assert!(params.is_some());
        assert_eq!(params.unwrap().get("id"), Some(&"123".to_string()));
    }

    #[test]
    fn test_match_path_no_match() {
        let params = match_path("/users/{id}", "/posts/123");
        assert!(params.is_none());
    }

    #[test]
    fn test_match_path_multiple_params() {
        let params = match_path("/users/{userId}/posts/{postId}", "/users/1/posts/2");
        assert!(params.is_some());
        let params = params.unwrap();
        assert_eq!(params.get("userId"), Some(&"1".to_string()));
        assert_eq!(params.get("postId"), Some(&"2".to_string()));
    }
}
