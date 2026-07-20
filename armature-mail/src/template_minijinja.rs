//! MiniJinja template engine integration for email templates.
//!
//! This module provides MiniJinja-based email templating capabilities, mirroring
//! the Handlebars engine: templates live in a per-template directory containing
//! `html.jinja`, and optionally `text.jinja` and `subject.jinja`.
//!
//! # Features
//!
//! This module requires the `minijinja` feature to be enabled:
//!
//! ```toml
//! [dependencies]
//! armature-mail = { version = "0.1", features = ["minijinja"] }
//! ```
//!
//! # Example
//!
//! ```rust
//! use armature_mail::{MiniJinjaEngine, TemplateEngine};
//! use serde_json::json;
//!
//! let mut engine = MiniJinjaEngine::new();
//! engine
//!     .register_template("welcome", "<h1>Hello, {{ name }}!</h1>")
//!     .unwrap();
//!
//! let rendered = engine.render("welcome", &json!({ "name": "World" })).unwrap();
//! assert_eq!(rendered.html.as_deref(), Some("<h1>Hello, World!</h1>"));
//! ```

use minijinja::Environment;
use std::path::Path;
use tracing::debug;

use crate::{MailError, RenderedTemplate, Result, TemplateEngine};

/// File extension used for MiniJinja email templates.
const EXT: &str = "jinja";

/// MiniJinja template engine for email rendering.
pub struct MiniJinjaEngine {
    env: Environment<'static>,
}

impl MiniJinjaEngine {
    /// Create a new, empty MiniJinja engine.
    pub fn new() -> Self {
        Self {
            env: Environment::new(),
        }
    }

    /// Load templates from a directory.
    ///
    /// Expected structure:
    /// ```text
    /// templates/
    ///   welcome/
    ///     subject.jinja    (optional)
    ///     html.jinja
    ///     text.jinja       (optional)
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
                        .env
                        .add_template_owned(format!("{}/{}", template_name, part), content)?;
                }
            }

            debug!(template = %template_name, "Loaded email template (MiniJinja)");
        }

        Ok(engine)
    }

    /// Register a raw template under an explicit key (e.g. `"welcome/text"`).
    ///
    /// [`TemplateEngine::register_template`] registers the HTML part; this lets
    /// you register the text and subject parts too.
    pub fn register_raw(&mut self, key: &str, content: &str) -> Result<()> {
        self.env
            .add_template_owned(key.to_string(), content.to_string())?;
        Ok(())
    }

    /// Access the underlying MiniJinja environment (to register filters, …).
    pub fn environment_mut(&mut self) -> &mut Environment<'static> {
        &mut self.env
    }

    fn render_part(&self, key: &str, context: &serde_json::Value) -> Result<Option<String>> {
        match self.env.get_template(key) {
            Ok(template) => Ok(Some(template.render(context)?)),
            // A missing template is not an error here — the caller decides whether
            // the absence of *every* part is fatal.
            Err(e) if e.kind() == minijinja::ErrorKind::TemplateNotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

impl Default for MiniJinjaEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TemplateEngine for MiniJinjaEngine {
    fn render(&self, name: &str, context: &serde_json::Value) -> Result<RenderedTemplate> {
        let html = self.render_part(&format!("{}/html", name), context)?;
        let text = self.render_part(&format!("{}/text", name), context)?;
        let subject = self
            .render_part(&format!("{}/subject", name), context)?
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
        self.env.get_template(&format!("{}/html", name)).is_ok()
            || self.env.get_template(&format!("{}/text", name)).is_ok()
    }

    fn register_template(&mut self, name: &str, content: &str) -> Result<()> {
        self.env
            .add_template_owned(format!("{}/html", name), content.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_minijinja_render() {
        let mut engine = MiniJinjaEngine::new();
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
    fn test_minijinja_has_template_and_missing() {
        let mut engine = MiniJinjaEngine::new();
        engine.register_template("known", "hi").unwrap();

        assert!(engine.has_template("known"));
        assert!(!engine.has_template("unknown"));
        assert!(matches!(
            engine.render("unknown", &json!({})),
            Err(MailError::TemplateNotFound(_))
        ));
    }
}
