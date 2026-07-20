//! Tera template engine integration for email templates.
//!
//! This module provides Tera-based email templating capabilities, mirroring the
//! Handlebars engine: templates live in a per-template directory containing
//! `html.tera`, and optionally `text.tera` and `subject.tera`.
//!
//! # Features
//!
//! This module requires the `tera` feature to be enabled:
//!
//! ```toml
//! [dependencies]
//! armature-mail = { version = "0.1", features = ["tera"] }
//! ```
//!
//! # Example
//!
//! ```rust
//! use armature_mail::{TeraEngine, TemplateEngine};
//! use serde_json::json;
//!
//! let mut engine = TeraEngine::new();
//! engine
//!     .register_template("welcome", "<h1>Hello, {{ name }}!</h1>")
//!     .unwrap();
//!
//! let rendered = engine.render("welcome", &json!({ "name": "World" })).unwrap();
//! assert_eq!(rendered.html.as_deref(), Some("<h1>Hello, World!</h1>"));
//! ```

use std::path::Path;
use tera::{Context, Tera};
use tracing::debug;

use crate::{MailError, RenderedTemplate, Result, TemplateEngine};

/// File extension used for Tera email templates.
const EXT: &str = "tera";

/// Tera template engine for email rendering.
pub struct TeraEngine {
    tera: Tera,
}

impl TeraEngine {
    /// Create a new, empty Tera engine.
    pub fn new() -> Self {
        Self { tera: Tera::new() }
    }

    /// Load templates from a directory.
    ///
    /// Expected structure:
    /// ```text
    /// templates/
    ///   welcome/
    ///     subject.tera     (optional)
    ///     html.tera
    ///     text.tera        (optional)
    ///   password_reset/
    ///     subject.tera
    ///     html.tera
    ///     text.tera
    /// ```
    pub fn from_directory(path: impl AsRef<Path>) -> Result<Self> {
        let mut engine = Self::new();
        let path = path.as_ref();

        if !path.exists() {
            return Err(MailError::Config(format!(
                "Template directory not found: {}",
                path.display()
            )));
        }

        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let entry_path = entry.path();

            if !entry_path.is_dir() {
                continue;
            }

            let template_name = entry_path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| MailError::Config("Invalid template directory name".to_string()))?
                .to_string();

            for part in ["html", "text", "subject"] {
                let part_path = entry_path.join(format!("{}.{}", part, EXT));
                if part_path.exists() {
                    let content = std::fs::read_to_string(&part_path)?;
                    engine
                        .tera
                        .add_raw_template(&format!("{}/{}", template_name, part), &content)?;
                }
            }

            debug!(template = %template_name, "Loaded email template (Tera)");
        }

        Ok(engine)
    }

    /// Register a raw template under an explicit key (e.g. `"welcome/text"`).
    ///
    /// [`TemplateEngine::register_template`] registers the HTML part; this lets
    /// you register the text and subject parts too.
    pub fn register_raw(&mut self, key: &str, content: &str) -> Result<()> {
        self.tera.add_raw_template(key, content)?;
        Ok(())
    }

    /// Access the underlying Tera instance (to register filters, functions, …).
    pub fn tera_mut(&mut self) -> &mut Tera {
        &mut self.tera
    }

    fn render_part(&self, key: &str, context: &Context) -> Result<Option<String>> {
        if self.tera.contains_template(key) {
            Ok(Some(self.tera.render(key, context)?))
        } else {
            Ok(None)
        }
    }
}

impl Default for TeraEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TemplateEngine for TeraEngine {
    fn render(&self, name: &str, context: &serde_json::Value) -> Result<RenderedTemplate> {
        let context = Context::from_serialize(context)?;

        let html = self.render_part(&format!("{}/html", name), &context)?;
        let text = self.render_part(&format!("{}/text", name), &context)?;
        let subject = self
            .render_part(&format!("{}/subject", name), &context)?
            .map(|s| s.trim().to_string());

        if html.is_none() && text.is_none() {
            return Err(MailError::TemplateNotFound(name.to_string()));
        }

        Ok(RenderedTemplate {
            html,
            text,
            subject,
        })
    }

    fn has_template(&self, name: &str) -> bool {
        self.tera.contains_template(&format!("{}/html", name))
            || self.tera.contains_template(&format!("{}/text", name))
    }

    fn register_template(&mut self, name: &str, content: &str) -> Result<()> {
        self.tera
            .add_raw_template(&format!("{}/html", name), content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_tera_render() {
        let mut engine = TeraEngine::new();
        engine
            .register_template("test", "<h1>Hello, {{ name }}!</h1>")
            .unwrap();
        engine
            .register_raw("test/text", "Hello, {{ name }}!")
            .unwrap();
        engine
            .register_raw("test/subject", "Welcome {{ name }}")
            .unwrap();

        let result = engine.render("test", &json!({"name": "World"})).unwrap();

        assert_eq!(result.html.as_deref(), Some("<h1>Hello, World!</h1>"));
        assert_eq!(result.text.as_deref(), Some("Hello, World!"));
        assert_eq!(result.subject.as_deref(), Some("Welcome World"));
    }

    #[test]
    fn test_tera_has_template_and_missing() {
        let mut engine = TeraEngine::new();
        engine.register_template("known", "hi").unwrap();

        assert!(engine.has_template("known"));
        assert!(!engine.has_template("unknown"));
        assert!(matches!(
            engine.render("unknown", &json!({})),
            Err(MailError::TemplateNotFound(_))
        ));
    }
}
