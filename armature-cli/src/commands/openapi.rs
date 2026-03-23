//! OpenAPI client generation command.
//!
//! Generates TypeScript and/or Rust HTTP clients from OpenAPI 3.x specifications.

use crate::error::{CliError, CliResult};
use colored::Colorize;
use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};
use openapiv3::{
    OpenAPI, Operation, Parameter, ParameterSchemaOrContent, PathItem, ReferenceOr, Schema,
    SchemaKind, Type as OapiType,
};
use std::fs;
use std::path::Path;

/// Generate client code from an OpenAPI spec
pub async fn generate_client(
    spec_path: &str,
    output_dir: &str,
    language: ClientLanguage,
    options: &ClientOptions,
) -> CliResult<()> {
    println!(
        "  {} Generating {} client from {}",
        "→".cyan(),
        language.name().bright_white(),
        spec_path.cyan()
    );

    // Load the OpenAPI spec
    let spec = load_spec(spec_path)?;

    // Generate based on language
    match language {
        ClientLanguage::TypeScript => generate_typescript_client(&spec, output_dir, options)?,
        ClientLanguage::Rust => generate_rust_client(&spec, output_dir, options)?,
        ClientLanguage::Both => {
            let ts_dir = format!("{}/typescript", output_dir);
            let rs_dir = format!("{}/rust", output_dir);
            generate_typescript_client(&spec, &ts_dir, options)?;
            generate_rust_client(&spec, &rs_dir, options)?;
        }
    }

    println!(
        "  {} Client generated in {}",
        "✓".green().bold(),
        output_dir.cyan()
    );

    Ok(())
}

/// Client target language
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientLanguage {
    TypeScript,
    Rust,
    Both,
}

impl ClientLanguage {
    pub fn name(&self) -> &'static str {
        match self {
            Self::TypeScript => "TypeScript",
            Self::Rust => "Rust",
            Self::Both => "TypeScript + Rust",
        }
    }
}

/// Client generation options
#[derive(Debug, Clone, Default)]
pub struct ClientOptions {
    /// Base URL for the API (can be overridden at runtime)
    pub base_url: Option<String>,
    /// Generate async client (Rust only)
    pub async_client: bool,
    /// Include request/response logging
    pub with_logging: bool,
    /// Generate with retry logic
    pub with_retry: bool,
    /// Custom HTTP client name
    pub client_name: Option<String>,
}

// =============================================================================
// Spec Loading
// =============================================================================

fn load_spec(path: &str) -> CliResult<OpenAPI> {
    let content = if path.starts_with("http://") || path.starts_with("https://") {
        // TODO: fetch from URL
        return Err(CliError::Command(
            "URL spec fetching not yet implemented. Download the spec locally.".to_string(),
        ));
    } else {
        fs::read_to_string(path).map_err(|e| {
            CliError::Command(format!("Failed to read OpenAPI spec '{}': {}", path, e))
        })?
    };

    // Try JSON first, then YAML
    if path.ends_with(".json") {
        serde_json::from_str(&content)
            .map_err(|e| CliError::Command(format!("Failed to parse OpenAPI JSON: {}", e)))
    } else {
        serde_yaml::from_str(&content)
            .map_err(|e| CliError::Command(format!("Failed to parse OpenAPI YAML: {}", e)))
    }
}

// =============================================================================
// TypeScript Client Generation
// =============================================================================

fn generate_typescript_client(
    spec: &OpenAPI,
    output_dir: &str,
    options: &ClientOptions,
) -> CliResult<()> {
    fs::create_dir_all(output_dir)
        .map_err(|e| CliError::Command(format!("Failed to create output directory: {}", e)))?;

    let api_name = options
        .client_name
        .clone()
        .unwrap_or_else(|| spec.info.title.to_pascal_case());

    // Collect all schemas for type generation
    let mut types = String::new();
    let mut operations = Vec::new();

    // Generate types from schemas
    if let Some(components) = &spec.components {
        types.push_str(
            "// =============================================================================\n",
        );
        types.push_str("// Types\n");
        types.push_str(
            "// =============================================================================\n\n",
        );

        for (name, schema) in &components.schemas {
            if let ReferenceOr::Item(schema) = schema {
                types.push_str(&generate_ts_type(name, schema));
                types.push('\n');
            }
        }
    }

    // Generate operations from paths
    for (path, path_item) in &spec.paths.paths {
        if let ReferenceOr::Item(item) = path_item {
            collect_operations(path, item, &mut operations);
        }
    }

    // Build the client class
    let mut client = String::new();

    // File header
    client.push_str(&format!(
        r#"/**
 * {} API Client
 *
 * Auto-generated from OpenAPI specification
 * Generated by: armature openapi:client
 * Spec version: {}
 *
 * DO NOT EDIT - This file is auto-generated
 */

"#,
        api_name, spec.info.version
    ));

    // Types
    client.push_str(&types);

    // Client configuration
    client.push_str(
        r#"// =============================================================================
// Client Configuration
// =============================================================================

export interface ClientConfig {
  baseUrl: string;
  headers?: Record<string, string>;
  timeout?: number;
  onError?: (error: ApiError) => void;
}

export interface ApiError {
  status: number;
  message: string;
  body?: unknown;
}

export interface RequestOptions {
  headers?: Record<string, string>;
  signal?: AbortSignal;
}

"#,
    );

    // Client class
    client.push_str(&format!(
        r#"// =============================================================================
// {} Client
// =============================================================================

export class {}Client {{
  private baseUrl: string;
  private headers: Record<string, string>;
  private timeout: number;
  private onError?: (error: ApiError) => void;

  constructor(config: ClientConfig) {{
    this.baseUrl = config.baseUrl.replace(/\/$/, '');
    this.headers = config.headers ?? {{}};
    this.timeout = config.timeout ?? 30000;
    this.onError = config.onError;
  }}

  private async request<T>(
    method: string,
    path: string,
    options?: RequestOptions & {{ body?: unknown; query?: Record<string, string> }}
  ): Promise<T> {{
    const url = new URL(path, this.baseUrl);

    if (options?.query) {{
      for (const [key, value] of Object.entries(options.query)) {{
        if (value !== undefined && value !== null) {{
          url.searchParams.set(key, value);
        }}
      }}
    }}

    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), this.timeout);

    try {{
      const response = await fetch(url.toString(), {{
        method,
        headers: {{
          'Content-Type': 'application/json',
          ...this.headers,
          ...options?.headers,
        }},
        body: options?.body ? JSON.stringify(options.body) : undefined,
        signal: options?.signal ?? controller.signal,
      }});

      clearTimeout(timeoutId);

      if (!response.ok) {{
        const error: ApiError = {{
          status: response.status,
          message: response.statusText,
          body: await response.json().catch(() => undefined),
        }};
        this.onError?.(error);
        throw error;
      }}

      if (response.status === 204) {{
        return undefined as T;
      }}

      return response.json();
    }} catch (error) {{
      clearTimeout(timeoutId);
      throw error;
    }}
  }}

"#,
        api_name, api_name
    ));

    // Generate methods for each operation
    for op in &operations {
        client.push_str(&generate_ts_method(op));
    }

    client.push_str("}\n\n");

    // Default export
    client.push_str(&format!(
        r#"// Default client instance factory
export function create{}Client(config: ClientConfig): {}Client {{
  return new {}Client(config);
}}
"#,
        api_name, api_name, api_name
    ));

    // Write the client file
    let client_path = Path::new(output_dir).join("client.ts");
    fs::write(&client_path, client)
        .map_err(|e| CliError::Command(format!("Failed to write TypeScript client: {}", e)))?;

    println!(
        "  {} Created {}",
        "✓".green(),
        client_path.display().to_string().cyan()
    );

    // Generate index.ts
    let index = format!(
        r#"/**
 * {} API Client
 *
 * @example
 * ```typescript
 * import {{ create{}Client }} from './client';
 *
 * const api = create{}Client({{
 *   baseUrl: 'https://api.example.com',
 *   headers: {{ 'Authorization': 'Bearer token' }}
 * }});
 *
 * const users = await api.getUsers();
 * ```
 */

export * from './client';
"#,
        api_name, api_name, api_name
    );

    let index_path = Path::new(output_dir).join("index.ts");
    fs::write(&index_path, index)
        .map_err(|e| CliError::Command(format!("Failed to write index.ts: {}", e)))?;

    println!(
        "  {} Created {}",
        "✓".green(),
        index_path.display().to_string().cyan()
    );

    Ok(())
}

fn generate_ts_type(name: &str, schema: &Schema) -> String {
    let mut output = String::new();

    // Add description if available
    if let Some(desc) = &schema.schema_data.description {
        output.push_str(&format!("/** {} */\n", desc));
    }

    match &schema.schema_kind {
        SchemaKind::Type(OapiType::Object(obj)) => {
            output.push_str(&format!("export interface {} {{\n", name.to_pascal_case()));

            for (prop_name, prop_schema) in &obj.properties {
                let required = obj.required.contains(prop_name);
                let ts_type = boxed_schema_to_ts_type(prop_schema);
                let optional = if required { "" } else { "?" };

                output.push_str(&format!(
                    "  {}{}: {};\n",
                    prop_name.to_lower_camel_case(),
                    optional,
                    ts_type
                ));
            }

            output.push_str("}\n");
        }
        SchemaKind::Type(OapiType::String(s)) if !s.enumeration.is_empty() => {
            // Enum type
            let variants: Vec<String> = s
                .enumeration
                .iter()
                .filter_map(|v| v.as_ref().map(|s| format!("\"{}\"", s)))
                .collect();
            output.push_str(&format!(
                "export type {} = {};\n",
                name.to_pascal_case(),
                variants.join(" | ")
            ));
        }
        _ => {
            // Simple type alias
            let ts_type = schema_kind_to_ts_type(&schema.schema_kind);
            output.push_str(&format!(
                "export type {} = {};\n",
                name.to_pascal_case(),
                ts_type
            ));
        }
    }

    output
}

fn schema_to_ts_type(schema: &ReferenceOr<Schema>) -> String {
    match schema {
        ReferenceOr::Reference { reference } => {
            // Extract type name from #/components/schemas/TypeName
            reference
                .rsplit('/')
                .next()
                .map(|s| s.to_pascal_case())
                .unwrap_or_else(|| "unknown".to_string())
        }
        ReferenceOr::Item(schema) => schema_kind_to_ts_type(&schema.schema_kind),
    }
}

fn boxed_schema_to_ts_type(schema: &ReferenceOr<Box<Schema>>) -> String {
    match schema {
        ReferenceOr::Reference { reference } => reference
            .rsplit('/')
            .next()
            .map(|s| s.to_pascal_case())
            .unwrap_or_else(|| "unknown".to_string()),
        ReferenceOr::Item(schema) => schema_kind_to_ts_type(&schema.schema_kind),
    }
}

fn schema_kind_to_ts_type(kind: &SchemaKind) -> String {
    match kind {
        SchemaKind::Type(t) => match t {
            OapiType::String(_) => "string".to_string(),
            OapiType::Number(_) => "number".to_string(),
            OapiType::Integer(_) => "number".to_string(),
            OapiType::Boolean(_) => "boolean".to_string(),
            OapiType::Array(arr) => {
                let item_type = arr
                    .items
                    .as_ref()
                    .map(boxed_schema_to_ts_type)
                    .unwrap_or_else(|| "unknown".to_string());
                format!("{}[]", item_type)
            }
            OapiType::Object(_) => "Record<string, unknown>".to_string(),
        },
        SchemaKind::OneOf { .. } => "unknown".to_string(),
        SchemaKind::AllOf { .. } => "unknown".to_string(),
        SchemaKind::AnyOf { .. } => "unknown".to_string(),
        SchemaKind::Not { .. } => "unknown".to_string(),
        SchemaKind::Any(_) => "unknown".to_string(),
    }
}

/// Collected operation info
struct OperationInfo {
    method: String,
    path: String,
    operation_id: Option<String>,
    summary: Option<String>,
    parameters: Vec<ParameterInfo>,
    request_body: Option<String>,
    response_type: String,
}

struct ParameterInfo {
    name: String,
    location: String, // path, query, header
    required: bool,
    param_type: String,
}

fn collect_operations(path: &str, item: &PathItem, operations: &mut Vec<OperationInfo>) {
    let methods = [
        ("get", &item.get),
        ("post", &item.post),
        ("put", &item.put),
        ("patch", &item.patch),
        ("delete", &item.delete),
    ];

    for (method, op) in methods {
        if let Some(operation) = op {
            operations.push(parse_operation(method, path, operation));
        }
    }
}

fn parse_operation(method: &str, path: &str, op: &Operation) -> OperationInfo {
    let mut parameters = Vec::new();

    for param_ref in &op.parameters {
        if let ReferenceOr::Item(param) = param_ref {
            let param_type = match &param.parameter_data_ref().format {
                ParameterSchemaOrContent::Schema(s) => schema_to_ts_type(s),
                _ => "string".to_string(),
            };

            parameters.push(ParameterInfo {
                name: param.parameter_data_ref().name.clone(),
                location: match param {
                    Parameter::Path { .. } => "path".to_string(),
                    Parameter::Query { .. } => "query".to_string(),
                    Parameter::Header { .. } => "header".to_string(),
                    Parameter::Cookie { .. } => "cookie".to_string(),
                },
                required: param.parameter_data_ref().required,
                param_type,
            });
        }
    }

    // Get request body type
    let request_body = op.request_body.as_ref().and_then(|rb| {
        if let ReferenceOr::Item(body) = rb {
            body.content
                .get("application/json")
                .and_then(|mt| mt.schema.as_ref().map(schema_to_ts_type))
        } else {
            None
        }
    });

    // Get response type (from 200 or 201 response)
    let response_type = op
        .responses
        .responses
        .get(&openapiv3::StatusCode::Code(200))
        .or_else(|| {
            op.responses
                .responses
                .get(&openapiv3::StatusCode::Code(201))
        })
        .and_then(|r| {
            if let ReferenceOr::Item(response) = r {
                response
                    .content
                    .get("application/json")
                    .and_then(|mt| mt.schema.as_ref().map(schema_to_ts_type))
            } else {
                None
            }
        })
        .unwrap_or_else(|| "void".to_string());

    OperationInfo {
        method: method.to_string(),
        path: path.to_string(),
        operation_id: op.operation_id.clone(),
        summary: op.summary.clone(),
        parameters,
        request_body,
        response_type,
    }
}

fn generate_ts_method(op: &OperationInfo) -> String {
    let method_name = op
        .operation_id
        .clone()
        .unwrap_or_else(|| format!("{}_{}", op.method, op.path.replace('/', "_")))
        .to_lower_camel_case();

    let mut params = Vec::new();
    let mut path_params = Vec::new();
    let mut query_params = Vec::new();

    for param in &op.parameters {
        let optional = if param.required { "" } else { "?" };
        params.push(format!(
            "{}{}: {}",
            param.name.to_lower_camel_case(),
            optional,
            param.param_type
        ));

        if param.location == "path" {
            path_params.push(param.name.clone());
        } else if param.location == "query" {
            query_params.push(param.name.clone());
        }
    }

    // Add body parameter if present
    if let Some(ref body_type) = op.request_body {
        params.push(format!("body: {}", body_type));
    }

    // Add options parameter
    params.push("options?: RequestOptions".to_string());

    let params_str = params.join(", ");

    // Build the path with interpolation
    let mut path = op.path.clone();
    for param in &path_params {
        path = path.replace(
            &format!("{{{}}}", param),
            &format!("${{{}}}", param.to_lower_camel_case()),
        );
    }

    // Build query object
    let query_obj = if query_params.is_empty() {
        "".to_string()
    } else {
        let entries: Vec<String> = query_params
            .iter()
            .map(|p| {
                format!(
                    "{}: String({})",
                    p.to_lower_camel_case(),
                    p.to_lower_camel_case()
                )
            })
            .collect();
        format!(", query: {{ {} }}", entries.join(", "))
    };

    // Build body option
    let body_opt = if op.request_body.is_some() {
        ", body"
    } else {
        ""
    };

    let mut output = String::new();

    // Add JSDoc comment
    if let Some(ref summary) = op.summary {
        output.push_str(&format!("  /** {} */\n", summary));
    }

    output.push_str(&format!(
        r#"  async {}({}): Promise<{}> {{
    return this.request<{}>('{}', `{}`{}{}, {{ ...options }});
  }}

"#,
        method_name,
        params_str,
        op.response_type,
        op.response_type,
        op.method.to_uppercase(),
        path,
        body_opt,
        query_obj
    ));

    output
}

// =============================================================================
// Rust Client Generation
// =============================================================================

fn generate_rust_client(
    spec: &OpenAPI,
    output_dir: &str,
    options: &ClientOptions,
) -> CliResult<()> {
    fs::create_dir_all(output_dir)
        .map_err(|e| CliError::Command(format!("Failed to create output directory: {}", e)))?;

    let api_name = options
        .client_name
        .clone()
        .unwrap_or_else(|| spec.info.title.to_pascal_case());

    let module_name = api_name.to_snake_case();

    // Collect operations
    let mut operations = Vec::new();
    for (path, path_item) in &spec.paths.paths {
        if let ReferenceOr::Item(item) = path_item {
            collect_rust_operations(path, item, &mut operations);
        }
    }

    // Generate types module
    let mut types = String::new();
    types.push_str("//! API types\n\n");
    types.push_str("use serde::{Deserialize, Serialize};\n\n");

    if let Some(components) = &spec.components {
        for (name, schema) in &components.schemas {
            if let ReferenceOr::Item(schema) = schema {
                types.push_str(&generate_rust_type(name, schema));
                types.push('\n');
            }
        }
    }

    let types_path = Path::new(output_dir).join("types.rs");
    fs::write(&types_path, types)
        .map_err(|e| CliError::Command(format!("Failed to write types.rs: {}", e)))?;

    println!(
        "  {} Created {}",
        "✓".green(),
        types_path.display().to_string().cyan()
    );

    // Generate client module
    let mut client = String::new();

    client.push_str(&format!(
        r#"//! {} API Client
//!
//! Auto-generated from OpenAPI specification
//! Spec version: {}
//!
//! # Example
//!
//! ```rust,ignore
//! use {}::{{{}Client, ClientConfig}};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {{
//!     let client = {}Client::new(ClientConfig {{
//!         base_url: "https://api.example.com".to_string(),
//!         ..Default::default()
//!     }})?;
//!
//!     // Use the client...
//!     Ok(())
//! }}
//! ```

use reqwest::{{Client, header::HeaderMap}};
use serde::{{Deserialize, Serialize}};
use std::time::Duration;

mod types;
pub use types::*;

/// Client configuration
#[derive(Debug, Clone)]
pub struct ClientConfig {{
    /// Base URL for the API
    pub base_url: String,
    /// Custom headers to include in all requests
    pub headers: Option<HeaderMap>,
    /// Request timeout (default: 30s)
    pub timeout: Duration,
    /// User agent string
    pub user_agent: Option<String>,
}}

impl Default for ClientConfig {{
    fn default() -> Self {{
        Self {{
            base_url: String::new(),
            headers: None,
            timeout: Duration::from_secs(30),
            user_agent: Some("armature-client/1.0".to_string()),
        }}
    }}
}}

/// API error type
#[derive(Debug, thiserror::Error)]
pub enum ApiError {{
    #[error("HTTP error: {{0}}")]
    Http(#[from] reqwest::Error),

    #[error("API error ({{status}}): {{message}}")]
    Api {{ status: u16, message: String }},

    #[error("Invalid configuration: {{0}}")]
    Config(String),
}}

/// {} API client
pub struct {}Client {{
    client: Client,
    base_url: String,
}}

impl {}Client {{
    /// Create a new API client
    pub fn new(config: ClientConfig) -> Result<Self, ApiError> {{
        if config.base_url.is_empty() {{
            return Err(ApiError::Config("base_url is required".to_string()));
        }}

        let mut builder = Client::builder()
            .timeout(config.timeout);

        if let Some(ref ua) = config.user_agent {{
            builder = builder.user_agent(ua);
        }}

        if let Some(headers) = config.headers {{
            builder = builder.default_headers(headers);
        }}

        let client = builder.build()?;
        let base_url = config.base_url.trim_end_matches('/').to_string();

        Ok(Self {{ client, base_url }})
    }}

"#,
        api_name, spec.info.version, module_name, api_name, api_name, api_name, api_name, api_name
    ));

    // Generate methods
    for op in &operations {
        client.push_str(&generate_rust_method(op));
    }

    client.push_str("}\n");

    let client_path = Path::new(output_dir).join("client.rs");
    fs::write(&client_path, &client)
        .map_err(|e| CliError::Command(format!("Failed to write client.rs: {}", e)))?;

    println!(
        "  {} Created {}",
        "✓".green(),
        client_path.display().to_string().cyan()
    );

    // Generate lib.rs
    let lib_rs = format!(
        r#"//! {} API Client
//!
//! Auto-generated by `armature openapi:client`

mod client;
mod types;

pub use client::*;
pub use types::*;
"#,
        api_name
    );

    let lib_path = Path::new(output_dir).join("lib.rs");
    fs::write(&lib_path, lib_rs)
        .map_err(|e| CliError::Command(format!("Failed to write lib.rs: {}", e)))?;

    // Generate Cargo.toml
    let cargo_toml = format!(
        r#"[package]
name = "{}-client"
version = "0.1.0"
edition = "2021"
description = "API client for {}"

[dependencies]
reqwest = {{ version = "0.12", features = ["json"] }}
serde = {{ version = "1.0", features = ["derive"] }}
serde_json = "1.0"
thiserror = "2.0"
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
"#,
        module_name, api_name
    );

    let cargo_path = Path::new(output_dir).join("Cargo.toml");
    fs::write(&cargo_path, cargo_toml)
        .map_err(|e| CliError::Command(format!("Failed to write Cargo.toml: {}", e)))?;

    println!(
        "  {} Created {}",
        "✓".green(),
        cargo_path.display().to_string().cyan()
    );

    Ok(())
}

fn generate_rust_type(name: &str, schema: &Schema) -> String {
    let mut output = String::new();

    // Add doc comment
    if let Some(desc) = &schema.schema_data.description {
        output.push_str(&format!("/// {}\n", desc));
    }

    match &schema.schema_kind {
        SchemaKind::Type(OapiType::Object(obj)) => {
            output.push_str("#[derive(Debug, Clone, Serialize, Deserialize)]\n");
            output.push_str(&format!("pub struct {} {{\n", name.to_pascal_case()));

            for (prop_name, prop_schema) in &obj.properties {
                let rust_type = boxed_schema_to_rust_type(prop_schema);
                let required = obj.required.contains(prop_name);

                let field_type = if required {
                    rust_type
                } else {
                    format!("Option<{}>", rust_type)
                };

                // Use serde rename if needed
                let serde_attr = if prop_name != &prop_name.to_snake_case() {
                    format!("    #[serde(rename = \"{}\")]\n", prop_name)
                } else {
                    String::new()
                };

                output.push_str(&serde_attr);
                output.push_str(&format!(
                    "    pub {}: {},\n",
                    prop_name.to_snake_case(),
                    field_type
                ));
            }

            output.push_str("}\n");
        }
        SchemaKind::Type(OapiType::String(s)) if !s.enumeration.is_empty() => {
            output.push_str("#[derive(Debug, Clone, Serialize, Deserialize)]\n");
            output.push_str(&format!("pub enum {} {{\n", name.to_pascal_case()));

            for v in s.enumeration.iter().flatten() {
                let variant_name = v.to_pascal_case();
                if variant_name != *v {
                    output.push_str(&format!("    #[serde(rename = \"{}\")]\n", v));
                }
                output.push_str(&format!("    {},\n", variant_name));
            }

            output.push_str("}\n");
        }
        _ => {
            // Type alias
            let rust_type = schema_kind_to_rust_type(&schema.schema_kind);
            output.push_str(&format!(
                "pub type {} = {};\n",
                name.to_pascal_case(),
                rust_type
            ));
        }
    }

    output
}

fn schema_to_rust_type(schema: &ReferenceOr<Schema>) -> String {
    match schema {
        ReferenceOr::Reference { reference } => reference
            .rsplit('/')
            .next()
            .map(|s| s.to_pascal_case())
            .unwrap_or_else(|| "serde_json::Value".to_string()),
        ReferenceOr::Item(schema) => schema_kind_to_rust_type(&schema.schema_kind),
    }
}

fn boxed_schema_to_rust_type(schema: &ReferenceOr<Box<Schema>>) -> String {
    match schema {
        ReferenceOr::Reference { reference } => reference
            .rsplit('/')
            .next()
            .map(|s| s.to_pascal_case())
            .unwrap_or_else(|| "serde_json::Value".to_string()),
        ReferenceOr::Item(schema) => schema_kind_to_rust_type(&schema.schema_kind),
    }
}

fn schema_kind_to_rust_type(kind: &SchemaKind) -> String {
    match kind {
        SchemaKind::Type(t) => match t {
            OapiType::String(_) => "String".to_string(),
            OapiType::Number(_) => "f64".to_string(),
            OapiType::Integer(_) => "i64".to_string(),
            OapiType::Boolean(_) => "bool".to_string(),
            OapiType::Array(arr) => {
                let item_type = arr
                    .items
                    .as_ref()
                    .map(boxed_schema_to_rust_type)
                    .unwrap_or_else(|| "serde_json::Value".to_string());
                format!("Vec<{}>", item_type)
            }
            OapiType::Object(_) => "serde_json::Value".to_string(),
        },
        _ => "serde_json::Value".to_string(),
    }
}

struct RustOperationInfo {
    method: String,
    path: String,
    operation_id: Option<String>,
    summary: Option<String>,
    parameters: Vec<RustParameterInfo>,
    request_body: Option<String>,
    response_type: String,
}

struct RustParameterInfo {
    name: String,
    location: String,
    required: bool,
    param_type: String,
}

fn collect_rust_operations(path: &str, item: &PathItem, operations: &mut Vec<RustOperationInfo>) {
    let methods = [
        ("get", &item.get),
        ("post", &item.post),
        ("put", &item.put),
        ("patch", &item.patch),
        ("delete", &item.delete),
    ];

    for (method, op) in methods {
        if let Some(operation) = op {
            operations.push(parse_rust_operation(method, path, operation));
        }
    }
}

fn parse_rust_operation(method: &str, path: &str, op: &Operation) -> RustOperationInfo {
    let mut parameters = Vec::new();

    for param_ref in &op.parameters {
        if let ReferenceOr::Item(param) = param_ref {
            let param_type = match &param.parameter_data_ref().format {
                ParameterSchemaOrContent::Schema(s) => schema_to_rust_type(s),
                _ => "String".to_string(),
            };

            parameters.push(RustParameterInfo {
                name: param.parameter_data_ref().name.clone(),
                location: match param {
                    Parameter::Path { .. } => "path".to_string(),
                    Parameter::Query { .. } => "query".to_string(),
                    Parameter::Header { .. } => "header".to_string(),
                    Parameter::Cookie { .. } => "cookie".to_string(),
                },
                required: param.parameter_data_ref().required,
                param_type,
            });
        }
    }

    let request_body = op.request_body.as_ref().and_then(|rb| {
        if let ReferenceOr::Item(body) = rb {
            body.content
                .get("application/json")
                .and_then(|mt| mt.schema.as_ref().map(schema_to_rust_type))
        } else {
            None
        }
    });

    let response_type = op
        .responses
        .responses
        .get(&openapiv3::StatusCode::Code(200))
        .or_else(|| {
            op.responses
                .responses
                .get(&openapiv3::StatusCode::Code(201))
        })
        .and_then(|r| {
            if let ReferenceOr::Item(response) = r {
                response
                    .content
                    .get("application/json")
                    .and_then(|mt| mt.schema.as_ref().map(schema_to_rust_type))
            } else {
                None
            }
        })
        .unwrap_or_else(|| "()".to_string());

    RustOperationInfo {
        method: method.to_string(),
        path: path.to_string(),
        operation_id: op.operation_id.clone(),
        summary: op.summary.clone(),
        parameters,
        request_body,
        response_type,
    }
}

fn generate_rust_method(op: &RustOperationInfo) -> String {
    let method_name = op
        .operation_id
        .clone()
        .unwrap_or_else(|| format!("{}_{}", op.method, op.path.replace('/', "_")))
        .to_snake_case();

    let mut params = Vec::new();
    let mut path_replacements = Vec::new();
    let mut query_params = Vec::new();

    for param in &op.parameters {
        let rust_type = if param.required {
            if param.param_type == "String" {
                "&str".to_string()
            } else {
                param.param_type.clone()
            }
        } else {
            format!("Option<{}>", param.param_type)
        };

        params.push(format!("{}: {}", param.name.to_snake_case(), rust_type));

        if param.location == "path" {
            path_replacements.push((param.name.clone(), param.name.to_snake_case()));
        } else if param.location == "query" {
            query_params.push((param.name.clone(), param.name.to_snake_case()));
        }
    }

    if let Some(ref body_type) = op.request_body {
        params.push(format!("body: &{}", body_type));
    }

    let params_str = if params.is_empty() {
        "&self".to_string()
    } else {
        format!("&self, {}", params.join(", "))
    };

    // Build path string
    let mut path_code = format!("let path = \"{}\".to_string()", op.path);
    for (orig, snake) in &path_replacements {
        path_code.push_str(&format!(".replace(\"{{{}}}\", {})", orig, snake));
    }
    path_code.push(';');

    // Build request
    let method_call = format!(
        "self.client.{}(&format!(\"{{}}{{}}\", self.base_url, path))",
        op.method
    );

    let mut request_build = method_call;

    // Add query params
    for (orig, snake) in &query_params {
        request_build.push_str(&format!(
            "\n            .query(&[(\"{}\", {})])",
            orig, snake
        ));
    }

    // Add body
    if op.request_body.is_some() {
        request_build.push_str("\n            .json(body)");
    }

    let mut output = String::new();

    if let Some(ref summary) = op.summary {
        output.push_str(&format!("    /// {}\n", summary));
    }

    output.push_str(&format!(
        r#"    pub async fn {}({}) -> Result<{}, ApiError> {{
        {}
        let response = {}
            .send()
            .await?;

        if !response.status().is_success() {{
            return Err(ApiError::Api {{
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            }});
        }}

        Ok(response.json().await?)
    }}

"#,
        method_name, params_str, op.response_type, path_code, request_build
    ));

    output
}
