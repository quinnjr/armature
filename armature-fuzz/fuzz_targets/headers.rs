//! Fuzz target for HTTP header parsing.
//!
//! `armature_core::simd_parser::{parse_headers, is_valid_header_name}` have
//! no confirmed production caller: a workspace-wide grep for
//! `parse_headers`/`is_valid_header_name` turns up nothing outside this
//! fuzz target, benches, and `simd_parser.rs` itself. Real HTTP/1 request
//! parsing goes through `hyper` (an `armature-core` dependency), not this
//! module. These functions are, however, `pub fn`s in the `pub mod
//! simd_parser`, and the module's own doc comment frames it as a general
//! parsing-utility module ("complements Hyper's built-in parsing") with
//! usage examples aimed at external callers - i.e. they are part of
//! `armature_core`'s public API surface even though nothing inside the
//! crate currently calls them. This target fuzzes them as public library
//! utilities (valuable for catching panics/UB in code reachable by external
//! consumers, and in case they become hot-path functions later), not as
//! "the" live request-handling path.

#![no_main]

#[path = "common.rs"]
mod common;

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use armature_core::simd_parser::{is_valid_header_name, parse_headers};

/// Arbitrary input for header parsing.
#[derive(Debug, Arbitrary)]
struct FuzzHeaders {
    /// Raw HTTP request buffer (request line + headers) fed directly into
    /// the SIMD-optimized header parser (`simd_parser::parse_headers`).
    buf: Vec<u8>,
    /// Candidate header names to validate directly.
    names: Vec<Vec<u8>>,
    /// Fuzzer-controlled method for the synthesized request line in Test 5.
    method: String,
    /// Fuzzer-controlled HTTP/1.x minor version digit for Test 5.
    version_digit: u8,
}

fuzz_target!(|data: FuzzHeaders| {
    // Test 1: Parse arbitrary bytes as HTTP headers using
    // `simd_parser::parse_headers` (public API, no confirmed internal
    // caller - see module doc above). Should never panic regardless of how
    // malformed the input is.
    if let Ok(headers) = parse_headers(&data.buf) {
        for (name, value) in &headers {
            // Exercise the header-name validator on names the parser
            // actually produced.
            let _ = is_valid_header_name(name.as_bytes());
            let _ = value.len();
        }
    }

    // Test 2: Validate arbitrary byte sequences as header names directly.
    for name in &data.names {
        let _ = is_valid_header_name(name);
    }

    // Test 3: Validate newline-delimited chunks of the raw buffer, mirroring
    // how a caller might validate individual header lines.
    for line in data.buf.split(|&b| b == b'\n') {
        let _ = is_valid_header_name(line);
    }

    // Test 4: Re-parse the raw buffer with a synthesized, well-formed
    // request line prepended, to reach deeper into the header-parsing path
    // for inputs that are pure header garbage.
    let mut synthesized = common::synth_request_line("GET", "/", 1);
    synthesized.extend_from_slice(&data.buf);
    let _ = parse_headers(&synthesized);

    // Test 5: Same idea as Test 4, but the method and HTTP minor version in
    // the request line are also fuzzer-controlled (target path is kept
    // fixed and simple so the line stays structurally parseable), so
    // request-line fuzzing and header fuzzing aren't fully decoupled.
    let method = if data.method.is_empty() || data.method.len() > 16 {
        "GET"
    } else {
        data.method.as_str()
    };
    let mut fuzzed_line = common::synth_request_line(method, "/", data.version_digit);
    fuzzed_line.extend_from_slice(&data.buf);
    let _ = parse_headers(&fuzzed_line);
});
