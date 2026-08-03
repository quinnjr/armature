//! Fuzz target for the `simd_parser` query-string helpers.
//!
//! These are **not** the live request path: `HttpRequest::query()` parses
//! through the crate-private `armature_core::query::parse`, and nothing in
//! `armature-core` routes a served request through
//! `parse_query_string_fast`/`parse_query_string_decoded`. They are public
//! API, so arbitrary input must still not panic — that is what this target
//! covers. The production query parser is fuzzed via `HttpRequest::query()` in
//! `http_request.rs`, which is the only way to reach it (`query::parse` is
//! `pub(crate)`).

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use armature_core::simd_parser::{parse_query_string_decoded, parse_query_string_fast, url_decode};

/// Arbitrary query string for fuzzing.
#[derive(Debug, Arbitrary)]
struct FuzzQuery {
    /// Raw query string
    raw: String,
    /// Individual parameters, used to build a synthetic query string
    params: Vec<(String, String)>,
}

fuzz_target!(|data: FuzzQuery| {
    // Test 1: Fast query string parsing (no decoding) - real production
    // parser used on the live request path.
    let params = parse_query_string_fast(&data.raw);
    let _ = params.len();

    // Test 2: Query string parsing with URL decoding - real production
    // parser used on the live request path.
    let decoded_params = parse_query_string_decoded(&data.raw);
    let _ = decoded_params.len();

    // Test 3: URL decode the raw string directly.
    let _ = url_decode(&data.raw);

    // Test 4: Look for common parameter names in the decoded map.
    let common_params = [
        "page", "limit", "offset", "sort", "order", "filter", "q", "search",
    ];
    for param in &common_params {
        let _ = decoded_params.get(*param);
    }

    // Test 5: Parse numeric values out of the decoded map.
    for (key, value) in &decoded_params {
        let _ = value.parse::<i32>();
        let _ = value.parse::<u64>();
        let _ = value.parse::<f64>();
        let _ = value.parse::<bool>();
        let _ = key.len();
    }

    // Test 6: Build a query string from structured params and re-parse it
    // through the real parsers.
    let mut built_query = String::new();
    for (i, (key, value)) in data.params.iter().enumerate() {
        if i > 0 {
            built_query.push('&');
        }
        built_query.push_str(key);
        built_query.push('=');
        built_query.push_str(value);
    }
    let _ = parse_query_string_fast(&built_query);
    let reparsed = parse_query_string_decoded(&built_query);
    let _ = reparsed.len();
    for (key, value) in &data.params {
        let _ = url_decode(key);
        let _ = url_decode(value);
    }

    // Test 7: Handle edge cases through the real parsers.
    let _ = parse_query_string_fast("");
    let _ = parse_query_string_decoded("");
    let _ = parse_query_string_fast("&");
    let _ = parse_query_string_decoded("&&&&");
    let _ = parse_query_string_decoded("key=");
    let _ = parse_query_string_decoded("=value");
    let _ = parse_query_string_decoded("key=value=extra");
});
