// Configuration file loaders

use crate::{ConfigError, Result};
use serde_json::Value;
use std::fs;
use std::path::Path;

/// Supported configuration file formats
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileFormat {
    Json,
    Toml,
    Env,
}

impl FileFormat {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "json" => Some(FileFormat::Json),
            "toml" => Some(FileFormat::Toml),
            "env" => Some(FileFormat::Env),
            _ => None,
        }
    }
}

/// Name the shape of a JSON value, for an error message that says what was
/// found rather than only what was wanted.
fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Configuration file loader
pub struct ConfigLoader {
    format: FileFormat,
}

impl ConfigLoader {
    pub fn new(format: FileFormat) -> Self {
        Self { format }
    }

    /// Auto-detect format from file extension
    pub fn auto(path: &str) -> Result<Self> {
        let path_obj = Path::new(path);
        let ext = path_obj
            .extension()
            .and_then(|s| s.to_str())
            .ok_or_else(|| ConfigError::LoadError("No file extension found".to_string()))?;

        let format = FileFormat::from_extension(ext)
            .ok_or_else(|| ConfigError::LoadError(format!("Unsupported format: {}", ext)))?;

        Ok(Self::new(format))
    }

    /// Load configuration from file
    pub fn load_file(&self, path: &str) -> Result<Value> {
        let content = fs::read_to_string(path)
            .map_err(|e| ConfigError::LoadError(format!("Failed to read file: {}", e)))?;

        self.parse(&content)
    }

    /// Parse configuration from string
    pub fn parse(&self, content: &str) -> Result<Value> {
        match self.format {
            FileFormat::Json => self.parse_json(content),
            FileFormat::Toml => self.parse_toml(content),
            FileFormat::Env => self.parse_env(content),
        }
    }

    fn parse_json(&self, content: &str) -> Result<Value> {
        let value: Value = serde_json::from_str(content)
            .map_err(|e| ConfigError::ParseError(format!("JSON parse error: {}", e)))?;

        // A configuration document is a mapping of keys to values. JSON is the
        // only format here that can parse to something else — a TOML document
        // is a table and a `.env` file is a set of assignments, both objects by
        // construction — so `99`, `[1, 2]` or `"text"` at the top level parses
        // happily and then carries no keys.
        //
        // Rejecting it here rather than downstream matters because
        // `ConfigManager::load_file` pattern-matches the object case and
        // otherwise falls through to `Ok(())`. A file that renders to a bare
        // scalar — a templating mistake is the usual way — would load
        // "successfully" while applying nothing, leaving the service on
        // defaults with no error to explain why.
        if !value.is_object() {
            return Err(ConfigError::ParseError(format!(
                "JSON parse error: expected a top-level object, found {}",
                json_kind(&value)
            )));
        }

        Ok(value)
    }

    fn parse_toml(&self, content: &str) -> Result<Value> {
        let toml_value: toml::Value = toml::from_str(content)
            .map_err(|e| ConfigError::ParseError(format!("TOML parse error: {}", e)))?;

        // Convert TOML value to JSON value
        let json_str = serde_json::to_string(&toml_value)
            .map_err(|e| ConfigError::SerializationError(e.to_string()))?;

        serde_json::from_str(&json_str)
            .map_err(|e| ConfigError::ParseError(format!("TOML to JSON conversion error: {}", e)))
    }

    /// Parse `.env` content with `dotenvy`, the same parser
    /// `ConfigManager::load_dotenv` uses.
    ///
    /// The hand-rolled parser this replaces disagreed with `dotenvy` on the
    /// same file: it did not understand `export KEY=value`, escape sequences,
    /// inline `#` comments, or multi-line values, and its
    /// `trim_matches('"')` stripped *every* leading and trailing quote rather
    /// than one matched pair (so `""quoted""` and `"a"b"` came out wrong).
    /// Two parsers for one file format meant `FileFormat::Env` and
    /// `load_dotenv` could produce different configuration from identical
    /// bytes.
    fn parse_env(&self, content: &str) -> Result<Value> {
        let mut map = serde_json::Map::new();

        for item in dotenvy::from_read_iter(content.as_bytes()) {
            let (key, value) =
                item.map_err(|e| ConfigError::ParseError(format!(".env parse error: {}", e)))?;
            map.insert(key, Value::String(value));
        }

        Ok(Value::Object(map))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_json() {
        let loader = ConfigLoader::new(FileFormat::Json);
        let json = r#"{"key": "value", "number": 42}"#;

        let result = loader.parse(json).unwrap();
        assert!(result.is_object());
    }

    #[test]
    fn test_parse_toml() {
        let loader = ConfigLoader::new(FileFormat::Toml);
        let toml = r#"
            key = "value"
            number = 42
        "#;

        let result = loader.parse(toml).unwrap();
        assert!(result.is_object());
    }

    #[test]
    fn test_parse_env() {
        let loader = ConfigLoader::new(FileFormat::Env);
        let env = concat!(
            "KEY=value\n",
            "NUMBER=42\n",
            "# Comment\n",
            "QUOTED=\"quoted value\"\n",
            // `export` prefixes, inline comments and escapes: all understood by
            // `dotenvy`, none understood by the hand-rolled parser this
            // replaced. The value is quoted because a bare value may not
            // contain spaces — the old parser accepted it, `dotenvy` rejects
            // the whole file for it, and real `.env` consumers agree with
            // `dotenvy`.
            "export EXPORTED=\"exported value\"\n",
            "INLINE=kept # trailing comment\n",
        );

        let result = loader.parse(env).unwrap();
        let obj = result.as_object().expect("env parses to an object");
        assert_eq!(obj.get("KEY").and_then(Value::as_str), Some("value"));
        assert_eq!(obj.get("NUMBER").and_then(Value::as_str), Some("42"));
        assert!(!obj.contains_key("# Comment"));
        assert_eq!(
            obj.get("QUOTED").and_then(Value::as_str),
            Some("quoted value"),
            "one matched pair of quotes is stripped, not every quote"
        );
        assert_eq!(
            obj.get("EXPORTED").and_then(Value::as_str),
            Some("exported value")
        );
        assert_eq!(obj.get("INLINE").and_then(Value::as_str), Some("kept"));
    }

    /// JSON is the only supported format whose document can parse to something
    /// that is not a mapping. Accepting one produces a config carrying no keys,
    /// which downstream cannot distinguish from a file that legitimately set
    /// nothing — so a rendering mistake would leave the service on defaults
    /// and report success.
    #[test]
    fn top_level_json_that_is_not_an_object_is_rejected() {
        let loader = ConfigLoader::new(FileFormat::Json);

        for content in ["99", "\"text\"", "[1, 2]", "true", "null"] {
            let err = loader
                .parse(content)
                .expect_err("a non-object JSON document must not parse as configuration");
            assert!(
                err.to_string().contains("expected a top-level object"),
                "{content:?} was rejected, but not for the reason we mean: {err}"
            );
        }

        // The empty object carries no keys either, but it says so deliberately
        // and is a valid config document.
        assert!(loader.parse("{}").is_ok());
    }

    #[test]
    fn test_format_detection() {
        assert_eq!(FileFormat::from_extension("json"), Some(FileFormat::Json));
        assert_eq!(FileFormat::from_extension("toml"), Some(FileFormat::Toml));
        assert_eq!(FileFormat::from_extension("env"), Some(FileFormat::Env));
        assert_eq!(FileFormat::from_extension("unknown"), None);
    }
}
