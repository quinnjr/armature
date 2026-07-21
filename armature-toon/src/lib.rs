//! TOON (Token-Oriented Object Notation) support for Armature.
//!
//! TOON is a serialization format optimized for Large Language Model (LLM)
//! applications, reducing token count by 30-60% compared to JSON.
//!
//! ## Features
//!
//! - **Token Efficiency**: Optimized format for LLM token reduction
//! - **Serde Compatible**: Works with existing Rust types
//! - **HTTP Integration**: Response helpers for TOON content
//! - **Comparison Tools**: Token counting and format comparison
//!
//! ## Quick Start
//!
//! ```rust
//! use armature_toon::{to_string, from_str};
//! use serde::{Serialize, Deserialize};
//!
//! #[derive(Serialize, Deserialize, Debug)]
//! struct User {
//!     id: u32,
//!     name: String,
//!     active: bool,
//! }
//!
//! let user = User {
//!     id: 123,
//!     name: "Alice".to_string(),
//!     active: true,
//! };
//!
//! // Serialize to TOON
//! let toon = to_string(&user).unwrap();
//!
//! // Deserialize from TOON
//! let parsed: User = from_str(&toon).unwrap();
//! ```
//!
//! ## Token Comparison
//!
//! ```rust
//! use armature_toon::compare_formats;
//! use serde::Serialize;
//!
//! #[derive(Serialize)]
//! struct Data {
//!     items: Vec<String>,
//!     count: usize,
//! }
//!
//! let data = Data {
//!     items: vec!["one".to_string(), "two".to_string()],
//!     count: 2,
//! };
//!
//! let comparison = compare_formats(&data).unwrap();
//! println!("JSON chars: {}", comparison.json_chars);
//! println!("TOON chars: {}", comparison.toon_chars);
//! println!("Reduction: {:.1}%", comparison.reduction_percent);
//! ```

mod error;

#[cfg(feature = "http")]
mod http;

pub use error::ToonError;
pub use serde_toon::{from_str, to_string};

#[cfg(feature = "http")]
pub use http::*;

use serde::{Serialize, de::DeserializeOwned};

/// TOON content type for HTTP responses.
pub const TOON_CONTENT_TYPE: &str = "application/toon";

/// Result type for TOON operations.
pub type Result<T> = std::result::Result<T, ToonError>;

/// Serialize a value to a TOON byte vector.
pub fn to_vec<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let s = to_string(value)?;
    Ok(s.into_bytes())
}

/// Deserialize a value from TOON bytes.
pub fn from_slice<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let s = std::str::from_utf8(bytes).map_err(|e| ToonError::Utf8Error(e.to_string()))?;
    from_str(s).map_err(ToonError::from)
}

/// Two-space indentation is TOON's baseline; pretty output widens nested
/// structures for readability while remaining parseable by [`from_str`].
const PRETTY_INDENT: usize = 4;

/// Serialize a value to a pretty-printed TOON string.
///
/// Unlike [`to_string`], which emits TOON's compact 2-space-indented form, this
/// widens nested-structure indentation (4 spaces) for readability. `serde_toon`'s
/// own `pretty` flag is a no-op for plain structs/maps, so the wider indent is
/// what makes the output genuinely distinct from [`to_string`]. The result still
/// round-trips through [`from_str`].
pub fn to_string_pretty<T: Serialize>(value: &T) -> Result<String> {
    let options = serde_toon::ToonOptions::pretty().with_indent(PRETTY_INDENT);
    serde_toon::to_string_with_options(value, options).map_err(ToonError::from)
}

/// Format comparison result.
#[derive(Debug, Clone)]
pub struct FormatComparison {
    /// JSON character count.
    pub json_chars: usize,
    /// TOON character count.
    pub toon_chars: usize,
    /// Estimated JSON tokens (chars / 4 rough estimate).
    pub json_tokens_estimate: usize,
    /// Estimated TOON tokens (chars / 4 rough estimate).
    pub toon_tokens_estimate: usize,
    /// Percentage reduction in characters.
    pub reduction_percent: f64,
}

/// Compare JSON and TOON serialization for a value.
///
/// # Example
///
/// ```rust
/// use armature_toon::compare_formats;
/// use serde::Serialize;
///
/// #[derive(Serialize)]
/// struct Data { name: String, count: u32 }
///
/// let data = Data { name: "test".to_string(), count: 42 };
/// let comparison = compare_formats(&data).unwrap();
/// println!("Reduction: {:.1}%", comparison.reduction_percent);
/// ```
pub fn compare_formats<T: Serialize>(value: &T) -> Result<FormatComparison> {
    let json_string =
        serde_json::to_string(value).map_err(|e| ToonError::SerializeError(e.to_string()))?;
    let toon_string = to_string(value)?;

    let json_chars = json_string.len();
    let toon_chars = toon_string.len();

    // Rough token estimate (GPT models use ~4 chars per token on average)
    let json_tokens_estimate = json_chars.div_ceil(4);
    let toon_tokens_estimate = toon_chars.div_ceil(4);

    let reduction_percent = if json_chars > 0 {
        ((json_chars - toon_chars) as f64 / json_chars as f64) * 100.0
    } else {
        0.0
    };

    Ok(FormatComparison {
        json_chars,
        toon_chars,
        json_tokens_estimate,
        toon_tokens_estimate,
        reduction_percent,
    })
}

/// Token counter for estimating LLM token usage.
#[derive(Debug, Default)]
pub struct TokenCounter {
    total_chars: usize,
    total_tokens_estimate: usize,
}

impl TokenCounter {
    /// Create a new token counter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a serialized value to the counter.
    pub fn add<T: Serialize>(&mut self, value: &T) -> Result<()> {
        let toon = to_string(value)?;
        self.total_chars += toon.len();
        self.total_tokens_estimate += toon.len().div_ceil(4);
        Ok(())
    }

    /// Add raw TOON string to the counter.
    pub fn add_raw(&mut self, toon: &str) {
        self.total_chars += toon.len();
        self.total_tokens_estimate += toon.len().div_ceil(4);
    }

    /// Get total character count.
    pub fn total_chars(&self) -> usize {
        self.total_chars
    }

    /// Get estimated token count.
    pub fn total_tokens_estimate(&self) -> usize {
        self.total_tokens_estimate
    }

    /// Reset the counter.
    pub fn reset(&mut self) {
        self.total_chars = 0;
        self.total_tokens_estimate = 0;
    }
}

/// TOON serializer with configuration options.
#[derive(Debug, Clone)]
pub struct ToonSerializer {
    /// Whether to include type hints.
    pub include_type_hints: bool,
    /// Whether to use compact mode.
    pub compact: bool,
}

impl Default for ToonSerializer {
    fn default() -> Self {
        Self {
            include_type_hints: false,
            compact: true,
        }
    }
}

impl ToonSerializer {
    /// Create a new TOON serializer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable type hints in output.
    pub fn with_type_hints(mut self) -> Self {
        self.include_type_hints = true;
        self
    }

    /// Disable compact mode.
    pub fn pretty(mut self) -> Self {
        self.compact = false;
        self
    }

    /// Serialize a value to TOON string.
    ///
    /// Applies the configured knobs: `pretty` (non-compact) emits newlines and
    /// indentation, and `include_type_hints` annotates array lengths with a `#`
    /// length marker (e.g. `[#3]:` instead of `[3]:`).
    pub fn serialize<T: Serialize>(&self, value: &T) -> Result<String> {
        let mut options = if self.compact {
            serde_toon::ToonOptions::new()
        } else {
            serde_toon::ToonOptions::pretty().with_indent(PRETTY_INDENT)
        };
        if self.include_type_hints {
            options = options.with_length_marker('#');
        }
        serde_toon::to_string_with_options(value, options).map_err(ToonError::from)
    }

    /// Serialize a value to TOON bytes, honoring the configured knobs.
    pub fn serialize_bytes<T: Serialize>(&self, value: &T) -> Result<Vec<u8>> {
        Ok(self.serialize(value)?.into_bytes())
    }
}

/// TOON deserializer with configuration options.
#[derive(Debug, Clone, Default)]
pub struct ToonDeserializer {
    /// Whether to be strict about unknown fields.
    pub strict: bool,
}

impl ToonDeserializer {
    /// Create a new TOON deserializer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable strict mode (fail on unknown fields).
    pub fn strict(mut self) -> Self {
        self.strict = true;
        self
    }

    /// Deserialize from TOON string.
    ///
    /// When [`strict`](Self::strict) is enabled, any field present in the input
    /// that is not represented by the target type `T` is rejected with a
    /// [`ToonError::DeserializeError`]. Strict mode re-serializes the parsed
    /// value to compare structures, so `T` must also implement [`Serialize`].
    pub fn deserialize<T: DeserializeOwned + Serialize>(&self, s: &str) -> Result<T> {
        let value: T = from_str(s).map_err(ToonError::from)?;
        if self.strict {
            let input: serde_toon::Value = from_str(s).map_err(ToonError::from)?;
            let roundtrip = serde_toon::to_value(&value).map_err(ToonError::from)?;
            check_no_unknown_fields(&input, &roundtrip)?;
        }
        Ok(value)
    }

    /// Deserialize from TOON bytes, honoring strict mode.
    pub fn deserialize_bytes<T: DeserializeOwned + Serialize>(&self, bytes: &[u8]) -> Result<T> {
        let s = std::str::from_utf8(bytes).map_err(|e| ToonError::Utf8Error(e.to_string()))?;
        self.deserialize(s)
    }
}

/// Verify that every field present in `input` is also present in the value
/// reconstructed from the deserialized target (`roundtrip`). A key that exists
/// in the input but not in the round-tripped structure is an unknown field.
fn check_no_unknown_fields(input: &serde_toon::Value, roundtrip: &serde_toon::Value) -> Result<()> {
    use serde_toon::Value;
    match (input, roundtrip) {
        (Value::Object(in_obj), Value::Object(rt_obj)) => {
            for (key, in_val) in in_obj.iter() {
                match rt_obj.get(key) {
                    Some(rt_val) => check_no_unknown_fields(in_val, rt_val)?,
                    None => {
                        return Err(ToonError::DeserializeError(format!(
                            "unknown field `{key}`"
                        )));
                    }
                }
            }
            Ok(())
        }
        (Value::Array(in_arr), Value::Array(rt_arr)) => {
            for (in_val, rt_val) in in_arr.iter().zip(rt_arr.iter()) {
                check_no_unknown_fields(in_val, rt_val)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Batch converter for converting between JSON and TOON.
pub struct BatchConverter;

impl BatchConverter {
    /// Convert JSON string to TOON string.
    pub fn json_to_toon(json: &str) -> Result<String> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| ToonError::DeserializeError(e.to_string()))?;
        to_string(&value).map_err(ToonError::from)
    }

    /// Convert TOON string to JSON string.
    pub fn toon_to_json(toon: &str) -> Result<String> {
        let value: serde_json::Value = from_str(toon)?;
        serde_json::to_string(&value).map_err(|e| ToonError::SerializeError(e.to_string()))
    }

    /// Convert TOON string to pretty JSON string.
    pub fn toon_to_json_pretty(toon: &str) -> Result<String> {
        let value: serde_json::Value = from_str(toon)?;
        serde_json::to_string_pretty(&value).map_err(|e| ToonError::SerializeError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct TestUser {
        id: u32,
        name: String,
        active: bool,
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct TestData {
        users: Vec<TestUser>,
        total: usize,
    }

    #[test]
    fn test_serialize_deserialize() {
        let user = TestUser {
            id: 123,
            name: "Alice".to_string(),
            active: true,
        };

        let toon = to_string(&user).unwrap();
        let parsed: TestUser = from_str(&toon).unwrap();
        assert_eq!(user, parsed);
    }

    #[test]
    fn test_to_vec_from_slice() {
        let user = TestUser {
            id: 456,
            name: "Bob".to_string(),
            active: false,
        };

        let bytes = to_vec(&user).unwrap();
        let parsed: TestUser = from_slice(&bytes).unwrap();
        assert_eq!(user, parsed);
    }

    #[test]
    fn test_compare_formats() {
        let data = TestData {
            users: vec![
                TestUser {
                    id: 1,
                    name: "Alice".to_string(),
                    active: true,
                },
                TestUser {
                    id: 2,
                    name: "Bob".to_string(),
                    active: false,
                },
            ],
            total: 2,
        };

        let comparison = compare_formats(&data).unwrap();
        assert!(comparison.json_chars > 0);
        assert!(comparison.toon_chars > 0);
        // TOON should generally be smaller
        println!(
            "JSON: {} chars, TOON: {} chars, Reduction: {:.1}%",
            comparison.json_chars, comparison.toon_chars, comparison.reduction_percent
        );
    }

    #[test]
    fn test_token_counter() {
        let mut counter = TokenCounter::new();

        let user = TestUser {
            id: 1,
            name: "Test".to_string(),
            active: true,
        };

        counter.add(&user).unwrap();
        assert!(counter.total_chars() > 0);
        assert!(counter.total_tokens_estimate() > 0);

        counter.reset();
        assert_eq!(counter.total_chars(), 0);
    }

    #[test]
    fn test_serializer() {
        let serializer = ToonSerializer::new();
        let user = TestUser {
            id: 1,
            name: "Test".to_string(),
            active: true,
        };

        let toon = serializer.serialize(&user).unwrap();
        assert!(!toon.is_empty());
    }

    #[test]
    fn test_deserializer() {
        let user = TestUser {
            id: 1,
            name: "Test".to_string(),
            active: true,
        };
        let toon = to_string(&user).unwrap();

        let deserializer = ToonDeserializer::new();
        let parsed: TestUser = deserializer.deserialize(&toon).unwrap();
        assert_eq!(user, parsed);
    }

    #[test]
    fn test_pretty_differs_and_roundtrips() {
        // Regression: `to_string_pretty` must produce a distinct, multi-line
        // rendering from the compact `to_string`, and still round-trip.
        let data = TestData {
            users: vec![
                TestUser {
                    id: 1,
                    name: "Alice".to_string(),
                    active: true,
                },
                TestUser {
                    id: 2,
                    name: "Bob".to_string(),
                    active: false,
                },
            ],
            total: 2,
        };

        let compact = to_string(&data).unwrap();
        let pretty = to_string_pretty(&data).unwrap();

        // Pretty output must be genuinely different formatting, not identical.
        assert_ne!(
            compact, pretty,
            "pretty output must differ from compact output"
        );
        // Pretty adds newlines/indentation for the nested structure.
        assert!(
            pretty.contains('\n'),
            "pretty output should contain newlines"
        );

        // And it must still round-trip back to the original value.
        let parsed: TestData = from_str(&pretty).unwrap();
        assert_eq!(data, parsed);
    }

    #[test]
    fn test_serializer_pretty_and_type_hints_applied() {
        // Regression: the ToonSerializer knobs must actually affect output.
        let data = TestData {
            users: vec![TestUser {
                id: 1,
                name: "Alice".to_string(),
                active: true,
            }],
            total: 1,
        };

        let compact = ToonSerializer::new().serialize(&data).unwrap();
        let pretty = ToonSerializer::new().pretty().serialize(&data).unwrap();
        assert_ne!(
            compact, pretty,
            "pretty() knob must change serializer output"
        );
        assert!(pretty.contains('\n'));

        // Type hints add a `#` array length marker.
        let hinted = ToonSerializer::new()
            .with_type_hints()
            .serialize(&data)
            .unwrap();
        assert_ne!(
            compact, hinted,
            "with_type_hints() knob must change serializer output"
        );
        assert!(
            hinted.contains("[#"),
            "type hints should emit a `#` length marker, got: {hinted}"
        );
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct TestUserPlus {
        id: u32,
        name: String,
        active: bool,
        role: String,
    }

    #[test]
    fn test_strict_rejects_unknown_fields() {
        // Input carries an extra `role` field not present on TestUser.
        let plus = TestUserPlus {
            id: 7,
            name: "Alice".to_string(),
            active: true,
            role: "admin".to_string(),
        };
        let toon = to_string(&plus).unwrap();

        // Non-strict: unknown fields are ignored (serde default).
        let lenient: TestUser = ToonDeserializer::new().deserialize(&toon).unwrap();
        assert_eq!(
            lenient,
            TestUser {
                id: 7,
                name: "Alice".to_string(),
                active: true,
            }
        );

        // Strict: the extra `role` field must be rejected.
        let strict_result: Result<TestUser> = ToonDeserializer::new().strict().deserialize(&toon);
        match strict_result {
            Err(ToonError::DeserializeError(msg)) => {
                assert!(msg.contains("role"), "error should name the field: {msg}");
            }
            other => panic!("strict mode should reject unknown field, got: {other:?}"),
        }
    }

    #[test]
    fn test_parse_error_classified_as_deserialize() {
        // A malformed / type-mismatched parse must classify as a
        // deserialization error, never a serialization error.
        let bad = "id: not_a_number\nname: Alice\nactive: true";
        let result: Result<TestUser> = ToonDeserializer::new().deserialize(bad);
        match result {
            Err(ToonError::DeserializeError(_)) => {}
            Err(ToonError::SerializeError(msg)) => {
                panic!("parse failure misclassified as SerializeError: {msg}")
            }
            other => panic!("expected DeserializeError, got: {other:?}"),
        }
    }

    #[test]
    fn test_batch_converter_json_to_toon() {
        let json = r#"{"id":123,"name":"Alice","active":true}"#;
        let toon = BatchConverter::json_to_toon(json).unwrap();
        assert!(!toon.is_empty());

        // Convert back
        let json_back = BatchConverter::toon_to_json(&toon).unwrap();
        assert!(json_back.contains("123"));
        assert!(json_back.contains("Alice"));
    }
}
