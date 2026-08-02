// Test assertions for HTTP responses

use crate::TestResponse;
use armature_core::{HttpResponse, HttpStatus};

/// Assert that a response has a specific status code
pub fn assert_status(response: &TestResponse, expected: u16) {
    let actual = response.status().unwrap_or(0);
    assert_eq!(
        actual, expected,
        "Expected status {}, got {}",
        expected, actual
    );
}

/// Assert that a response has a specific HTTP status
#[allow(dead_code)]
pub fn assert_http_status(response: &HttpResponse, expected: HttpStatus) {
    assert_eq!(
        response.status,
        expected.code(),
        "Expected status {}, got {}",
        expected.code(),
        response.status
    );
}

/// Assert that a response body contains JSON matching expected value
pub fn assert_json<T>(response: &TestResponse, expected: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let actual: T = response
        .body_json()
        .expect("Failed to deserialize response body");
    assert_eq!(actual, *expected, "JSON bodies do not match");
}

/// Assert that a response has a specific header
pub fn assert_header(response: &TestResponse, key: &str, expected: &str) {
    let actual = response.header(key).map(|s| s.as_str());
    assert_eq!(
        actual,
        Some(expected),
        "Expected header '{}' to be '{}', got {:?}",
        key,
        expected,
        actual
    );
}

/// Assert that a response body contains a string
#[allow(dead_code)]
pub fn assert_body_contains(response: &TestResponse, expected: &str) {
    let body = response.body_string().unwrap_or_default();
    assert!(
        body.contains(expected),
        "Expected body to contain '{}', but it didn't. Body: {}",
        expected,
        body
    );
}

/// Assert that a response is successful (2xx status)
#[allow(dead_code)]
pub fn assert_success(response: &TestResponse) {
    let status = response.status().unwrap_or(0);
    assert!(
        (200..300).contains(&status),
        "Expected successful status (2xx), got {}",
        status
    );
}

/// Assert that a response is a client error (4xx status)
#[allow(dead_code)]
pub fn assert_client_error(response: &TestResponse) {
    let status = response.status().unwrap_or(0);
    assert!(
        (400..500).contains(&status),
        "Expected client error status (4xx), got {}",
        status
    );
}

/// Assert that a response is a server error (5xx status)
#[allow(dead_code)]
pub fn assert_server_error(response: &TestResponse) {
    let status = response.status().unwrap_or(0);
    assert!(
        (500..600).contains(&status),
        "Expected server error status (5xx), got {}",
        status
    );
}

/// Assert that a response has JSON content type
#[allow(dead_code)]
pub fn assert_json_content_type(response: &TestResponse) {
    let content_type = response.header("Content-Type");
    assert!(
        content_type
            .map(|ct| ct.contains("application/json"))
            .unwrap_or(false),
        "Expected JSON content type, got {:?}",
        content_type
    );
}

/// Assert that a response has HTML content type
#[allow(dead_code)]
pub fn assert_html_content_type(response: &TestResponse) {
    let content_type = response.header("Content-Type");
    assert!(
        content_type
            .map(|ct| ct.contains("text/html"))
            .unwrap_or(false),
        "Expected HTML content type, got {:?}",
        content_type
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use armature_core::HttpResponse;

    fn create_test_response(status: u16, body: &str) -> TestResponse {
        let mut response = HttpResponse::new(status);
        response.body = armature_core::Bytes::copy_from_slice(body.as_bytes());
        TestResponse::Success(response)
    }

    #[test]
    fn test_assert_status() {
        let response = create_test_response(200, "OK");
        assert_status(&response, 200);
    }

    #[test]
    fn test_assert_success() {
        let response = create_test_response(200, "OK");
        assert_success(&response);
    }

    #[test]
    fn test_assert_body_contains() {
        let response = create_test_response(200, "Hello World");
        assert_body_contains(&response, "Hello");
    }

    #[test]
    fn test_assert_client_error() {
        let response = create_test_response(404, "Not Found");
        assert_client_error(&response);
    }

    #[test]
    fn test_assert_server_error() {
        let response = create_test_response(500, "Internal Error");
        assert_server_error(&response);
    }

    #[test]
    fn test_assert_json_content_type() {
        let mut response = HttpResponse::ok();
        response
            .headers
            .insert("Content-Type".to_string(), "application/json".to_string());
        let test_response = TestResponse::Success(response);
        assert_json_content_type(&test_response);
    }

    #[test]
    fn test_assert_html_content_type() {
        let mut response = HttpResponse::ok();
        response
            .headers
            .insert("Content-Type".to_string(), "text/html".to_string());
        let test_response = TestResponse::Success(response);
        assert_html_content_type(&test_response);
    }

    #[test]
    fn test_assert_header() {
        let mut response = HttpResponse::ok();
        response
            .headers
            .insert("X-Custom".to_string(), "value".to_string());
        let test_response = TestResponse::Success(response);
        assert_header(&test_response, "X-Custom", "value");
    }

    #[test]
    fn test_assert_status_ranges() {
        let response_200 = create_test_response(200, "OK");
        let response_201 = create_test_response(201, "Created");
        let response_204 = create_test_response(204, "No Content");

        assert_success(&response_200);
        assert_success(&response_201);
        assert_success(&response_204);
    }

    #[test]
    fn test_assert_various_4xx_errors() {
        let response_400 = create_test_response(400, "Bad Request");
        let response_401 = create_test_response(401, "Unauthorized");
        let response_403 = create_test_response(403, "Forbidden");
        let response_404 = create_test_response(404, "Not Found");

        assert_client_error(&response_400);
        assert_client_error(&response_401);
        assert_client_error(&response_403);
        assert_client_error(&response_404);
    }

    #[test]
    fn test_assert_various_5xx_errors() {
        let response_500 = create_test_response(500, "Internal Server Error");
        let response_502 = create_test_response(502, "Bad Gateway");
        let response_503 = create_test_response(503, "Service Unavailable");

        assert_server_error(&response_500);
        assert_server_error(&response_502);
        assert_server_error(&response_503);
    }

    #[test]
    fn test_assert_body_contains_multiple_strings() {
        let response = create_test_response(200, "Hello World from Armature");
        assert_body_contains(&response, "Hello");
        assert_body_contains(&response, "World");
        assert_body_contains(&response, "Armature");
    }

    #[test]
    fn test_assert_empty_body() {
        let response = create_test_response(204, "");
        assert_body_contains(&response, ""); // Empty string should match
    }
}
