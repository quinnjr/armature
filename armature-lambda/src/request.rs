//! Lambda request conversion.

use bytes::Bytes;
use lambda_http::Request;
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
    /// Authorizer claims (for Cognito).
    pub authorizer_claims: HashMap<String, String>,
}

impl LambdaRequest {
    /// Create from a lambda_http::Request.
    pub fn from_lambda_request(request: Request) -> Self {
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

        // Extract path parameters and request context from extensions
        let (path_parameters, request_context) = parts
            .extensions
            .get::<lambda_http::request::RequestContext>()
            .map(|ctx| {
                match ctx {
                    lambda_http::request::RequestContext::ApiGatewayV2(v2) => {
                        // V2 doesn't expose path parameters directly in the same way
                        let params = HashMap::new();

                        let ctx = RequestContext {
                            request_id: v2.request_id.clone(),
                            stage: v2.stage.clone(),
                            domain_name: v2.domain_name.clone(),
                            http_method: Some(v2.http.method.to_string()),
                            source_ip: v2.http.source_ip.clone(),
                            user_agent: v2.http.user_agent.clone(),
                            authorizer_claims: HashMap::new(),
                        };
                        (params, ctx)
                    }
                    lambda_http::request::RequestContext::ApiGatewayV1(v1) => {
                        let params = HashMap::new();
                        let ctx = RequestContext {
                            request_id: v1.request_id.clone(),
                            stage: v1.stage.clone(),
                            domain_name: v1.domain_name.clone(),
                            http_method: Some(v1.http_method.to_string()),
                            source_ip: v1.identity.source_ip.clone(),
                            user_agent: v1.identity.user_agent.clone(),
                            authorizer_claims: extract_claims_v1(v1),
                        };
                        (params, ctx)
                    }
                    lambda_http::request::RequestContext::Alb(_) => {
                        (HashMap::new(), RequestContext::default())
                    }
                    _ => (HashMap::new(), RequestContext::default()),
                }
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
            stage_variables: HashMap::new(),
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

    /// Get authorizer claims (for Cognito).
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

/// Extract claims from API Gateway V1 authorizer.
fn extract_claims_v1(
    _v1: &lambda_http::aws_lambda_events::apigw::ApiGatewayProxyRequestContext,
) -> HashMap<String, String> {
    // The authorizer field structure has changed - just return empty for now
    // Users can access raw context if needed
    HashMap::new()
}
