//! Handlebars template engine integration.

use handlebars::Handlebars;
use std::path::Path;
use tracing::debug;

use crate::{MailError, RenderedTemplate, Result, TemplateEngine};

/// File extension used for Handlebars email templates.
const EXT: &str = "hbs";

/// Handlebars-based template engine for emails.
///
/// Two registries, not one. Handlebars escapes every `{{ }}` expansion by
/// default, and that default was applied to all three parts — so a user named
/// `Bob & Alice` went out as `Bob &amp; Alice` in the `text/plain` body *and*
/// in the `Subject:` header. HTML escaping belongs to the HTML part only, which
/// is what the Tera and MiniJinja engines already did.
pub struct HandlebarsEngine {
    /// Renders the `html` part, with escaping on.
    handlebars: Handlebars<'static>,
    /// Renders the `text` and `subject` parts, with escaping off.
    plain: Handlebars<'static>,
}

impl HandlebarsEngine {
    /// Create a new Handlebars engine.
    ///
    /// The `html` part is HTML-escaped; the `text` and `subject` parts are not.
    /// All three engines agree on this rule, and — since they escape via
    /// [`crate::template_dir::escape_html`] rather than their own built-ins —
    /// on the exact character set as well.
    pub fn new() -> Self {
        let mut handlebars = Handlebars::new();
        handlebars.set_strict_mode(true);
        handlebars.register_escape_fn(crate::template_dir::escape_html);

        let mut plain = Handlebars::new();
        plain.set_strict_mode(true);
        plain.register_escape_fn(handlebars::no_escape);

        Self { handlebars, plain }
    }

    /// Register one template string into both registries.
    ///
    /// Every template is available in both; which registry actually renders it
    /// is decided by the part, in [`TemplateEngine::render`].
    fn register(&mut self, key: &str, content: &str) -> Result<()> {
        self.handlebars.register_template_string(key, content)?;
        self.plain.register_template_string(key, content)?;
        Ok(())
    }

    /// Load templates from a directory.
    ///
    /// Expected structure:
    /// ```text
    /// templates/
    ///   welcome/
    ///     subject.hbs      (optional)
    ///     html.hbs
    ///     text.hbs         (optional)
    ///   password_reset/
    ///     subject.hbs
    ///     html.hbs
    ///     text.hbs
    /// ```
    pub fn from_directory(path: impl AsRef<Path>) -> Result<Self> {
        let mut engine = Self::new();

        for part in crate::template_dir::scan_template_dir(path.as_ref(), EXT)? {
            engine.register(&part.key, &part.content)?;
            debug!(template = %part.template, part = %part.key, "Loaded email template");
        }

        Ok(engine)
    }

    /// Register helpers.
    ///
    /// Registered in both registries, so a helper is available whichever part
    /// uses it.
    pub fn register_helper<H: handlebars::HelperDef + Send + Sync + Clone + 'static>(
        mut self,
        name: &str,
        helper: H,
    ) -> Self {
        self.handlebars
            .register_helper(name, Box::new(helper.clone()));
        self.plain.register_helper(name, Box::new(helper));
        self
    }

    /// Register a partial template.
    pub fn register_partial(mut self, name: &str, content: &str) -> Result<Self> {
        self.handlebars.register_partial(name, content)?;
        self.plain.register_partial(name, content)?;
        Ok(self)
    }
}

impl Default for HandlebarsEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TemplateEngine for HandlebarsEngine {
    fn render(&self, name: &str, context: &serde_json::Value) -> Result<RenderedTemplate> {
        let html = if self.handlebars.has_template(&format!("{}/html", name)) {
            Some(self.handlebars.render(&format!("{}/html", name), context)?)
        } else {
            None
        };

        // `text` and `subject` render from the non-escaping registry: neither is
        // HTML, and escaping them corrupts the message (`Bob & Alice` became
        // `Bob &amp; Alice` in the plain-text body and the `Subject:` header).
        let text = if self.plain.has_template(&format!("{}/text", name)) {
            Some(self.plain.render(&format!("{}/text", name), context)?)
        } else {
            None
        };

        let subject = if self.plain.has_template(&format!("{}/subject", name)) {
            Some(
                self.plain
                    .render(&format!("{}/subject", name), context)?
                    .trim()
                    .to_string(),
            )
        } else {
            None
        };

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
        self.handlebars.has_template(&format!("{}/html", name))
            || self.handlebars.has_template(&format!("{}/text", name))
    }

    fn register_template(&mut self, name: &str, content: &str) -> Result<()> {
        self.register(&format!("{}/html", name), content)
    }
}

impl HandlebarsEngine {
    /// Register a raw template under an explicit key (e.g. `"welcome/text"`).
    ///
    /// [`TemplateEngine::register_template`] registers the HTML part; this lets
    /// you register the text and subject parts too, matching `TeraEngine` and
    /// `MiniJinjaEngine`'s `register_raw`.
    pub fn register_raw(&mut self, key: &str, content: &str) -> Result<()> {
        self.register(key, content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_handlebars_render() {
        let mut engine = HandlebarsEngine::new();
        engine
            .register_template("test", "<h1>Hello, {{name}}!</h1>")
            .unwrap();
        engine
            .register_raw("test/text", "Hello, {{name}}!")
            .unwrap();
        engine
            .register_raw("test/subject", "Welcome {{name}}")
            .unwrap();

        let result = engine.render("test", &json!({"name": "World"})).unwrap();

        assert_eq!(result.html.as_deref(), Some("<h1>Hello, World!</h1>"));
        assert_eq!(result.text.as_deref(), Some("Hello, World!"));
        assert_eq!(result.subject.as_deref(), Some("Welcome World"));
    }

    #[test]
    fn html_part_is_autoescaped() {
        let mut engine = HandlebarsEngine::new();
        engine.register_template("xss", "<p>{{name}}</p>").unwrap();

        let result = engine
            .render("xss", &json!({"name": "<script>alert(1)</script>"}))
            .unwrap();
        let html = result.html.unwrap();

        assert!(html.contains("&lt;script&gt;"), "not escaped: {html}");
        assert!(!html.contains("<script>"), "raw script tag present: {html}");
    }

    /// Handlebars escaped *every* part, so a user named `Bob & Alice` reached
    /// the `text/plain` body and the `Subject:` header as `Bob &amp; Alice`.
    /// Neither part is HTML; neither may be escaped.
    #[test]
    fn text_and_subject_parts_are_not_escaped() {
        let mut engine = HandlebarsEngine::new();
        engine.register_raw("xss/text", "{{name}}").unwrap();
        engine.register_raw("xss/subject", "{{name}}").unwrap();

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
    fn from_directory_errors_when_the_path_is_missing() {
        let missing = std::env::temp_dir().join("armature-mail-hbs-does-not-exist-4f2a");
        assert!(matches!(
            HandlebarsEngine::from_directory(&missing),
            Err(MailError::Config(_))
        ));
    }
}
