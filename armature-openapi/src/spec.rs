//! OpenAPI 3.0 specification types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// OpenAPI 3.0 specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiSpec {
    pub openapi: String,
    pub info: Info,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub servers: Vec<Server>,
    pub paths: HashMap<String, PathItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Components>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub security: Vec<SecurityRequirement>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tags: Vec<Tag>,
}

/// API information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Info {
    pub title: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terms_of_service: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Contact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<License>,
}

/// Contact information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// License information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct License {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Server information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Path item
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PathItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub get: Option<Operation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post: Option<Operation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub put: Option<Operation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete: Option<Operation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<Operation>,
}

/// Operation (endpoint)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Operation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub parameters: Vec<Parameter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_body: Option<RequestBody>,
    pub responses: HashMap<String, Response>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub security: Vec<SecurityRequirement>,
}

/// Parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    #[serde(rename = "in")]
    pub location: ParameterLocation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Schema>,
}

/// Parameter location
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterLocation {
    Query,
    Header,
    Path,
    Cookie,
}

/// Request body
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub content: HashMap<String, MediaType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

/// Media type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaType {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Schema>,
}

/// Response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<HashMap<String, MediaType>>,
}

/// Schema (simplified)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Schema {
    #[serde(skip_serializing_if = "Option::is_none", rename = "type")]
    pub schema_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<HashMap<String, Schema>>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub required: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<Schema>>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "$ref")]
    pub reference: Option<String>,
}

/// Components
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Components {
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub schemas: HashMap<String, Schema>,
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub security_schemes: HashMap<String, SecurityScheme>,
}

/// Security scheme
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SecurityScheme {
    #[serde(rename = "http")]
    Http {
        scheme: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        bearer_format: Option<String>,
    },
    #[serde(rename = "apiKey")]
    ApiKey {
        name: String,
        #[serde(rename = "in")]
        location: ApiKeyLocation,
    },
    #[serde(rename = "oauth2")]
    OAuth2 { flows: Box<OAuthFlows> },
    #[serde(rename = "openIdConnect")]
    OpenIdConnect {
        #[serde(rename = "openIdConnectUrl")]
        open_id_connect_url: String,
    },
}

/// API key location
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiKeyLocation {
    Query,
    Header,
    Cookie,
}

/// OAuth flows
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OAuthFlows {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit: Option<OAuthFlow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<OAuthFlow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_credentials: Option<OAuthFlow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_code: Option<OAuthFlow>,
}

/// OAuth flow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthFlow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_url: Option<String>,
    pub scopes: HashMap<String, String>,
}

/// Security requirement
pub type SecurityRequirement = HashMap<String, Vec<String>>;

/// Tag
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openapi_spec_creation() {
        let spec = OpenApiSpec {
            openapi: "3.0.0".to_string(),
            info: Info {
                title: "Test".to_string(),
                version: "1.0.0".to_string(),
                description: None,
                terms_of_service: None,
                contact: None,
                license: None,
            },
            paths: HashMap::new(),
            components: None,
            servers: Vec::new(),
            security: Vec::new(),
            tags: Vec::new(),
        };

        assert_eq!(spec.openapi, "3.0.0");
    }

    #[test]
    fn test_info_creation() {
        let info = Info {
            title: "Test API".to_string(),
            version: "1.0.0".to_string(),
            description: None,
            terms_of_service: None,
            contact: None,
            license: None,
        };
        assert!(!info.title.is_empty());
        assert!(!info.version.is_empty());
    }

    #[test]
    fn test_path_item_creation() {
        let path = PathItem {
            get: Some(Operation::default()),
            ..Default::default()
        };

        assert!(path.get.is_some());
        assert!(path.post.is_none());
    }

    #[test]
    fn test_schema_string_type() {
        let schema = Schema {
            schema_type: Some("string".to_string()),
            format: Some("email".to_string()),
            ..Default::default()
        };

        assert_eq!(schema.schema_type, Some("string".to_string()));
        assert_eq!(schema.format, Some("email".to_string()));
    }

    #[test]
    fn test_schema_object_type() {
        let mut properties = HashMap::new();
        properties.insert(
            "name".to_string(),
            Schema {
                schema_type: Some("string".to_string()),
                ..Default::default()
            },
        );

        let schema = Schema {
            schema_type: Some("object".to_string()),
            properties: Some(properties),
            ..Default::default()
        };

        assert_eq!(schema.schema_type, Some("object".to_string()));
        assert!(schema.properties.is_some());
        assert_eq!(schema.properties.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_schema_array_type() {
        let schema = Schema {
            schema_type: Some("array".to_string()),
            items: Some(Box::new(Schema {
                schema_type: Some("string".to_string()),
                ..Default::default()
            })),
            ..Default::default()
        };

        assert_eq!(schema.schema_type, Some("array".to_string()));
        assert!(schema.items.is_some());
    }

    #[test]
    fn test_media_type_with_schema() {
        let media_type = MediaType {
            schema: Some(Schema {
                schema_type: Some("object".to_string()),
                ..Default::default()
            }),
        };

        assert!(media_type.schema.is_some());
    }

    #[test]
    fn test_request_body() {
        let mut content = HashMap::new();
        content.insert(
            "application/json".to_string(),
            MediaType {
                schema: Some(Schema::default()),
            },
        );

        let request_body = RequestBody {
            description: Some("User data".to_string()),
            content,
            required: Some(true),
        };

        assert!(request_body.required.unwrap());
        assert_eq!(request_body.content.len(), 1);
    }

    #[test]
    fn test_tag_creation() {
        let tag = Tag {
            name: "users".to_string(),
            description: Some("User operations".to_string()),
        };

        assert_eq!(tag.name, "users");
        assert!(tag.description.is_some());
    }

    #[test]
    fn test_components() {
        let mut schemas = HashMap::new();
        schemas.insert("User".to_string(), Schema::default());

        let components = Components {
            schemas,
            security_schemes: HashMap::new(),
        };

        assert!(!components.schemas.is_empty());
        assert_eq!(components.schemas.len(), 1);
    }

    #[test]
    fn test_schema_serialization() {
        let schema = Schema {
            schema_type: Some("string".to_string()),
            format: Some("email".to_string()),
            ..Default::default()
        };

        let json = serde_json::to_string(&schema).unwrap();
        let deserialized: Schema = serde_json::from_str(&json).unwrap();

        assert_eq!(schema.schema_type, deserialized.schema_type);
        assert_eq!(schema.format, deserialized.format);
    }

    #[test]
    fn test_operation_with_tags() {
        let operation = Operation {
            tags: vec!["users".to_string(), "admin".to_string()],
            ..Default::default()
        };

        assert_eq!(operation.tags.len(), 2);
        assert!(operation.tags.contains(&"users".to_string()));
    }

    #[test]
    fn test_schema_with_ref() {
        let schema = Schema {
            reference: Some("#/components/schemas/User".to_string()),
            ..Default::default()
        };

        assert!(schema.reference.is_some());
        assert!(schema.schema_type.is_none());
    }

    #[test]
    fn test_response_creation() {
        let response = Response {
            description: "Success".to_string(),
            content: Some(HashMap::new()),
        };

        assert_eq!(response.description, "Success");
        assert!(response.content.is_some());
    }

    #[test]
    fn test_schema_defaults() {
        let schema = Schema::default();
        assert!(schema.schema_type.is_none());
        assert!(schema.format.is_none());
        assert!(schema.properties.is_none());
    }

    #[test]
    fn test_security_scheme_open_id_connect_serialization() {
        let scheme = SecurityScheme::OpenIdConnect {
            open_id_connect_url: "https://example.com/.well-known/openid-configuration".to_string(),
        };

        let json = serde_json::to_value(&scheme).unwrap();
        assert_eq!(json["type"], "openIdConnect");
        assert_eq!(
            json["openIdConnectUrl"],
            "https://example.com/.well-known/openid-configuration"
        );

        let deserialized: SecurityScheme = serde_json::from_value(json).unwrap();
        match deserialized {
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
