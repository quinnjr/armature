//! Lambda request conversion.

use bytes::Bytes;
use lambda_http::aws_lambda_events::apigw::ApiGatewayRequestAuthorizer;
use lambda_http::aws_lambda_events::query_map::QueryMap;
use lambda_http::{Request, RequestExt};
use std::collections::HashMap;

/// Wrapper for Lambda HTTP requests.
pub struct LambdaRequest {
    /// HTTP method.
    pub method: http::Method,
    /// Request path.
    pub path: String,
    /// Query string.
    pub query_string: Option<String>,
    /// Headers.
    pub headers: HashMap<String, String>,
    /// Request body.
    pub body: Bytes,
    /// Path parameters (from API Gateway).
    pub path_parameters: HashMap<String, String>,
    /// Stage variables (from API Gateway).
    pub stage_variables: HashMap<String, String>,
    /// Request context.
    pub request_context: RequestContext,
}

/// Request context from API Gateway.
#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    /// Request ID.
    pub request_id: Option<String>,
    /// Stage name.
    pub stage: Option<String>,
    /// Domain name.
    pub domain_name: Option<String>,
    /// HTTP method.
    pub http_method: Option<String>,
    /// Source IP.
    pub source_ip: Option<String>,
    /// User agent.
    pub user_agent: Option<String>,
    /// Authorizer claims (for Cognito / JWT authorizers).
    pub authorizer_claims: HashMap<String, String>,
}

impl LambdaRequest {
    /// Create from a lambda_http::Request.
    pub fn from_lambda_request(request: Request) -> Self {
        // Path parameters and stage variables are pre-extracted by
        // `lambda_http` into request extensions for both API Gateway REST
        // (V1) and HTTP (V2) events, so read them before consuming the
        // request into parts.
        let path_parameters = query_map_to_hashmap(&request.path_parameters());
        let stage_variables = query_map_to_hashmap(&request.stage_variables());

        let (parts, body) = request.into_parts();

        // Extract headers
        let mut headers = HashMap::new();
        for (name, value) in parts.headers.iter() {
            if let Ok(v) = value.to_str() {
                headers.insert(name.to_string(), v.to_string());
            }
        }

        // Extract query string
        let query_string = parts.uri.query().map(String::from);

        // Extract request context (including authorizer claims) from extensions.
        let request_context = parts
            .extensions
            .get::<lambda_http::request::RequestContext>()
            .map(|ctx| match ctx {
                lambda_http::request::RequestContext::ApiGatewayV2(v2) => RequestContext {
                    request_id: v2.request_id.clone(),
                    stage: v2.stage.clone(),
                    domain_name: v2.domain_name.clone(),
                    http_method: Some(v2.http.method.to_string()),
                    source_ip: v2.http.source_ip.clone(),
                    user_agent: v2.http.user_agent.clone(),
                    authorizer_claims: v2
                        .authorizer
                        .as_ref()
                        .map(extract_claims)
                        .unwrap_or_default(),
                },
                lambda_http::request::RequestContext::ApiGatewayV1(v1) => RequestContext {
                    request_id: v1.request_id.clone(),
                    stage: v1.stage.clone(),
                    domain_name: v1.domain_name.clone(),
                    http_method: Some(v1.http_method.to_string()),
                    source_ip: v1.identity.source_ip.clone(),
                    user_agent: v1.identity.user_agent.clone(),
                    authorizer_claims: extract_claims(&v1.authorizer),
                },
                lambda_http::request::RequestContext::Alb(_) => RequestContext::default(),
                _ => RequestContext::default(),
            })
            .unwrap_or_default();

        // Convert body. lambda_http::Body is #[non_exhaustive] so we
        // need a wildcard arm — fall back to an empty body on any
        // future variant rather than panicking.
        let body_bytes = match body {
            lambda_http::Body::Empty => Bytes::new(),
            lambda_http::Body::Text(s) => Bytes::from(s),
            lambda_http::Body::Binary(b) => Bytes::from(b),
            _ => Bytes::new(),
        };

        Self {
            method: parts.method,
            path: parts.uri.path().to_string(),
            query_string,
            headers,
            body: body_bytes,
            path_parameters,
            stage_variables,
            request_context,
        }
    }

    /// Get a header value.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_lowercase())
            .or_else(|| self.headers.get(name))
            .map(|s| s.as_str())
    }

    /// Get the content type.
    pub fn content_type(&self) -> Option<&str> {
        self.header("content-type")
    }

    /// Check if the request is JSON.
    pub fn is_json(&self) -> bool {
        self.content_type()
            .map(|ct| ct.contains("application/json"))
            .unwrap_or(false)
    }

    /// Get the source IP.
    pub fn source_ip(&self) -> Option<&str> {
        self.request_context.source_ip.as_deref()
    }

    /// Get a path parameter.
    pub fn path_parameter(&self, name: &str) -> Option<&str> {
        self.path_parameters.get(name).map(|s| s.as_str())
    }

    /// Get a stage variable.
    pub fn stage_variable(&self, name: &str) -> Option<&str> {
        self.stage_variables.get(name).map(|s| s.as_str())
    }

    /// Get authorizer claims (for Cognito / JWT authorizers).
    pub fn claims(&self) -> &HashMap<String, String> {
        &self.request_context.authorizer_claims
    }

    /// Get a specific claim.
    pub fn claim(&self, key: &str) -> Option<&str> {
        self.request_context
            .authorizer_claims
            .get(key)
            .map(|s| s.as_str())
    }
}

/// Convert a `lambda_http` `QueryMap` (path parameters / stage variables) into
/// a flat `HashMap`. When a key is multi-valued the first value wins.
fn query_map_to_hashmap(map: &QueryMap) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (key, value) in map.iter() {
        out.entry(key.to_string())
            .or_insert_with(|| value.to_string());
    }
    out
}

/// Extract authorizer claims from an API Gateway request authorizer.
///
/// Handles both shapes:
/// - HTTP API (V2) and REST API JWT authorizers expose `authorizer.jwt.claims`.
/// - REST API (V1) Cognito / custom authorizers expose the claims as a nested
///   `claims` object inside the raw `authorizer` map (captured in `fields`).
fn extract_claims(authorizer: &ApiGatewayRequestAuthorizer) -> HashMap<String, String> {
    if let Some(jwt) = &authorizer.jwt
        && !jwt.claims.is_empty()
    {
        return jwt.claims.clone();
    }

    if let Some(serde_json::Value::Object(claims)) = authorizer.fields.get("claims") {
        return claims
            .iter()
            .filter_map(|(key, value)| match value {
                serde_json::Value::String(s) => Some((key.clone(), s.clone())),
                serde_json::Value::Null => None,
                other => Some((key.clone(), other.to_string())),
            })
            .collect();
    }

    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lambda_http::Body;
    use lambda_http::request::RequestContext as HttpRequestContext;

    fn v2_context_json() -> &'static str {
        r#"{
            "routeKey": "POST /users/{id}",
            "accountId": "123456789012",
            "stage": "$default",
            "requestId": "req-v2-1",
            "authorizer": {
                "jwt": {
                    "claims": { "sub": "user-123", "email": "user@example.com" },
                    "scopes": ["read"]
                }
            },
            "apiId": "abcd1234",
            "domainName": "api.example.com",
            "http": {
                "method": "POST",
                "path": "/users/42",
                "protocol": "HTTP/1.1",
                "sourceIp": "203.0.113.7",
                "userAgent": "test-agent/1.0"
            },
            "timeEpoch": 0
        }"#
    }

    fn v1_context_json() -> &'static str {
        r#"{
            "accountId": "123456789012",
            "resourceId": "abc123",
            "stage": "prod",
            "requestId": "req-v1-1",
            "domainName": "api.example.com",
            "identity": {
                "sourceIp": "198.51.100.9",
                "userAgent": "rest-agent/2.0"
            },
            "authorizer": {
                "claims": {
                    "sub": "cognito-user-9",
                    "cognito:username": "alice"
                }
            },
            "resourcePath": "/users/{id}",
            "httpMethod": "POST",
            "apiId": "restapi1"
        }"#
    }

    fn make_v2_request() -> Request {
        let ctx: lambda_http::aws_lambda_events::apigw::ApiGatewayV2httpRequestContext =
            serde_json::from_str(v2_context_json()).expect("v2 context deserializes");

        let mut path_params = HashMap::new();
        path_params.insert("id".to_string(), "42".to_string());

        let mut stage_vars = HashMap::new();
        stage_vars.insert("env".to_string(), "staging".to_string());

        http::Request::builder()
            .method("POST")
            .uri("https://api.example.com/users/42?page=2")
            .header("content-type", "application/json")
            .header("x-custom", "hello")
            .body(Body::Text("{\"name\":\"a\"}".to_string()))
            .unwrap()
            .with_path_parameters(path_params)
            .with_stage_variables(stage_vars)
            .with_request_context(HttpRequestContext::ApiGatewayV2(ctx))
    }

    fn make_v1_request() -> Request {
        let ctx: lambda_http::aws_lambda_events::apigw::ApiGatewayProxyRequestContext =
            serde_json::from_str(v1_context_json()).expect("v1 context deserializes");

        let mut path_params = HashMap::new();
        path_params.insert("id".to_string(), "42".to_string());

        let mut stage_vars = HashMap::new();
        stage_vars.insert("region".to_string(), "us-east-1".to_string());

        http::Request::builder()
            .method("POST")
            .uri("https://api.example.com/users/42")
            .header("content-type", "application/json")
            .body(Body::Text("body-1".to_string()))
            .unwrap()
            .with_path_parameters(path_params)
            .with_stage_variables(stage_vars)
            .with_request_context(HttpRequestContext::ApiGatewayV1(ctx))
    }

    #[test]
    fn v2_claims_are_populated_from_jwt() {
        let req = LambdaRequest::from_lambda_request(make_v2_request());

        assert_eq!(req.claim("sub"), Some("user-123"));
        assert_eq!(req.claim("email"), Some("user@example.com"));
        assert_eq!(req.claims().len(), 2);
    }

    #[test]
    fn v1_claims_are_populated_from_authorizer_map() {
        let req = LambdaRequest::from_lambda_request(make_v1_request());

        assert_eq!(req.claim("sub"), Some("cognito-user-9"));
        assert_eq!(req.claim("cognito:username"), Some("alice"));
        assert_eq!(req.claims().len(), 2);
    }

    #[test]
    fn v2_path_parameters_and_stage_variables_are_extracted() {
        let req = LambdaRequest::from_lambda_request(make_v2_request());

        assert_eq!(req.path_parameter("id"), Some("42"));
        assert_eq!(req.stage_variable("env"), Some("staging"));
    }

    #[test]
    fn v1_path_parameters_and_stage_variables_are_extracted() {
        let req = LambdaRequest::from_lambda_request(make_v1_request());

        assert_eq!(req.path_parameter("id"), Some("42"));
        assert_eq!(req.stage_variable("region"), Some("us-east-1"));
    }

    #[test]
    fn context_and_headers_and_body_are_mapped() {
        let req = LambdaRequest::from_lambda_request(make_v2_request());

        assert_eq!(req.method, http::Method::POST);
        assert_eq!(req.path, "/users/42");
        assert_eq!(req.query_string.as_deref(), Some("page=2"));
        assert_eq!(req.header("content-type"), Some("application/json"));
        assert_eq!(req.header("x-custom"), Some("hello"));
        assert!(req.is_json());
        assert_eq!(&req.body[..], b"{\"name\":\"a\"}");

        assert_eq!(req.request_context.request_id.as_deref(), Some("req-v2-1"));
        assert_eq!(req.request_context.stage.as_deref(), Some("$default"));
        assert_eq!(req.source_ip(), Some("203.0.113.7"));
        assert_eq!(
            req.request_context.user_agent.as_deref(),
            Some("test-agent/1.0")
        );
    }

    #[test]
    fn missing_context_yields_empty_defaults() {
        let req = LambdaRequest::from_lambda_request(
            http::Request::builder()
                .method("GET")
                .uri("https://api.example.com/health")
                .body(Body::Empty)
                .unwrap(),
        );

        assert!(req.claims().is_empty());
        assert!(req.path_parameters.is_empty());
        assert!(req.stage_variables.is_empty());
        assert_eq!(req.body.len(), 0);
    }
}
