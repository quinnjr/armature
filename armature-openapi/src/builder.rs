//! Builder for creating OpenAPI specifications programmatically

use crate::spec::*;
use std::collections::HashMap;

/// Builder for OpenAPI specifications
#[derive(Debug, Clone)]
pub struct OpenApiBuilder {
    spec: OpenApiSpec,
}

impl OpenApiBuilder {
    /// Create a new OpenAPI builder
    pub fn new(title: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            spec: OpenApiSpec {
                openapi: "3.0.0".to_string(),
                info: Info {
                    title: title.into(),
                    version: version.into(),
                    description: None,
                    terms_of_service: None,
                    contact: None,
                    license: None,
                },
                servers: Vec::new(),
                paths: HashMap::new(),
                components: Some(Components::default()),
                security: Vec::new(),
                tags: Vec::new(),
            },
        }
    }

    /// Set description
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.spec.info.description = Some(description.into());
        self
    }

    /// Set terms of service
    pub fn terms_of_service(mut self, terms: impl Into<String>) -> Self {
        self.spec.info.terms_of_service = Some(terms.into());
        self
    }

    /// Add contact information
    pub fn contact(
        mut self,
        name: Option<String>,
        url: Option<String>,
        email: Option<String>,
    ) -> Self {
        self.spec.info.contact = Some(Contact { name, url, email });
        self
    }

    /// Add license
    pub fn license(mut self, name: impl Into<String>, url: Option<String>) -> Self {
        self.spec.info.license = Some(License {
            name: name.into(),
            url,
        });
        self
    }

    /// Add a server
    pub fn server(mut self, url: impl Into<String>, description: Option<String>) -> Self {
        self.spec.servers.push(Server {
            url: url.into(),
            description,
        });
        self
    }

    /// Add a tag
    pub fn tag(mut self, name: impl Into<String>, description: Option<String>) -> Self {
        self.spec.tags.push(Tag {
            name: name.into(),
            description,
        });
        self
    }

    /// Add a path
    pub fn path(mut self, path: impl Into<String>, item: PathItem) -> Self {
        self.spec.paths.insert(path.into(), item);
        self
    }

    /// Add a schema component
    pub fn schema(mut self, name: impl Into<String>, schema: Schema) -> Self {
        if let Some(ref mut components) = self.spec.components {
            components.schemas.insert(name.into(), schema);
        }
        self
    }

    /// Add a security scheme
    pub fn security_scheme(mut self, name: impl Into<String>, scheme: SecurityScheme) -> Self {
        if let Some(ref mut components) = self.spec.components {
            components.security_schemes.insert(name.into(), scheme);
        }
        self
    }

    /// Add global security requirement
    pub fn security(mut self, requirement: SecurityRequirement) -> Self {
        self.spec.security.push(requirement);
        self
    }

    /// Build the OpenAPI specification
    pub fn build(self) -> OpenApiSpec {
        self.spec
    }
}

/// Helper functions for creating common components
impl OpenApiBuilder {
    /// Add Bearer JWT authentication
    pub fn add_bearer_auth(self, name: impl Into<String>) -> Self {
        self.security_scheme(
            name,
            SecurityScheme::Http {
                scheme: "bearer".to_string(),
                bearer_format: Some("JWT".to_string()),
            },
        )
    }

    /// Add API key authentication
    pub fn add_api_key_auth(
        self,
        name: impl Into<String>,
        key_name: impl Into<String>,
        location: ApiKeyLocation,
    ) -> Self {
        self.security_scheme(
            name,
            SecurityScheme::ApiKey {
                name: key_name.into(),
                location,
            },
        )
    }

    /// Add OpenID Connect authentication
    pub fn add_openid_connect_auth(
        self,
        name: impl Into<String>,
        open_id_connect_url: impl Into<String>,
    ) -> Self {
        self.security_scheme(
            name,
            SecurityScheme::OpenIdConnect {
                open_id_connect_url: open_id_connect_url.into(),
            },
        )
    }
}

/// Builder for path items
pub struct PathItemBuilder {
    item: PathItem,
}

impl PathItemBuilder {
    pub fn new() -> Self {
        Self {
            item: PathItem::default(),
        }
    }

    pub fn get(mut self, operation: Operation) -> Self {
        self.item.get = Some(operation);
        self
    }

    pub fn post(mut self, operation: Operation) -> Self {
        self.item.post = Some(operation);
        self
    }

    pub fn put(mut self, operation: Operation) -> Self {
        self.item.put = Some(operation);
        self
    }

    pub fn delete(mut self, operation: Operation) -> Self {
        self.item.delete = Some(operation);
        self
    }

    pub fn patch(mut self, operation: Operation) -> Self {
        self.item.patch = Some(operation);
        self
    }

    pub fn build(self) -> PathItem {
        self.item
    }
}

impl Default for PathItemBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for operations
pub struct OperationBuilder {
    operation: Operation,
}

impl OperationBuilder {
    pub fn new() -> Self {
        Self {
            operation: Operation::default(),
        }
    }

    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.operation.summary = Some(summary.into());
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.operation.description = Some(description.into());
        self
    }

    pub fn operation_id(mut self, id: impl Into<String>) -> Self {
        self.operation.operation_id = Some(id.into());
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.operation.tags.push(tag.into());
        self
    }

    pub fn parameter(mut self, parameter: Parameter) -> Self {
        self.operation.parameters.push(parameter);
        self
    }

    pub fn request_body(mut self, body: RequestBody) -> Self {
        self.operation.request_body = Some(body);
        self
    }

    pub fn response(mut self, status: impl Into<String>, response: Response) -> Self {
        self.operation.responses.insert(status.into(), response);
        self
    }

    pub fn security(mut self, requirement: SecurityRequirement) -> Self {
        self.operation.security.push(requirement);
        self
    }

    pub fn build(self) -> Operation {
        self.operation
    }
}

impl Default for OperationBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper functions for creating schemas
pub fn string_schema() -> Schema {
    Schema {
        schema_type: Some("string".to_string()),
        ..Default::default()
    }
}

pub fn integer_schema() -> Schema {
    Schema {
        schema_type: Some("integer".to_string()),
        format: Some("int64".to_string()),
        ..Default::default()
    }
}

pub fn number_schema() -> Schema {
    Schema {
        schema_type: Some("number".to_string()),
        format: Some("double".to_string()),
        ..Default::default()
    }
}

pub fn boolean_schema() -> Schema {
    Schema {
        schema_type: Some("boolean".to_string()),
        ..Default::default()
    }
}

pub fn array_schema(items: Schema) -> Schema {
    Schema {
        schema_type: Some("array".to_string()),
        items: Some(Box::new(items)),
        ..Default::default()
    }
}

pub fn object_schema(properties: HashMap<String, Schema>, required: Vec<String>) -> Schema {
    Schema {
        schema_type: Some("object".to_string()),
        properties: Some(properties),
        required,
        ..Default::default()
    }
}

pub fn ref_schema(reference: impl Into<String>) -> Schema {
    Schema {
        reference: Some(format!("#/components/schemas/{}", reference.into())),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openapi_builder_basic() {
        let spec = OpenApiBuilder::new("Test API", "1.0.0").build();

        assert_eq!(spec.info.title, "Test API");
        assert_eq!(spec.info.version, "1.0.0");
        assert_eq!(spec.openapi, "3.0.0");
    }

    #[test]
    fn test_openapi_builder_with_description() {
        let spec = OpenApiBuilder::new("Test API", "1.0.0")
            .description("A test API")
            .build();

        assert_eq!(spec.info.description, Some("A test API".to_string()));
    }

    #[test]
    fn test_operation_builder_basic() {
        let operation = OperationBuilder::new()
            .summary("Get user")
            .description("Get a user by ID")
            .operation_id("getUser")
            .build();

        assert_eq!(operation.summary, Some("Get user".to_string()));
        assert_eq!(operation.description, Some("Get a user by ID".to_string()));
        assert_eq!(operation.operation_id, Some("getUser".to_string()));
    }

    #[test]
    fn test_operation_builder_with_tag() {
        let operation = OperationBuilder::new().tag("users").tag("admin").build();

        assert_eq!(operation.tags.len(), 2);
        assert!(operation.tags.contains(&"users".to_string()));
        assert!(operation.tags.contains(&"admin".to_string()));
    }

    #[test]
    fn test_string_schema() {
        let schema = string_schema();
        assert_eq!(schema.schema_type, Some("string".to_string()));
    }

    #[test]
    fn test_integer_schema() {
        let schema = integer_schema();
        assert_eq!(schema.schema_type, Some("integer".to_string()));
        assert_eq!(schema.format, Some("int64".to_string()));
    }

    #[test]
    fn test_number_schema() {
        let schema = number_schema();
        assert_eq!(schema.schema_type, Some("number".to_string()));
        assert_eq!(schema.format, Some("double".to_string()));
    }

    #[test]
    fn test_boolean_schema() {
        let schema = boolean_schema();
        assert_eq!(schema.schema_type, Some("boolean".to_string()));
    }

    #[test]
    fn test_array_schema() {
        let items = string_schema();
        let schema = array_schema(items);

        assert_eq!(schema.schema_type, Some("array".to_string()));
        assert!(schema.items.is_some());
    }

    #[test]
    fn test_object_schema() {
        let mut properties = HashMap::new();
        properties.insert("name".to_string(), string_schema());
        properties.insert("age".to_string(), integer_schema());

        let required = vec!["name".to_string()];
        let schema = object_schema(properties, required);

        assert_eq!(schema.schema_type, Some("object".to_string()));
        assert_eq!(schema.properties.as_ref().unwrap().len(), 2);
        assert_eq!(schema.required.len(), 1);
    }

    #[test]
    fn test_ref_schema() {
        let schema = ref_schema("User");
        assert_eq!(
            schema.reference,
            Some("#/components/schemas/User".to_string())
        );
    }

    #[test]
    fn test_operation_builder_default() {
        let builder = OperationBuilder::default();
        let operation = builder.build();
        assert!(operation.summary.is_none());
    }

    #[test]
    fn test_add_openid_connect_auth() {
        let spec = OpenApiBuilder::new("Test API", "1.0.0")
            .add_openid_connect_auth(
                "oidc",
                "https://example.com/.well-known/openid-configuration",
            )
            .build();

        let components = spec.components.unwrap();
        match components.security_schemes.get("oidc").unwrap() {
            SecurityScheme::OpenIdConnect {
                open_id_connect_url,
            } => {
                assert_eq!(
                    open_id_connect_url,
                    "https://example.com/.well-known/openid-configuration"
                );
            }
            _ => panic!("expected OpenIdConnect variant"),
        }
    }
}
