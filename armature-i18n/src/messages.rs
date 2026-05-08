//! Message Translation System
//!
//! Provides loading and formatting of localized messages.

use crate::{I18nError, Locale, PluralCategory, Result, plural_category};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

/// Source for translation messages.
#[derive(Debug, Clone)]
pub enum TranslationSource {
    /// JSON file format
    Json(String),
    /// Fluent file format (.ftl)
    Fluent(String),
    /// In-memory messages
    Memory(HashMap<String, String>),
}

/// A bundle of messages for a single locale.
#[derive(Debug, Clone, Default)]
pub struct MessageBundle {
    /// Messages keyed by message ID
    messages: HashMap<String, String>,
    /// Plural messages keyed by (message_id, category)
    plurals: HashMap<(String, PluralCategory), String>,
}

impl MessageBundle {
    /// Create a new empty bundle.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load from JSON.
    pub fn from_json(json: &str) -> Result<Self> {
        let data: HashMap<String, serde_json::Value> = serde_json::from_str(json)?;
        let mut bundle = Self::new();

        for (key, value) in data {
            match value {
                serde_json::Value::String(s) => {
                    bundle.messages.insert(key, s);
                }
                serde_json::Value::Object(obj) => {
                    // Plural forms
                    for (form, msg) in obj {
                        if let serde_json::Value::String(s) = msg {
                            if let Ok(category) = PluralCategory::parse(&form) {
                                bundle.plurals.insert((key.clone(), category), s);
                            } else {
                                // Nested key
                                bundle.messages.insert(format!("{}.{}", key, form), s);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(bundle)
    }

    /// Add a message.
    pub fn add(&mut self, key: impl Into<String>, message: impl Into<String>) {
        self.messages.insert(key.into(), message.into());
    }

    /// Add a plural form.
    pub fn add_plural(
        &mut self,
        key: impl Into<String>,
        category: PluralCategory,
        message: impl Into<String>,
    ) {
        self.plurals.insert((key.into(), category), message.into());
    }

    /// Get a message.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.messages.get(key).map(|s| s.as_str())
    }

    /// Get a plural form.
    pub fn get_plural(&self, key: &str, category: PluralCategory) -> Option<&str> {
        self.plurals
            .get(&(key.to_string(), category))
            .map(|s| s.as_str())
            .or_else(|| {
                // Fallback to "other" category
                self.plurals
                    .get(&(key.to_string(), PluralCategory::Other))
                    .map(|s| s.as_str())
            })
    }

    /// Check if bundle has a message.
    pub fn has(&self, key: &str) -> bool {
        self.messages.contains_key(key)
    }

    /// Get all message keys.
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.messages.keys()
    }
}

/// Collection of message bundles for multiple locales.
#[derive(Debug, Default)]
pub struct Messages {
    /// Bundles keyed by locale tag
    bundles: HashMap<String, MessageBundle>,
}

impl Messages {
    /// Create a new messages collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a bundle for a locale.
    pub fn add_bundle(&mut self, locale: &Locale, bundle: MessageBundle) {
        self.bundles.insert(locale.tag(), bundle);
    }

    /// Get a bundle for a locale.
    pub fn get_bundle(&self, locale: &Locale) -> Option<&MessageBundle> {
        // Try exact match first
        if let Some(bundle) = self.bundles.get(&locale.tag()) {
            return Some(bundle);
        }

        // Try language-only fallback
        if locale.region.is_some() {
            let lang_only = locale.language_only();
            if let Some(bundle) = self.bundles.get(&lang_only.tag()) {
                return Some(bundle);
            }
        }

        None
    }

    /// Load from a directory.
    ///
    /// Expected structure:
    /// - `locales/en.json`
    /// - `locales/en-US.json`
    /// - `locales/fr.json`
    pub fn load_from_dir(&mut self, dir: impl AsRef<Path>) -> Result<()> {
        let dir = dir.as_ref();

        if !dir.exists() {
            return Err(I18nError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Directory not found: {:?}", dir),
            )));
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().is_some_and(|ext| ext == "json") {
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| I18nError::ParseError("Invalid filename".to_string()))?;

                let locale = Locale::parse(stem)?;
                let content = fs::read_to_string(&path)?;
                let bundle = MessageBundle::from_json(&content)?;

                self.add_bundle(&locale, bundle);
            }
        }

        Ok(())
    }
}

/// Main i18n interface.
///
/// Thread-safe translation system with locale fallback.
pub struct I18n {
    messages: Arc<RwLock<Messages>>,
    default_locale: Locale,
    fallback_locale: Option<Locale>,
}

impl I18n {
    /// Create a new i18n instance.
    pub fn new() -> Self {
        Self {
            messages: Arc::new(RwLock::new(Messages::new())),
            default_locale: Locale::en_us(),
            fallback_locale: Some(Locale::en()),
        }
    }

    /// Set the default locale.
    pub fn with_default_locale(mut self, locale: Locale) -> Self {
        self.default_locale = locale;
        self
    }

    /// Set the fallback locale.
    pub fn with_fallback(mut self, locale: Locale) -> Self {
        self.fallback_locale = Some(locale);
        self
    }

    /// Load messages from a directory.
    pub fn load_from_dir(self, dir: impl AsRef<Path>) -> Result<Self> {
        self.messages.write().load_from_dir(dir)?;
        Ok(self)
    }

    /// Add a message bundle.
    pub fn add_bundle(&self, locale: &Locale, bundle: MessageBundle) {
        self.messages.write().add_bundle(locale, bundle);
    }

    /// Get the default locale.
    pub fn default_locale(&self) -> &Locale {
        &self.default_locale
    }

    /// Translate a message key.
    ///
    /// Looks up the message in the given locale, falling back to
    /// language-only, then fallback locale, then default locale.
    pub fn t(&self, key: &str, locale: &Locale) -> String {
        let messages = self.messages.read();

        // Try exact locale
        if let Some(bundle) = messages.get_bundle(locale)
            && let Some(msg) = bundle.get(key)
        {
            return msg.to_string();
        }

        // Try fallback locale
        if let Some(ref fallback) = self.fallback_locale
            && let Some(bundle) = messages.get_bundle(fallback)
            && let Some(msg) = bundle.get(key)
        {
            return msg.to_string();
        }

        // Try default locale
        if let Some(bundle) = messages.get_bundle(&self.default_locale)
            && let Some(msg) = bundle.get(key)
        {
            return msg.to_string();
        }

        // Return key as fallback
        key.to_string()
    }

    /// Translate with arguments.
    ///
    /// Replaces `{name}` placeholders with provided values.
    pub fn t_args(&self, key: &str, locale: &Locale, args: &[(&str, &str)]) -> String {
        let mut result = self.t(key, locale);

        for (name, value) in args {
            let placeholder = format!("{{{}}}", name);
            result = result.replace(&placeholder, value);
        }

        result
    }

    /// Translate with number argument.
    ///
    /// Useful for simple number interpolation.
    pub fn t_num(&self, key: &str, locale: &Locale, n: impl std::fmt::Display) -> String {
        self.t_args(key, locale, &[("n", &n.to_string())])
    }

    /// Translate with pluralization.
    ///
    /// Selects the appropriate plural form based on the count.
    pub fn t_plural(
        &self,
        key: &str,
        count: impl Into<f64> + Copy + std::fmt::Display,
        locale: &Locale,
    ) -> String {
        let n = count.into();
        let category = plural_category(n, locale);
        let messages = self.messages.read();

        // Try to get plural form
        let msg = messages
            .get_bundle(locale)
            .and_then(|b| b.get_plural(key, category))
            .or_else(|| {
                self.fallback_locale
                    .as_ref()
                    .and_then(|fb| messages.get_bundle(fb))
                    .and_then(|b| b.get_plural(key, category))
            })
            .or_else(|| {
                messages
                    .get_bundle(&self.default_locale)
                    .and_then(|b| b.get_plural(key, category))
            })
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{}[{}]", key, category));

        // Replace {n} placeholder
        msg.replace("{n}", &count.to_string())
    }

    /// Check if a message exists.
    pub fn has(&self, key: &str, locale: &Locale) -> bool {
        let messages = self.messages.read();

        messages
            .get_bundle(locale)
            .map(|b| b.has(key))
            .unwrap_or(false)
    }
}

impl Default for I18n {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for I18n {
    fn clone(&self) -> Self {
        Self {
            messages: Arc::clone(&self.messages),
            default_locale: self.default_locale.clone(),
            fallback_locale: self.fallback_locale.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_i18n() -> I18n {
        let i18n = I18n::new()
            .with_default_locale(Locale::en_us())
            .with_fallback(Locale::en());

        // English bundle
        let mut en = MessageBundle::new();
        en.add("hello", "Hello!");
        en.add("greeting", "Hello, {name}!");
        en.add_plural("items", PluralCategory::One, "{n} item");
        en.add_plural("items", PluralCategory::Other, "{n} items");
        i18n.add_bundle(&Locale::en(), en);

        // French bundle
        let mut fr = MessageBundle::new();
        fr.add("hello", "Bonjour!");
        fr.add("greeting", "Bonjour, {name}!");
        fr.add_plural("items", PluralCategory::One, "{n} article");
        fr.add_plural("items", PluralCategory::Other, "{n} articles");
        i18n.add_bundle(&Locale::fr(), fr);

        i18n
    }

    #[test]
    fn test_simple_translation() {
        let i18n = create_test_i18n();

        assert_eq!(i18n.t("hello", &Locale::en()), "Hello!");
        assert_eq!(i18n.t("hello", &Locale::fr()), "Bonjour!");
    }

    #[test]
    fn test_translation_with_args() {
        let i18n = create_test_i18n();

        let msg = i18n.t_args("greeting", &Locale::en(), &[("name", "Alice")]);
        assert_eq!(msg, "Hello, Alice!");

        let msg = i18n.t_args("greeting", &Locale::fr(), &[("name", "Alice")]);
        assert_eq!(msg, "Bonjour, Alice!");
    }

    #[test]
    fn test_plural_translation() {
        let i18n = create_test_i18n();

        assert_eq!(i18n.t_plural("items", 1, &Locale::en()), "1 item");
        assert_eq!(i18n.t_plural("items", 5, &Locale::en()), "5 items");
        assert_eq!(i18n.t_plural("items", 0, &Locale::en()), "0 items");
    }

    #[test]
    fn test_locale_fallback() {
        let i18n = create_test_i18n();

        // en-US should fall back to en
        assert_eq!(i18n.t("hello", &Locale::en_us()), "Hello!");

        // Unknown locale should fall back to default
        let de = Locale::de();
        assert_eq!(i18n.t("hello", &de), "Hello!");
    }

    #[test]
    fn test_missing_key() {
        let i18n = create_test_i18n();

        // Missing key returns the key itself
        assert_eq!(i18n.t("unknown.key", &Locale::en()), "unknown.key");
    }

    #[test]
    fn test_message_bundle_from_json() {
        let json = r#"{
            "hello": "Hello!",
            "greeting": "Hello, {name}!",
            "items": {
                "one": "{n} item",
                "other": "{n} items"
            }
        }"#;

        let bundle = MessageBundle::from_json(json).unwrap();

        assert_eq!(bundle.get("hello"), Some("Hello!"));
        assert_eq!(
            bundle.get_plural("items", PluralCategory::One),
            Some("{n} item")
        );
        assert_eq!(
            bundle.get_plural("items", PluralCategory::Other),
            Some("{n} items")
        );
    }
}
