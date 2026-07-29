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
    ///
    /// HTML parts are autoescaped; the `text` and `subject` parts are not.
    /// Templates are registered under keys of the form `"<name>/<part>"` (no
    /// file extension), so Tera's default `.html`-suffix autoescape rule would
    /// never match and every part would render unescaped. The `/html` suffix is
    /// registered below so the HTML part escapes.
    ///
    /// All three engines agree on this rule: escaping the `text/plain` body or
    /// the `Subject:` header is a bug (`Bob & Alice` would go out as
    /// `Bob &amp; Alice`), not a safety measure.
    ///
    /// The escape function is [`crate::template_dir::escape_html`] rather than
    /// Tera's built-in, which escapes a different set from Handlebars' and
    /// MiniJinja's — the same template rendered identical-looking output on one
    /// engine and different HTML on another.
    pub fn new() -> Self {
        let mut tera = Tera::new();
        tera.autoescape_on(vec!["/html", ".html", ".htm", ".xml"]);
        tera.set_escape_fn(|input, output| {
            let mut escaped = String::with_capacity(input.len());
            crate::template_dir::push_escaped(input, &mut escaped);
            output.write_all(escaped.as_bytes())
        });
        Self { tera }
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

        for part in crate::template_dir::scan_template_dir(path.as_ref(), EXT)? {
            engine.tera.add_raw_template(&part.key, &part.content)?;
            debug!(template = %part.template, part = %part.key, "Loaded email template (Tera)");
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
    fn html_part_is_autoescaped() {
        let mut engine = TeraEngine::new();
        engine
            .register_template("xss", "<p>{{ name }}</p>")
            .unwrap();

        let result = engine
            .render("xss", &json!({"name": "<script>alert(1)</script>"}))
            .unwrap();
        let html = result.html.unwrap();

        assert!(html.contains("&lt;script&gt;"), "not escaped: {html}");
        assert!(!html.contains("<script>"), "raw script tag present: {html}");
    }

    #[test]
    fn text_and_subject_parts_are_not_escaped() {
        let mut engine = TeraEngine::new();
        engine.register_raw("xss/text", "{{ name }}").unwrap();
        engine.register_raw("xss/subject", "{{ name }}").unwrap();

        let result = engine
            .render("xss", &json!({"name": "Bob & Alice <them>"}))
            .unwrap();

        assert_eq!(result.text.as_deref(), Some("Bob & Alice <them>"));
        assert_eq!(result.subject.as_deref(), Some("Bob & Alice <them>"));
    }

    // `from_directory` is covered for all three engines together by
    // `tests/template_conformance.rs`, which also asserts they agree on which
    // parts escape.

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

    /// `tera_mut` must expose the real, live `tera::Tera` instance — a
    /// function registered through it must actually run when a template
    /// using it is rendered, not just compile.
    #[test]
    fn tera_mut_allows_registering_a_custom_function() {
        let mut engine = TeraEngine::new();
        engine.tera_mut().register_function(
            "shout",
            |kwargs: tera::Kwargs, _state: &tera::State| -> String {
                let text: String = kwargs.get("text").ok().flatten().unwrap_or_default();
                text.to_uppercase()
            },
        );
        engine
            .register_raw("greet/text", "{{ shout(text=name) }}")
            .unwrap();

        let result = engine.render("greet", &json!({"name": "world"})).unwrap();

        assert_eq!(result.text.as_deref(), Some("WORLD"));
    }
}
