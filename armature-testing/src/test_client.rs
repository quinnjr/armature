// Test HTTP Client

use armature_core::Bytes;
use armature_core::{Error, HttpMethod, HttpRequest, HttpResponse, Router};
use std::collections::HashMap;
use std::sync::Arc;

/// Test HTTP client for making requests to the application
pub struct TestClient {
    router: Arc<Router>,
}

impl TestClient {
    /// Create a new test client
    pub fn new(router: Arc<Router>) -> Self {
        Self { router }
    }

    /// Make a GET request
    pub async fn get(&self, path: &str) -> TestResponse {
        self.request(HttpMethod::GET, path, None).await
    }

    /// Make a POST request
    pub async fn post(&self, path: &str, body: Vec<u8>) -> TestResponse {
        self.request(HttpMethod::POST, path, Some(body)).await
    }

    /// Make a PUT request
    pub async fn put(&self, path: &str, body: Vec<u8>) -> TestResponse {
        self.request(HttpMethod::PUT, path, Some(body)).await
    }

    /// Make a DELETE request
    pub async fn delete(&self, path: &str) -> TestResponse {
        self.request(HttpMethod::DELETE, path, None).await
    }

    /// Make a PATCH request
    pub async fn patch(&self, path: &str, body: Vec<u8>) -> TestResponse {
        self.request(HttpMethod::PATCH, path, Some(body)).await
    }

    /// Make a QUERY request (safe query with a request body,
    /// draft-ietf-httpbis-safe-method-w-body)
    pub async fn query(&self, path: &str, body: Vec<u8>) -> TestResponse {
        self.request(HttpMethod::QUERY, path, Some(body)).await
    }

    /// Make a request with custom method
    pub async fn request(
        &self,
        method: HttpMethod,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> TestResponse {
        let mut req = HttpRequest::new(method.as_str().to_string(), path.to_string());
        req.body = Bytes::from(body.unwrap_or_default());

        // Route the request
        match self.router.route(req).await {
            Ok(response) => TestResponse::Success(response),
            Err(error) => TestResponse::Error(error),
        }
    }
}

/// Builder for test requests
#[allow(dead_code)]
pub struct TestRequestBuilder {
    method: HttpMethod,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
    query_params: HashMap<String, String>,
}

impl TestRequestBuilder {
    /// Create a new request builder
    #[allow(dead_code)]
    pub fn new(method: HttpMethod, path: &str) -> Self {
        Self {
            method,
            path: path.to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
            query_params: HashMap::new(),
        }
    }

    /// Add a header
    #[allow(dead_code)]
    pub fn header(mut self, key: &str, value: &str) -> Self {
        self.headers.insert(key.to_string(), value.to_string());
        self
    }

    /// Set the body
    #[allow(dead_code)]
    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }

    /// Set JSON body
    #[allow(dead_code)]
    pub fn json<T: serde::Serialize>(mut self, data: &T) -> Result<Self, Error> {
        self.body = serde_json::to_vec(data).map_err(|e| Error::Serialization(e.to_string()))?;
        self.headers
            .insert("Content-Type".to_string(), "application/json".to_string());
        Ok(self)
    }

    /// Add a query parameter
    #[allow(dead_code)]
    pub fn query(mut self, key: &str, value: &str) -> Self {
        self.query_params.insert(key.to_string(), value.to_string());
        self
    }

    /// Build the request
    #[allow(dead_code)]
    pub fn build(self) -> HttpRequest {
        // Build query string
        let query_string = if !self.query_params.is_empty() {
            let params: Vec<String> = self
                .query_params
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();
            format!("?{}", params.join("&"))
        } else {
            String::new()
        };

        HttpRequest::from_parts(
            self.method.as_str().to_string(),
            format!("{}{}", self.path, query_string),
            self.headers,
            self.body,
            HashMap::new(),
            self.query_params,
        )
    }
}

/// Response from a test request
#[derive(Debug)]
pub enum TestResponse {
    Success(HttpResponse),
    Error(Error),
}

impl TestResponse {
    /// Assert the response is successful
    pub fn assert_success(&self) -> &HttpResponse {
        match self {
            TestResponse::Success(response) => response,
            TestResponse::Error(error) => {
                panic!("Expected success response, got error: {:?}", error)
            }
        }
    }

    /// Assert the response is an error
    pub fn assert_error(&self) -> &Error {
        match self {
            TestResponse::Error(error) => error,
            TestResponse::Success(_) => {
                panic!("Expected error response, got success")
            }
        }
    }

    /// Get the status code
    pub fn status(&self) -> Option<u16> {
        match self {
            TestResponse::Success(response) => Some(response.status),
            TestResponse::Error(_) => None,
        }
    }

    /// Get the response body as string
    pub fn body_string(&self) -> Option<String> {
        match self {
            TestResponse::Success(response) => String::from_utf8(response.body.to_vec()).ok(),
            TestResponse::Error(_) => None,
        }
    }

    /// Get the response body as JSON
    pub fn body_json<T: serde::de::DeserializeOwned>(&self) -> Result<T, String> {
        match self {
            TestResponse::Success(response) => serde_json::from_slice(&response.body)
                .map_err(|e| format!("Serialization error: {}", e)),
            TestResponse::Error(error) => Err(format!("{:?}", error)),
        }
    }

    /// Get a header value
    pub fn header(&self, key: &str) -> Option<&String> {
        match self {
            TestResponse::Success(response) => response.headers.get(key),
            TestResponse::Error(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_builder() {
        let req = TestRequestBuilder::new(HttpMethod::GET, "/test")
            .header("Authorization", "Bearer token")
            .query("foo", "bar")
            .build();

        assert_eq!(req.method, "GET");
        assert!(req.path.contains("/test"));
        assert_eq!(req.headers.get("Authorization"), Some("Bearer token"));
    }
}
