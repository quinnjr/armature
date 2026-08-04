//! Fuzz target for this crate's JSON layer.
//!
//! `armature_core::json` is not a thin re-export: it is the seam that lets the
//! `simd-json` feature swap the parser out from under the framework, and it
//! carries arms `serde_json` does not have (`from_owned` and `from_slice_mut`
//! parse in place and mutate the caller's buffer, `from_slice` copies first to
//! avoid doing so). `HttpResponse::json` writes response bodies through
//! `json::to_vec` and `HttpRequest::json` reads request bodies back through
//! `json::from_slice`, so those two are the framework's real JSON surface.
//!
//! This target fuzzes that surface rather than `serde_json` itself, and pins
//! the properties the crate owns: the two halves of the wire round-trip agree,
//! the module's alternate entry points agree with each other, and reading a
//! request body does not disturb it.

#![no_main]

use arbitrary::Arbitrary;
use bytes::Bytes;
use libfuzzer_sys::fuzz_target;

use serde::{Deserialize, Serialize};

use armature_core::http::{HttpRequest, HttpResponse};
use armature_core::json as armature_json;

/// Test struct for JSON serialization/deserialization.
///
/// `PartialEq` is what makes the round-trip assertable: without it the
/// reparsed value could only be dropped.
#[derive(Debug, PartialEq, Serialize, Deserialize, Arbitrary)]
struct TestData {
    id: u64,
    name: String,
    values: Vec<i32>,
    nested: Option<Box<NestedData>>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Arbitrary)]
struct NestedData {
    key: String,
    value: f64,
    tags: Vec<String>,
}

/// Whether `serde_json` reads back the exact `f64` it wrote.
///
/// It does not, always. `serde_json` renders `f64::from_bits(0x360947ff00fff7a8)`
/// as `2.1622653505009378e-48` — the correct shortest form, since Rust's own
/// `str::parse` recovers those bits from it — but its *parser* returns the
/// neighbouring value. The loss is a full ULP and belongs to `serde_json`, not
/// to this crate: `armature_core::json` is a thin seam over it and reproduces
/// whatever it does.
///
/// Round-trip equality is therefore asserted only where the encoding is
/// faithful. Skipping the handful of values it is not keeps the assertion
/// strong for every other value, instead of dropping it entirely and losing the
/// check that the response builder and the body extractor agree.
fn survives_serde_json(value: f64) -> bool {
    serde_json::to_string(&value)
        .ok()
        .and_then(|text| serde_json::from_str::<f64>(&text).ok())
        .is_some_and(|back| back.to_bits() == value.to_bits())
}

impl TestData {
    /// Whether JSON can express every value in here.
    ///
    /// JSON has no spelling for NaN or the infinities, so an encoder has to
    /// write them as `null` — which no longer reads back as an `f64`. That is a
    /// property of the format, not of this crate, so round-trip equality is only
    /// claimed where the format can carry the value; the encode side is still
    /// exercised unconditionally below.
    fn is_json_representable(&self) -> bool {
        self.nested
            .as_ref()
            .is_none_or(|nested| nested.value.is_finite() && survives_serde_json(nested.value))
    }
}

/// Raw bytes for JSON parsing.
#[derive(Debug, Arbitrary)]
struct FuzzJson {
    /// Raw bytes to parse as JSON
    raw: Vec<u8>,
    /// Structured data to serialize
    structured: TestData,
}

fuzz_target!(|data: FuzzJson| {
    // Test 1: untrusted bytes through the entry point handlers actually use.
    // `HttpRequest::json` is what every handler taking a JSON body calls, and it
    // reaches `json::from_slice`, whose `simd-json` arm has to copy the body
    // into a scratch buffer because it parses destructively.
    let request = HttpRequest::with_bytes_body("POST", "/fuzz", Bytes::from(data.raw.clone()));
    let _ = request.json::<serde_json::Value>();
    let _ = request.json::<TestData>();
    let _ = request.json::<Vec<serde_json::Value>>();
    let _ = request.json::<std::collections::HashMap<String, serde_json::Value>>();

    // Parsing a body must not consume or rewrite it: `HttpRequest::json` takes
    // `&self`, so a handler is free to parse twice, or to parse and then forward
    // the original bytes. The in-place parsers make that a real hazard rather
    // than a theoretical one.
    assert_eq!(
        request.body_ref(),
        &data.raw[..],
        "parsing a request body must leave the body untouched"
    );

    // Test 2: the wire round-trip this framework owns. `HttpResponse::json`
    // encodes with `json::to_vec` and `HttpRequest::json` decodes with
    // `json::from_slice`, so feeding one into the other pins the response
    // builder and the body extractor against each other.
    if let Ok(response) = HttpResponse::json(&data.structured) {
        assert_eq!(response.status, 200, "HttpResponse::json is a 200");
        assert_eq!(
            response.headers.get("Content-Type").map(String::as_str),
            Some("application/json"),
            "a JSON response must advertise its own content type"
        );

        if data.structured.is_json_representable() {
            let echoed = HttpRequest::with_bytes_body("POST", "/fuzz", response.body.clone());
            let parsed: TestData = echoed
                .json()
                .expect("a body written by HttpResponse::json must parse back");
            assert_eq!(
                parsed, data.structured,
                "a value serialized into a response body must deserialize back equal"
            );
        }
    }

    if !data.structured.is_json_representable() {
        return;
    }

    // Test 3: the module's alternate entry points exist only as allocation
    // optimizations of one another, so any disagreement between them is a bug
    // in this crate rather than in whichever parser is compiled in.
    let bytes = armature_json::to_vec(&data.structured).expect("finite values encode");

    let via_slice: TestData = armature_json::from_slice(&bytes).expect("to_vec output parses");
    assert_eq!(via_slice, data.structured, "from_slice must round-trip to_vec");

    // `from_slice_mut` parses destructively, so it gets its own copy; the
    // buffer is garbage afterwards and is not reused.
    let mut scratch = bytes.clone();
    let via_slice_mut: TestData =
        armature_json::from_slice_mut(&mut scratch).expect("to_vec output parses in place");
    assert_eq!(
        via_slice_mut, via_slice,
        "from_slice_mut must agree with from_slice"
    );

    let via_owned: TestData =
        armature_json::from_owned(bytes.clone()).expect("to_vec output parses when owned");
    assert_eq!(
        via_owned, via_slice,
        "from_owned must agree with from_slice"
    );

    // `to_string` and `to_vec` are the same encoder behind different return
    // types, and `to_vec_with_capacity` only pre-sizes the buffer.
    let text = armature_json::to_string(&data.structured).expect("finite values encode");
    assert_eq!(
        text.as_bytes(),
        &bytes[..],
        "to_string and to_vec must emit the same document"
    );
    let sized = armature_json::to_vec_with_capacity(&data.structured, bytes.len())
        .expect("finite values encode");
    assert_eq!(
        sized, bytes,
        "pre-sizing the buffer must not change the output"
    );

    let via_str: TestData = armature_json::from_str(&text).expect("to_string output parses");
    assert_eq!(via_str, via_slice, "from_str must agree with from_slice");

    // Pretty printing is the one arm that never uses simd-json; it may only add
    // whitespace, so the parsed value has to be unchanged.
    let pretty = armature_json::to_string_pretty(&data.structured).expect("finite values encode");
    let via_pretty: TestData = armature_json::from_str(&pretty).expect("pretty output parses");
    assert_eq!(
        via_pretty, via_slice,
        "pretty printing must not change the value"
    );

    // Test 4: the `Value` detour must land in the same place as the byte path,
    // and a struct always encodes as an object carrying its declared fields.
    let value = armature_json::to_value(&data.structured).expect("finite values encode");
    assert!(value.is_object(), "a struct encodes as a JSON object");
    for field in ["id", "name", "values", "nested"] {
        assert!(
            value.get(field).is_some(),
            "the encoded object is missing the field {field:?}"
        );
    }
    let via_value: TestData = armature_json::from_value(value).expect("to_value output converts");
    assert_eq!(
        via_value, via_slice,
        "the Value detour must agree with the byte path"
    );
});
