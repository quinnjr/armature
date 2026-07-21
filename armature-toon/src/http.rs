//! HTTP integration for TOON responses.

use crate::{TOON_CONTENT_TYPE, ToonError, to_string};
use armature_core::http::HttpResponse;
use bytes::Bytes;
use serde::Serialize;

/// TOON response wrapper for Armature HTTP responses.
pub struct Toon<T>(pub T);

impl<T: Serialize> Toon<T> {
    /// Create a new TOON response.
    pub fn new(value: T) -> Self {
        Toon(value)
    }

    /// Convert to HTTP response.
    pub fn into_response(self) -> Result<HttpResponse, ToonError> {
        let body = to_string(&self.0).map_err(ToonError::from_serialize)?;
        let mut response = HttpResponse::new(200).with_bytes_body(Bytes::from(body));
        response
            .headers
            .insert("Content-Type".to_string(), TOON_CONTENT_TYPE.to_string());
        Ok(response)
    }
}

/// Extension trait for HttpResponse to add TOON support.
pub trait ToonResponseExt {
    /// Create a TOON response from a serializable value.
    fn toon<T: Serialize>(value: T) -> Result<HttpResponse, ToonError>;

    /// Create a TOON response with a custom status code.
    fn toon_with_status<T: Serialize>(status: u16, value: T) -> Result<HttpResponse, ToonError>;
}

impl ToonResponseExt for HttpResponse {
    fn toon<T: Serialize>(value: T) -> Result<HttpResponse, ToonError> {
        let body = to_string(&value).map_err(ToonError::from_serialize)?;
        let mut response = HttpResponse::new(200).with_bytes_body(Bytes::from(body));
        response
            .headers
            .insert("Content-Type".to_string(), TOON_CONTENT_TYPE.to_string());
        Ok(response)
    }

    fn toon_with_status<T: Serialize>(status: u16, value: T) -> Result<HttpResponse, ToonError> {
        let body = to_string(&value).map_err(ToonError::from_serialize)?;
        let mut response = HttpResponse::new(status).with_bytes_body(Bytes::from(body));
        response
            .headers
            .insert("Content-Type".to_string(), TOON_CONTENT_TYPE.to_string());
        Ok(response)
    }
}

/// Content negotiation helper for TOON.
pub struct ToonContentNegotiator;

impl ToonContentNegotiator {
    /// Check if the request accepts TOON format.
    pub fn accepts_toon(accept_header: Option<&str>) -> bool {
        match accept_header {
            Some(accept) => {
                accept.contains(TOON_CONTENT_TYPE)
                    || accept.contains("application/*")
                    || accept.contains("*/*")
            }
            None => false,
        }
    }

    /// Check if the request prefers TOON over JSON.
    pub fn prefers_toon(accept_header: Option<&str>) -> bool {
        match accept_header {
            Some(accept) => {
                // Simple heuristic: TOON appears before JSON in Accept header
                let toon_pos = accept.find(TOON_CONTENT_TYPE);
                let json_pos = accept.find("application/json");

                match (toon_pos, json_pos) {
                    (Some(t), Some(j)) => t < j,
                    (Some(_), None) => true,
                    _ => false,
                }
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct TestData {
        name: String,
        value: i32,
    }

    #[test]
    fn test_toon_response() {
        let data = TestData {
            name: "test".to_string(),
            value: 42,
        };

        let response = HttpResponse::toon(data).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(
            response.headers.get("Content-Type"),
            Some(&TOON_CONTENT_TYPE.to_string())
        );
    }

    #[test]
    fn test_toon_wrapper() {
        let data = TestData {
            name: "test".to_string(),
            value: 42,
        };

        let toon = Toon::new(data);
        let response = toon.into_response().unwrap();
        assert_eq!(response.status, 200);
    }

    #[test]
    fn test_accepts_toon() {
        assert!(ToonContentNegotiator::accepts_toon(Some(
            "application/toon"
        )));
        assert!(ToonContentNegotiator::accepts_toon(Some(
            "application/json, application/toon"
        )));
        assert!(ToonContentNegotiator::accepts_toon(Some("*/*")));
        assert!(!ToonContentNegotiator::accepts_toon(Some(
            "application/json"
        )));
        assert!(!ToonContentNegotiator::accepts_toon(None));
    }

    #[test]
    fn test_prefers_toon() {
        assert!(ToonContentNegotiator::prefers_toon(Some(
            "application/toon, application/json"
        )));
        assert!(!ToonContentNegotiator::prefers_toon(Some(
            "application/json, application/toon"
        )));
        assert!(!ToonContentNegotiator::prefers_toon(Some(
            "application/json"
        )));
    }
}
