//! Fuzz target for HTTP response building.
//!
//! Tests response creation with arbitrary data to ensure
//! no panics occur during response construction.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use armature_core::http::HttpResponse;
use bytes::Bytes;

/// Arbitrary HTTP response for fuzzing.
#[derive(Debug, Arbitrary)]
struct FuzzResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fuzz_target!(|data: FuzzResponse| {
    if data.body.len() > 1_000_000 {
        return;
    }

    // Clamp status code to valid HTTP range
    let status = data.status.clamp(100, 599);

    // Create response - should not panic
    let mut response = HttpResponse::new(status).with_bytes_body(Bytes::from(data.body.clone()));

    // Add headers - should handle arbitrary header names/values
    for (key, value) in &data.headers {
        // Skip obviously invalid headers (empty names)
        if !key.is_empty() {
            response.headers.insert(key.clone(), value.clone());
        }
    }

    // Test accessors
    let _ = response.status;
    let _ = response.body.len();
    let _ = response.headers.len();

    // Test common response builders
    let _ = HttpResponse::ok();
    let _ = HttpResponse::not_found();
    let _ = HttpResponse::internal_server_error();
    let _ = HttpResponse::bad_request();

    // The `Vec<u8>` body setter takes a different path than `with_bytes_body`.
    let _ = HttpResponse::new(status).with_body(data.body.clone());

    // Test JSON response with arbitrary body
    if !data.body.is_empty() {
        // Try to interpret body as JSON - should not panic
        if let Ok(s) = std::str::from_utf8(&data.body) {
            let _ = serde_json::from_str::<serde_json::Value>(s);
        }
    }
});
