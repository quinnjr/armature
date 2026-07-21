//! Swagger UI integration

use crate::spec::OpenApiSpec;
use armature_core::{Error, HttpResponse};

/// Swagger UI configuration
#[derive(Debug, Clone)]
pub struct SwaggerConfig {
    /// Path where Swagger UI will be served (e.g., "/api-docs")
    pub path: String,
    /// Title for the Swagger UI page
    pub title: String,
    /// OpenAPI specification
    pub spec: OpenApiSpec,
}

impl SwaggerConfig {
    /// Create a new Swagger configuration
    pub fn new(path: impl Into<String>, spec: OpenApiSpec) -> Self {
        Self {
            path: path.into(),
            title: "API Documentation".to_string(),
            spec,
        }
    }

    /// Set the title
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Get the OpenAPI spec as JSON
    pub fn spec_json(&self) -> Result<String, Error> {
        serde_json::to_string_pretty(&self.spec)
            .map_err(|e| Error::Internal(format!("Failed to serialize spec: {}", e)))
    }

    /// Get the OpenAPI spec as YAML
    pub fn spec_yaml(&self) -> Result<String, Error> {
        serde_yaml::to_string(&self.spec)
            .map_err(|e| Error::Internal(format!("Failed to serialize spec: {}", e)))
    }
}

/// Generate a Swagger UI HTML response.
///
/// The returned page loads the `swagger-ui-dist` JS/CSS bundle from the
/// jsdelivr CDN (`cdn.jsdelivr.net`) rather than bundling it — it is not
/// self-contained. It will fail to render fully offline or under a strict
/// Content-Security-Policy that blocks that origin.
pub fn swagger_ui_response(config: &SwaggerConfig) -> Result<HttpResponse, Error> {
    let spec_json = config.spec_json()?;
    // JSON is already valid JS object-literal syntax, so it can be injected
    // as-is. The only hazard is a literal `</script>` (or any `</`) inside a
    // string value breaking out of the enclosing <script> block, since serde
    // does not escape `/`. Escaping `</` to `<\/` is a JSON/JS-safe transform
    // that renders identically. We must NOT double backslashes: that would
    // corrupt data (e.g. a `\d+` pattern would render as `\\d+`).
    let spec_json_safe = spec_json.replace("</", "<\\/");

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
    <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5.10.0/swagger-ui.css">
    <style>
        body {{
            margin: 0;
            padding: 0;
        }}
    </style>
</head>
<body>
    <div id="swagger-ui"></div>
    <script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5.10.0/swagger-ui-bundle.js"></script>
    <script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5.10.0/swagger-ui-standalone-preset.js"></script>
    <script>
        window.onload = function() {{
            const spec = {spec};
            SwaggerUIBundle({{
                spec: spec,
                dom_id: '#swagger-ui',
                deepLinking: true,
                presets: [
                    SwaggerUIBundle.presets.apis,
                    SwaggerUIStandalonePreset
                ],
                plugins: [
                    SwaggerUIBundle.plugins.DownloadUrl
                ],
                layout: "StandaloneLayout"
            }});
        }};
    </script>
</body>
</html>"#,
        title = config.title,
        spec = spec_json_safe
    );

    Ok(HttpResponse::ok()
        .with_header("content-type".to_string(), "text/html".to_string())
        .with_body(html.into_bytes()))
}

/// Generate a response for the OpenAPI spec JSON endpoint
pub fn spec_json_response(config: &SwaggerConfig) -> Result<HttpResponse, Error> {
    let spec_json = config.spec_json()?;

    Ok(HttpResponse::ok()
        .with_header("content-type".to_string(), "application/json".to_string())
        .with_body(spec_json.into_bytes()))
}

/// Generate a response for the OpenAPI spec YAML endpoint
pub fn spec_yaml_response(config: &SwaggerConfig) -> Result<HttpResponse, Error> {
    let spec_yaml = config.spec_yaml()?;

    Ok(HttpResponse::ok()
        .with_header("content-type".to_string(), "application/x-yaml".to_string())
        .with_body(spec_yaml.into_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::OpenApiBuilder;

    #[test]
    fn test_swagger_config() {
        let spec = OpenApiBuilder::new("Test API", "1.0.0").build();
        let config = SwaggerConfig::new("/api-docs", spec).with_title("Test Docs");

        assert_eq!(config.path, "/api-docs");
        assert_eq!(config.title, "Test Docs");
    }

    #[test]
    fn test_spec_json() {
        let spec = OpenApiBuilder::new("Test API", "1.0.0")
            .description("A test API")
            .build();
        let config = SwaggerConfig::new("/api-docs", spec);

        let json = config.spec_json().unwrap();
        assert!(json.contains("Test API"));
        assert!(json.contains("1.0.0"));
    }

    #[test]
    fn test_spec_yaml() {
        let spec = OpenApiBuilder::new("Test API", "1.0.0")
            .description("A test API")
            .build();
        let config = SwaggerConfig::new("/api-docs", spec);

        let yaml = config.spec_yaml().unwrap();
        assert!(yaml.contains("Test API"));
        assert!(yaml.contains("1.0.0"));
    }
}
