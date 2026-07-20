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

use minijinja::{AutoEscape, Environment};
use std::path::Path;
use tracing::debug;

use crate::{MailError, RenderedTemplate, Result, TemplateEngine};

/// File extension used for MiniJinja email templates.
const EXT: &str = "jinja";

/// Name of the crate's shared escaping format, as seen by MiniJinja.
///
/// [`AutoEscape::Html`] would use MiniJinja's own escaper, which disagrees with
/// Handlebars' and Tera's; a custom format routes through
/// [`crate::template_dir::escape_html`] instead.
const ESCAPE_FORMAT: &str = "armature-mail/html";

/// MiniJinja template engine for email rendering.
pub struct MiniJinjaEngine {
    env: Environment<'static>,
}

impl MiniJinjaEngine {
    /// Create a new, empty MiniJinja engine.
    ///
    /// HTML parts are autoescaped; the `text` and `subject` parts are not.
    /// Templates are registered under keys of the form `"<name>/<part>"` (no
    /// file extension), so MiniJinja's default extension-based autoescape
    /// callback would never match and every part would render unescaped. The
    /// callback below keys off the `/html` suffix instead.
    ///
    /// All three engines agree on this rule: escaping the `text/plain` body or
    /// the `Subject:` header is a bug (`Bob & Alice` would go out as
    /// `Bob &amp; Alice`), not a safety measure.
    /// The escaping applied to the `html` part is
    /// [`crate::template_dir::escape_html`], not MiniJinja's built-in HTML
    /// escaper: the built-ins of the three engines cover different character
    /// sets, so the same template produced different HTML depending on which
    /// engine loaded it. That is what [`ESCAPE_FORMAT`] and the custom formatter
    /// below exist for.
    pub fn new() -> Self {
        let mut env = Environment::new();
        env.set_auto_escape_callback(|name| {
            if name.ends_with("/html") {
                AutoEscape::Custom(ESCAPE_FORMAT)
            } else {
                AutoEscape::None
            }
        });
        env.set_formatter(|out, state, value| {
            match state.auto_escape() {
                // `|safe` output is already trusted markup — escaping it here
                // would double-escape every partial and `{{ x|safe }}`.
                AutoEscape::Custom(ESCAPE_FORMAT) if !value.is_safe() => {
                    out.write_str(&crate::template_dir::escape_html(&value.to_string()))?;
                }
                _ => write!(out, "{value}")?,
            }
            Ok(())
        });
        Self { env }
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

        for part in crate::template_dir::scan_template_dir(path.as_ref(), EXT)? {
            engine
                .env
                .add_template_owned(part.key.clone(), part.content)?;
            debug!(template = %part.template, part = %part.key, "Loaded email template (MiniJinja)");
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
    fn html_part_is_autoescaped() {
        let mut engine = MiniJinjaEngine::new();
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
        let mut engine = MiniJinjaEngine::new();
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

    /// `environment_mut` must expose the real, live `minijinja::Environment` —
    /// a filter registered through it must actually run when a template using
    /// it is rendered, not just compile.
    #[test]
    fn environment_mut_allows_registering_a_custom_filter() {
        let mut engine = MiniJinjaEngine::new();
        engine
            .environment_mut()
            .add_filter("shout", |v: String| v.to_uppercase());
        engine
            .register_raw("greet/text", "{{ name|shout }}")
            .unwrap();

        let result = engine.render("greet", &json!({"name": "world"})).unwrap();

        assert_eq!(result.text.as_deref(), Some("WORLD"));
    }
}
