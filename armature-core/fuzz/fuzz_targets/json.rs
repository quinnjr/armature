//! Fuzz target for JSON parsing and serialization.
//!
//! Tests JSON handling with arbitrary input to ensure
//! robust error handling.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use serde::{Deserialize, Serialize};

/// Test struct for JSON serialization/deserialization.
#[derive(Debug, Serialize, Deserialize, Arbitrary)]
struct TestData {
    id: u64,
    name: String,
    values: Vec<i32>,
    nested: Option<Box<NestedData>>,
}

#[derive(Debug, Serialize, Deserialize, Arbitrary)]
struct NestedData {
    key: String,
    value: f64,
    tags: Vec<String>,
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
    // Test 1: Parse arbitrary bytes as JSON
    // Should handle malformed JSON gracefully
    if let Ok(s) = std::str::from_utf8(&data.raw) {
        // Try parsing as generic Value
        let _ = serde_json::from_str::<serde_json::Value>(s);

        // Try parsing as TestData
        let _ = serde_json::from_str::<TestData>(s);

        // Try parsing as array
        let _ = serde_json::from_str::<Vec<serde_json::Value>>(s);

        // Try parsing as object
        let _ = serde_json::from_str::<std::collections::HashMap<String, serde_json::Value>>(s);
    }

    // Test 2: Serialize structured data
    // Should always succeed for valid Rust structs
    if let Ok(json_string) = serde_json::to_string(&data.structured) {
        // Verify round-trip
        let _ = serde_json::from_str::<TestData>(&json_string);

        // Verify pretty printing
        let _ = serde_json::to_string_pretty(&data.structured);
    }

    // Test 3: Serialize to bytes
    if let Ok(json_bytes) = serde_json::to_vec(&data.structured) {
        let _ = serde_json::from_slice::<TestData>(&json_bytes);
    }

    // Test 4: JSON Value operations
    if let Ok(value) = serde_json::to_value(&data.structured) {
        // Test accessors
        let _ = value.get("id");
        let _ = value.get("name");
        let _ = value.get("values");
        let _ = value.get("nested");

        // Test type checks
        let _ = value.is_object();
        let _ = value.is_array();
        let _ = value.is_string();
        let _ = value.is_number();
    }
});

