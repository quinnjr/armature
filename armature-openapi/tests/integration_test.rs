//! Integration tests for armature-openapi

use armature_openapi::*;

#[test]
fn test_openapi_builder_creation() {
    let builder = OpenApiBuilder::new("My API", "1.0.0");
    let spec = builder.build();

    assert_eq!(spec.openapi, "3.0.0");
    assert_eq!(spec.info.title, "My API");
    assert_eq!(spec.info.version, "1.0.0");
}

#[test]
fn test_openapi_builder_with_description() {
    let builder = OpenApiBuilder::new("My API", "1.0.0").description("API Description");
    let spec = builder.build();

    assert_eq!(spec.info.description, Some("API Description".to_string()));
}

#[test]
fn test_openapi_builder_with_server() {
    let builder = OpenApiBuilder::new("My API", "1.0.0")
        .server("https://api.example.com", Some("Production".to_string()));
    let spec = builder.build();

    assert_eq!(spec.servers.len(), 1);
    assert_eq!(spec.servers[0].url, "https://api.example.com");
}

#[test]
fn test_openapi_builder_with_path() {
    let builder = OpenApiBuilder::new("My API", "1.0.0").path("/users", PathItem::default());
    let spec = builder.build();

    assert!(spec.paths.contains_key("/users"));
}

#[test]
fn test_openapi_info_creation() {
    let info = Info {
        title: "Test API".to_string(),
        version: "1.0.0".to_string(),
        description: Some("Description".to_string()),
        terms_of_service: None,
        contact: None,
        license: None,
    };

    assert_eq!(info.title, "Test API");
    assert_eq!(info.version, "1.0.0");
}

#[test]
fn test_server_creation() {
    let server = Server {
        url: "https://api.example.com".to_string(),
        description: Some("Production".to_string()),
    };

    assert_eq!(server.url, "https://api.example.com");
}

#[test]
fn test_path_item_default() {
    let path = PathItem::default();
    assert!(path.get.is_none());
    assert!(path.post.is_none());
}

#[test]
fn test_operation_default() {
    let op = Operation::default();
    assert!(op.tags.is_empty());
    assert!(op.summary.is_none());
}

#[test]
fn test_schema_creation() {
    let schema = Schema {
        schema_type: Some("string".to_string()),
        format: None,
        description: Some("A string field".to_string()),
        properties: None,
        required: vec![],
        items: None,
        reference: None,
    };

    assert_eq!(schema.schema_type, Some("string".to_string()));
}

#[test]
fn test_swagger_config_creation() {
    let spec = OpenApiBuilder::new("My API", "1.0.0").build();
    let config = SwaggerConfig::new("/api-docs", spec);

    assert_eq!(config.path, "/api-docs");
    assert_eq!(config.title, "API Documentation");
}

#[test]
fn test_swagger_config_with_title() {
    let spec = OpenApiBuilder::new("My API", "1.0.0").build();
    let config = SwaggerConfig::new("/docs", spec).with_title("My API Docs");

    assert_eq!(config.path, "/docs");
    assert_eq!(config.title, "My API Docs");
}

#[test]
fn test_swagger_ui_response() {
    let spec = OpenApiBuilder::new("My API", "1.0.0").build();
    let config = SwaggerConfig::new("/api-docs", spec);
    let result = swagger_ui_response(&config);

    assert!(result.is_ok());
    let response = result.unwrap();
    let body = String::from_utf8_lossy(&response.body);
    assert!(body.contains("<!DOCTYPE html>"));
    assert!(body.contains("swagger-ui"));
}

#[test]
fn test_swagger_ui_response_escapes_script_but_not_backslashes() {
    // A description carrying both a regex backslash (`\d+`) and a raw
    // `</script>` sequence exercises the two hazards of injecting the spec
    // JSON into the HTML <script> block.
    let spec = OpenApiBuilder::new("My API", "1.0.0")
        .description(r"regex \d+ break</script>injection")
        .build();
    let config = SwaggerConfig::new("/api-docs", spec);
    let body_bytes = swagger_ui_response(&config).unwrap().body;
    let body = String::from_utf8(body_bytes).unwrap();

    // Backslash must be single-escaped by serde (`\\` in JSON text) and NOT
    // doubled again. A doubled backslash (`\\\\d+`) would corrupt the data
    // the browser reconstructs.
    assert!(
        body.contains(r"\\d+"),
        "expected single JSON-escaped backslash for the \\d+ pattern"
    );
    assert!(
        !body.contains(r"\\\\d+"),
        "backslash was double-escaped, corrupting the rendered data"
    );

    // The `</script>` inside the spec data must be neutralized to `<\/script>`
    // so it cannot break out of the enclosing <script> element. The raw,
    // unescaped data sequence must not appear.
    assert!(
        body.contains(r"break<\/script>injection"),
        "the spec's </script> was not escaped to <\\/script>"
    );
    assert!(
        !body.contains("break</script>injection"),
        "a raw </script> from the spec data leaked into the page"
    );
}

#[test]
fn test_openapi_spec_serialization() {
    let builder = OpenApiBuilder::new("My API", "1.0.0");
    let spec = builder.build();

    let json = serde_json::to_string(&spec);
    assert!(json.is_ok());

    let json_str = json.unwrap();
    assert!(json_str.contains("My API"));
    assert!(json_str.contains("1.0.0"));
}
