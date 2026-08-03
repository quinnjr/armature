//! Fuzz target for URL and path parsing.
//!
//! `armature_core::simd_parser::{split_uri, split_path, parse_request_line}`
//! have no confirmed production caller: a workspace-wide grep for these
//! names turns up nothing outside this fuzz target, benches, and
//! `simd_parser.rs` itself (real HTTP/1 parsing goes through `hyper`, not
//! this module). They are, however, `pub fn`s in the `pub mod simd_parser`,
//! whose module doc frames it as a general parsing-utility module intended
//! to complement Hyper - i.e. part of `armature_core`'s public API surface.
//! This target exercises them as public library utilities worth fuzzing for
//! robustness, not as functions confirmed to run on the live request path.
//! `url_decode` *is* used in production (transitively via
//! `parse_query_string_decoded`, see `fuzz_query_params.rs`).

#![no_main]

#[path = "common.rs"]
mod common;

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use armature_core::simd_parser::{parse_request_line, split_path, split_uri, url_decode};

/// Arbitrary URL components for fuzzing.
#[derive(Debug, Arbitrary)]
struct FuzzUrl {
    /// Raw URI (path + optional query), as would appear in a request line.
    uri: String,
    /// Raw bytes for a full HTTP request line.
    request_line_buf: Vec<u8>,
}

fuzz_target!(|data: FuzzUrl| {
    // Test 1: Split a URI into path + query using
    // `simd_parser::split_uri` (public API, no confirmed internal caller -
    // see module doc above).
    let (path, query) = split_uri(&data.uri);
    let _ = path.len();
    let _ = query.map(|q| q.len());

    // Test 2: Path segmentation via `simd_parser::split_path` (same status
    // as `split_uri`) on both the raw URI and the split-out path.
    let segments: Vec<&str> = split_path(&data.uri).collect();
    let _ = segments.len();
    let path_segments: Vec<&str> = split_path(path).collect();
    let _ = path_segments.len();

    // Test path traversal-looking segments (should never panic to inspect).
    let has_traversal = path_segments.iter().any(|s| *s == ".." || *s == ".");
    let _ = has_traversal;

    // Test 3: URL decoding - `url_decode` IS a genuine production helper
    // (called from `parse_query_string_decoded`, which is used on the live
    // request path - see `fuzz_query_params.rs`) - of the raw string and of
    // the split-out components.
    let _ = url_decode(&data.uri);
    let _ = url_decode(path);
    if let Some(q) = query {
        let _ = url_decode(q);
    }

    // Test 4: Full HTTP request-line parsing (method, path, version) via
    // `simd_parser::parse_request_line` (public API, no confirmed internal
    // caller - see module doc above).
    let _ = parse_request_line(&data.request_line_buf);

    // Also try the request line built from the fuzzed URI directly, to
    // reach deeper into the parser for structurally plausible input. Uses
    // the same request-line synthesis helper as `fuzz_headers.rs`.
    let mut synthesized = common::synth_request_line("GET", &data.uri, 1);
    synthesized.extend_from_slice(b"\r\n");
    let _ = parse_request_line(&synthesized);

    // Test 5: Path prefix matching.
    let test_prefixes = ["/api", "/v1", "/users", "/"];
    for prefix in &test_prefixes {
        let _ = data.uri.starts_with(prefix);
    }

    // Test 6: Path suffix matching.
    let test_suffixes = [".json", ".xml", ".html", "/"];
    for suffix in &test_suffixes {
        let _ = data.uri.ends_with(suffix);
    }
});
