//! High-Performance JSON Serialization and Deserialization
//!
//! This module provides a unified interface for JSON operations that can use
//! either `serde_json` (default) or `simd-json` (with the `simd-json` feature).
//!
//! ## Performance
//!
//! The `simd-json` feature provides SIMD-accelerated JSON parsing on x86_64
//! CPUs with AVX2 support. Performance benefits depend on payload size:
//!
//! - **Small payloads (<100 bytes)**: Similar performance to serde_json
//! - **Medium payloads (100-1KB)**: Up to 15% faster serialization
//! - **Large payloads (>1KB)**: Up to 20% faster serialization
//! - **Very large payloads (>10KB)**: Up to 2-3x faster when using `from_slice_mut`
//!
//! Note: Deserialization with `from_slice` requires copying the input buffer for
//! simd-json's in-place parsing. Use `from_slice_mut` when possible for best performance.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use armature_core::json;
//!
//! // Serialize to Vec<u8>
//! let bytes = json::to_vec(&data)?;
//!
//! // Deserialize from bytes
//! let data: MyType = json::from_slice(&bytes)?;
//!
//! // Serialize to String
//! let string = json::to_string(&data)?;
//!
//! // Pretty-print
//! let pretty = json::to_string_pretty(&data)?;
//! ```
//!
//! ## Feature Flags
//!
//! - `simd-json`: Use SIMD-accelerated JSON parsing (requires x86_64 with AVX2)
//!
//! ## Example with Feature Flag
//!
//! ```toml
//! [dependencies]
//! armature-core = { version = "0.1", features = ["simd-json"] }
//! ```

use serde::{Serialize, de::DeserializeOwned};

/// Error type for JSON operations.
///
/// This wraps the underlying JSON library's error type.
#[derive(Debug)]
pub struct JsonError {
    message: String,
}

impl std::fmt::Display for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for JsonError {}

impl From<serde_json::Error> for JsonError {
    fn from(err: serde_json::Error) -> Self {
        JsonError {
            message: err.to_string(),
        }
    }
}

#[cfg(feature = "simd-json")]
impl From<simd_json::Error> for JsonError {
    fn from(err: simd_json::Error) -> Self {
        JsonError {
            message: err.to_string(),
        }
    }
}

/// Result type for JSON operations.
pub type Result<T> = std::result::Result<T, JsonError>;

// ============================================================================
// Serialization Functions
// ============================================================================

/// Serialize a value to a JSON byte vector.
///
/// This is the primary serialization function for response bodies.
///
/// # Performance
///
/// With `simd-json`, serialization is typically 1.5-2x faster than `serde_json`.
///
/// # Example
///
/// ```rust,ignore
/// let data = User { name: "John", age: 30 };
/// let bytes = json::to_vec(&data)?;
/// ```
#[inline]
pub fn to_vec<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    #[cfg(feature = "simd-json")]
    {
        simd_json::to_vec(value).map_err(Into::into)
    }
    #[cfg(not(feature = "simd-json"))]
    {
        serde_json::to_vec(value).map_err(Into::into)
    }
}

/// Serialize a value to a JSON byte vector with pre-allocated capacity.
///
/// Use this when you have a reasonable estimate of the output size.
///
/// # Example
///
/// ```rust,ignore
/// let data = User { name: "John", age: 30 };
/// let bytes = json::to_vec_with_capacity(&data, 256)?;
/// ```
#[inline]
pub fn to_vec_with_capacity<T: Serialize>(value: &T, capacity: usize) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(capacity);

    #[cfg(feature = "simd-json")]
    {
        simd_json::to_writer(&mut buf, value)?;
    }
    #[cfg(not(feature = "simd-json"))]
    {
        serde_json::to_writer(&mut buf, value)?;
    }

    Ok(buf)
}

/// Serialize a value to a JSON string.
///
/// # Example
///
/// ```rust,ignore
/// let data = User { name: "John", age: 30 };
/// let json_str = json::to_string(&data)?;
/// ```
#[inline]
pub fn to_string<T: Serialize>(value: &T) -> Result<String> {
    #[cfg(feature = "simd-json")]
    {
        simd_json::to_string(value).map_err(Into::into)
    }
    #[cfg(not(feature = "simd-json"))]
    {
        serde_json::to_string(value).map_err(Into::into)
    }
}

/// Serialize a value to a pretty-printed JSON string.
///
/// This is useful for debugging and human-readable output.
/// Note: simd-json doesn't have a pretty-print option, so this falls back to serde_json.
///
/// # Example
///
/// ```rust,ignore
/// let data = User { name: "John", age: 30 };
/// let pretty_json = json::to_string_pretty(&data)?;
/// ```
#[inline]
pub fn to_string_pretty<T: Serialize>(value: &T) -> Result<String> {
    // simd-json doesn't support pretty printing, so always use serde_json
    serde_json::to_string_pretty(value).map_err(Into::into)
}

/// Serialize a value to a writer.
///
/// # Example
///
/// ```rust,ignore
/// let mut buffer = Vec::new();
/// json::to_writer(&mut buffer, &data)?;
/// ```
#[inline]
pub fn to_writer<W: std::io::Write, T: Serialize>(writer: W, value: &T) -> Result<()> {
    #[cfg(feature = "simd-json")]
    {
        simd_json::to_writer(writer, value).map_err(Into::into)
    }
    #[cfg(not(feature = "simd-json"))]
    {
        serde_json::to_writer(writer, value).map_err(Into::into)
    }
}

// ============================================================================
// Deserialization Functions
// ============================================================================

/// Deserialize a value from a JSON byte slice.
///
/// This is the primary deserialization function for request bodies.
///
/// # Performance
///
/// With `simd-json`, deserialization is typically 2-3x faster than `serde_json`,
/// especially for large payloads.
///
/// # Example
///
/// ```rust,ignore
/// let data: User = json::from_slice(&bytes)?;
/// ```
#[inline]
pub fn from_slice<T: DeserializeOwned>(slice: &[u8]) -> Result<T> {
    #[cfg(feature = "simd-json")]
    {
        // simd-json requires a mutable buffer for in-place parsing
        // We need to clone the slice to avoid mutating the original
        let mut buf = slice.to_vec();
        simd_json::from_slice(&mut buf).map_err(Into::into)
    }
    #[cfg(not(feature = "simd-json"))]
    {
        serde_json::from_slice(slice).map_err(Into::into)
    }
}

/// Deserialize a value from an owned JSON byte buffer, parsing in place.
///
/// This is the zero-copy counterpart to [`from_slice`]. When built with the
/// `simd-json` feature, `simd-json` parses directly inside the caller-owned
/// `buf` (mutating it in place) instead of first cloning the input with
/// `slice.to_vec()`. This avoids a full copy of the request body on the
/// deserialization hot path.
///
/// Prefer this over [`from_slice`] whenever you already own the byte buffer
/// (e.g. a request body you can move by value).
///
/// # Example
///
/// ```rust,ignore
/// // `body` is an owned `Vec<u8>` (e.g. request.body)
/// let data: User = json::from_owned(body)?;
/// ```
#[inline]
pub fn from_owned<T: DeserializeOwned>(buf: Vec<u8>) -> Result<T> {
    #[cfg(feature = "simd-json")]
    {
        // Parse in place: no copy, buffer is consumed and mutated by simd-json.
        let mut buf = buf;
        simd_json::from_slice(&mut buf).map_err(Into::into)
    }
    #[cfg(not(feature = "simd-json"))]
    {
        serde_json::from_slice(&buf).map_err(Into::into)
    }
}

/// Deserialize a value from a mutable JSON byte slice (zero-copy with simd-json).
///
/// This is the most efficient deserialization method when using `simd-json`,
/// as it can parse in-place without additional allocations.
///
/// # Warning
///
/// The input buffer will be modified during parsing when using `simd-json`.
///
/// # Example
///
/// ```rust,ignore
/// let mut buffer = request.body.clone();
/// let data: User = json::from_slice_mut(&mut buffer)?;
/// ```
#[inline]
pub fn from_slice_mut<T: DeserializeOwned>(slice: &mut [u8]) -> Result<T> {
    #[cfg(feature = "simd-json")]
    {
        simd_json::from_slice(slice).map_err(Into::into)
    }
    #[cfg(not(feature = "simd-json"))]
    {
        serde_json::from_slice(slice).map_err(Into::into)
    }
}

/// Deserialize a value from a JSON string.
///
/// # Example
///
/// ```rust,ignore
/// let data: User = json::from_str(r#"{"name":"John","age":30}"#)?;
/// ```
#[inline]
pub fn from_str<T: DeserializeOwned>(s: &str) -> Result<T> {
    #[cfg(feature = "simd-json")]
    {
        // simd-json requires mutable buffer
        let mut buf = s.as_bytes().to_vec();
        simd_json::from_slice(&mut buf).map_err(Into::into)
    }
    #[cfg(not(feature = "simd-json"))]
    {
        serde_json::from_str(s).map_err(Into::into)
    }
}

/// Deserialize a value from a reader.
///
/// # Example
///
/// ```rust,ignore
/// let data: User = json::from_reader(file)?;
/// ```
#[inline]
pub fn from_reader<R: std::io::Read, T: DeserializeOwned>(reader: R) -> Result<T> {
    // simd-json doesn't have a direct from_reader, so use serde_json for both
    serde_json::from_reader(reader).map_err(Into::into)
}

// ============================================================================
// Value Type (for dynamic JSON)
// ============================================================================

/// A JSON value type.
///
/// This is re-exported from the underlying JSON library.
#[cfg(feature = "simd-json")]
pub use simd_json::OwnedValue as Value;

#[cfg(not(feature = "simd-json"))]
pub use serde_json::Value;

/// Create a JSON value from a serializable type.
#[inline]
pub fn to_value<T: Serialize>(value: &T) -> Result<serde_json::Value> {
    serde_json::to_value(value).map_err(Into::into)
}

/// Convert a JSON value to a specific type.
#[inline]
pub fn from_value<T: DeserializeOwned>(value: serde_json::Value) -> Result<T> {
    serde_json::from_value(value).map_err(Into::into)
}

// ============================================================================
// Utility Functions
// ============================================================================

/// Check if the current build has SIMD JSON support enabled.
///
/// # Example
///
/// ```rust,ignore
/// if json::is_simd_enabled() {
///     println!("Using SIMD-accelerated JSON!");
/// }
/// ```
#[inline]
pub const fn is_simd_enabled() -> bool {
    cfg!(feature = "simd-json")
}

/// Get the name of the JSON library being used.
#[inline]
pub const fn library_name() -> &'static str {
    if cfg!(feature = "simd-json") {
        "simd-json"
    } else {
        "serde_json"
    }
}

// ============================================================================
// Macros for JSON creation
// ============================================================================

/// Create a JSON value using serde_json's json! macro.
///
/// This always uses serde_json for consistency, as simd-json's
/// JSON creation macros have different behavior.
///
/// # Example
///
/// ```rust,ignore
/// let value = json!({
///     "name": "John",
///     "age": 30
/// });
/// ```
#[macro_export]
macro_rules! json {
    ($($json:tt)+) => {
        serde_json::json!($($json)+)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TestUser {
        name: String,
        age: u32,
        email: String,
    }

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let user = TestUser {
            name: "John Doe".to_string(),
            age: 30,
            email: "john@example.com".to_string(),
        };

        // Serialize
        let bytes = to_vec(&user).unwrap();

        // Deserialize
        let parsed: TestUser = from_slice(&bytes).unwrap();

        assert_eq!(user, parsed);
    }

    #[test]
    fn test_to_string() {
        let user = TestUser {
            name: "John".to_string(),
            age: 25,
            email: "john@test.com".to_string(),
        };

        let json_str = to_string(&user).unwrap();
        assert!(json_str.contains("John"));
        assert!(json_str.contains("25"));
    }

    #[test]
    fn test_to_string_pretty() {
        let user = TestUser {
            name: "John".to_string(),
            age: 25,
            email: "john@test.com".to_string(),
        };

        let json_str = to_string_pretty(&user).unwrap();
        assert!(json_str.contains('\n')); // Pretty print has newlines
    }

    #[test]
    fn test_from_str() {
        let json_str = r#"{"name":"Alice","age":28,"email":"alice@example.com"}"#;
        let user: TestUser = from_str(json_str).unwrap();
        assert_eq!(user.name, "Alice");
        assert_eq!(user.age, 28);
    }

    #[test]
    fn test_from_owned() {
        let bytes = br#"{"name":"Dana","age":22,"email":"dana@test.com"}"#.to_vec();
        let user: TestUser = from_owned(bytes).unwrap();
        assert_eq!(user.name, "Dana");
        assert_eq!(user.age, 22);
        assert_eq!(user.email, "dana@test.com");
    }

    #[test]
    fn test_from_owned_roundtrip_with_to_vec() {
        let user = TestUser {
            name: "Eve".to_string(),
            age: 33,
            email: "eve@example.com".to_string(),
        };
        let bytes = to_vec(&user).unwrap();
        let parsed: TestUser = from_owned(bytes).unwrap();
        assert_eq!(user, parsed);
    }

    #[test]
    fn test_from_owned_error() {
        let bad = b"{ not json }".to_vec();
        let result: Result<TestUser> = from_owned(bad);
        assert!(result.is_err());
    }

    #[test]
    fn test_from_slice_mut() {
        let mut bytes = br#"{"name":"Bob","age":35,"email":"bob@test.com"}"#.to_vec();
        let user: TestUser = from_slice_mut(&mut bytes).unwrap();
        assert_eq!(user.name, "Bob");
        assert_eq!(user.age, 35);
    }

    #[test]
    fn test_to_vec_with_capacity() {
        let user = TestUser {
            name: "Charlie".to_string(),
            age: 40,
            email: "charlie@example.com".to_string(),
        };

        let bytes = to_vec_with_capacity(&user, 256).unwrap();
        let parsed: TestUser = from_slice(&bytes).unwrap();
        assert_eq!(user, parsed);
    }

    #[test]
    fn test_library_info() {
        let name = library_name();
        let simd = is_simd_enabled();

        if simd {
            assert_eq!(name, "simd-json");
        } else {
            assert_eq!(name, "serde_json");
        }
    }

    #[test]
    fn test_json_macro() {
        let value = serde_json::json!({
            "key": "value",
            "number": 42
        });

        assert_eq!(value["key"], "value");
        assert_eq!(value["number"], 42);
    }

    #[test]
    fn test_error_handling() {
        let bad_json = b"{ invalid json }";
        let result: Result<TestUser> = from_slice(bad_json);
        assert!(result.is_err());
    }
}
