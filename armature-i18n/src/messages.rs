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
///
/// Each variant carries the message data directly (not a file path) and can be
/// turned into a [`MessageBundle`] via [`MessageBundle::from_source`], or loaded
/// into a collection with [`Messages::add_source`] / [`I18n::add_source`].
#[derive(Debug, Clone)]
pub enum TranslationSource {
    /// JSON document content (see [`MessageBundle::from_json`]).
    Json(String),
    /// Fluent (`.ftl`) document content. A pragmatic `key = value` subset of
    /// the Fluent syntax is supported (comments and blank lines are skipped).
    Fluent(String),
    /// In-memory messages, keyed by message ID.
    Memory(HashMap<String, String>),
}

/// A bundle of messages for a single locale.
#[derive(Debug, Clone, Default)]
pub struct MessageBundle {
    /// Messages keyed by message ID
    messages: HashMap<String, String>,
    /// Plural messages: message ID -> category -> message.
    ///
    /// Nested rather than keyed by `(String, PluralCategory)` so a lookup can
    /// borrow the key instead of allocating a fresh tuple on every probe —
    /// `get_plural` runs on the request path and probes twice (the requested
    /// category, then `Other`).
    plurals: HashMap<String, HashMap<PluralCategory, String>>,
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
                                bundle
                                    .plurals
                                    .entry(key.clone())
                                    .or_default()
                                    .insert(category, s);
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

    /// Load from Fluent (`.ftl`) content.
    ///
    /// Parses the pragmatic `key = value` subset of Fluent: comment lines
    /// (`#`) and blank lines are ignored, and each `id = message` line becomes
    /// a simple message. Plural selectors and terms are not expanded here.
    pub fn from_fluent(source: &str) -> Result<Self> {
        let mut bundle = Self::new();
        for line in source.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                bundle
                    .messages
                    .insert(key.trim().to_string(), value.trim().to_string());
            }
        }
        Ok(bundle)
    }

    /// Build a bundle from a [`TranslationSource`].
    pub fn from_source(source: &TranslationSource) -> Result<Self> {
        match source {
            TranslationSource::Json(content) => Self::from_json(content),
            TranslationSource::Fluent(content) => Self::from_fluent(content),
            TranslationSource::Memory(map) => {
                let mut bundle = Self::new();
                for (key, value) in map {
                    bundle.messages.insert(key.clone(), value.clone());
                }
                Ok(bundle)
            }
        }
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
        self.plurals
            .entry(key.into())
            .or_default()
            .insert(category, message.into());
    }

    /// Get a message.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.messages.get(key).map(|s| s.as_str())
    }

    /// Get a plural form.
    pub fn get_plural(&self, key: &str, category: PluralCategory) -> Option<&str> {
        let forms = self.plurals.get(key)?;
        forms
            .get(&category)
            .or_else(|| forms.get(&PluralCategory::Other))
            .map(|s| s.as_str())
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

    /// Add a bundle for a locale from a [`TranslationSource`].
    pub fn add_source(&mut self, locale: &Locale, source: &TranslationSource) -> Result<()> {
        let bundle = MessageBundle::from_source(source)?;
        self.add_bundle(locale, bundle);
        Ok(())
    }

    /// Get the most specific bundle for a locale.
    ///
    /// Prefer [`Messages::lookup`] / [`Messages::lookup_plural`] for message
    /// resolution: this returns a single bundle, so a caller that reads a key
    /// straight off it will miss keys that only exist in the language-only
    /// bundle.
    pub fn get_bundle(&self, locale: &Locale) -> Option<&MessageBundle> {
        self.bundle_chain(locale).into_iter().flatten().next()
    }

    /// The bundles to consult for `locale`, most specific first: the exact
    /// tag, then the language-only tag when the locale carries a region.
    ///
    /// Returned as a fixed-size array rather than a `Vec` to keep the request
    /// path free of heap allocation.
    fn bundle_chain(&self, locale: &Locale) -> [Option<&MessageBundle>; 2] {
        let exact = self.bundles.get(locale.tag().as_str());

        // Only worth a second probe when stripping the region actually yields
        // a different tag.
        let lang_only = if locale.region.is_some() {
            self.bundles.get(Self::language_only_tag(locale).as_str())
        } else {
            None
        };

        [exact, lang_only]
    }

    /// Build the region-stripped tag for `locale`.
    ///
    /// Equivalent to `locale.language_only().tag()` but without cloning the
    /// whole `Locale` first.
    fn language_only_tag(locale: &Locale) -> String {
        let mut tag = locale.language.clone();
        if let Some(ref script) = locale.script {
            tag.push('-');
            tag.push_str(script);
        }
        tag
    }

    /// Look up a single message, walking the locale's bundle chain per KEY.
    ///
    /// Walking per key (rather than picking one bundle and reading it) is what
    /// makes a partially translated regional bundle work: a key present in
    /// `fr` but absent from `fr-CA` still resolves for an `fr-CA` request.
    pub fn lookup(&self, key: &str, locale: &Locale) -> Option<&str> {
        self.bundle_chain(locale)
            .into_iter()
            .flatten()
            .find_map(|bundle| bundle.get(key))
    }

    /// Look up a plural form, walking the locale's bundle chain per key.
    pub fn lookup_plural(
        &self,
        key: &str,
        locale: &Locale,
        category: PluralCategory,
    ) -> Option<&str> {
        self.bundle_chain(locale)
            .into_iter()
            .flatten()
            .find_map(|bundle| bundle.get_plural(key, category))
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

    /// Add a bundle for a locale from a [`TranslationSource`]
    /// (JSON, Fluent, or in-memory).
    pub fn add_source(&self, locale: &Locale, source: &TranslationSource) -> Result<()> {
        self.messages.write().add_source(locale, source)
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

        // Each step resolves the key across the whole locale chain, so a
        // regional bundle that is missing the key still falls through to its
        // language-only bundle instead of terminating the search.
        if let Some(msg) = messages.lookup(key, locale) {
            return msg.to_string();
        }

        if let Some(ref fallback) = self.fallback_locale
            && let Some(msg) = messages.lookup(key, fallback)
        {
            return msg.to_string();
        }

        if let Some(msg) = messages.lookup(key, &self.default_locale) {
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
            .lookup_plural(key, locale, category)
            .or_else(|| {
                self.fallback_locale
                    .as_ref()
                    .and_then(|fb| messages.lookup_plural(key, fb, category))
            })
            .or_else(|| messages.lookup_plural(key, &self.default_locale, category))
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{}[{}]", key, category));

        // Replace {n} placeholder
        msg.replace("{n}", &count.to_string())
    }

    /// Check if a message exists.
    ///
    /// Walks exactly the same chain as [`I18n::t`], so `has` never reports
    /// false for a key that `t` would resolve.
    pub fn has(&self, key: &str, locale: &Locale) -> bool {
        let messages = self.messages.read();

        messages.lookup(key, locale).is_some()
            || self
                .fallback_locale
                .as_ref()
                .is_some_and(|fb| messages.lookup(key, fb).is_some())
            || messages.lookup(key, &self.default_locale).is_some()
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
    fn test_translation_source_variants_are_consumable() {
        // Regression: the Fluent and Memory variants of TranslationSource were
        // never constructed or consumed anywhere. They must now build bundles.
        let json = TranslationSource::Json(r#"{"hello":"Hello!"}"#.to_string());
        let bundle = MessageBundle::from_source(&json).unwrap();
        assert_eq!(bundle.get("hello"), Some("Hello!"));

        let ftl = TranslationSource::Fluent("# comment\nhello = Bonjour!\n".to_string());
        let bundle = MessageBundle::from_source(&ftl).unwrap();
        assert_eq!(bundle.get("hello"), Some("Bonjour!"));

        let mut map = HashMap::new();
        map.insert("hello".to_string(), "Hallo!".to_string());
        let mem = TranslationSource::Memory(map);

        let i18n = I18n::new();
        i18n.add_source(&Locale::de(), &mem).unwrap();
        assert_eq!(i18n.t("hello", &Locale::de()), "Hallo!");
    }

    #[test]
    fn test_regional_bundle_falls_back_per_key() {
        // Regression: the fallback chain used to be applied per BUNDLE, so a
        // key missing from `fr-CA` was never looked up in `fr` once the
        // `fr-CA` bundle existed -- it jumped straight to the English default.
        let i18n = create_test_i18n();

        let mut fr_ca = MessageBundle::new();
        fr_ca.add("hello", "Allo!");
        let fr_ca_locale = Locale::new("fr", Some("CA"));
        i18n.add_bundle(&fr_ca_locale, fr_ca);

        assert_eq!(i18n.t("hello", &fr_ca_locale), "Allo!");
        assert_eq!(i18n.t("greeting", &fr_ca_locale), "Bonjour, {name}!");
        assert_eq!(i18n.t_plural("items", 2, &fr_ca_locale), "2 articles");
    }

    #[test]
    fn test_has_follows_the_same_chain_as_t() {
        // Regression: `has` only consulted the requested locale's bundle, so
        // it reported false for keys `t` resolves through the fallback chain.
        let i18n = create_test_i18n();

        let fr_ca = Locale::new("fr", Some("CA"));
        assert!(i18n.has("greeting", &fr_ca));

        // `de` has no bundle at all; `t` still resolves via the fallback.
        let de = Locale::de();
        assert_eq!(i18n.t("hello", &de), "Hello!");
        assert!(i18n.has("hello", &de));

        assert!(!i18n.has("unknown.key", &Locale::en()));
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
