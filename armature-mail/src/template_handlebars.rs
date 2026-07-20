//! Handlebars template engine integration.

use handlebars::Handlebars;
use std::path::Path;
use tracing::debug;

use crate::{MailError, RenderedTemplate, Result, TemplateEngine};

/// Handlebars-based template engine for emails.
pub struct HandlebarsEngine {
    handlebars: Handlebars<'static>,
}

impl HandlebarsEngine {
    /// Create a new Handlebars engine.
    pub fn new() -> Self {
        let mut handlebars = Handlebars::new();
        handlebars.set_strict_mode(true);
        Self { handlebars }
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

            if entry_path.is_dir() {
                let template_name =
                    entry_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .ok_or_else(|| {
                            MailError::Config("Invalid template directory name".to_string())
                        })?;

                // Load HTML template
                let html_path = entry_path.join("html.hbs");
                if html_path.exists() {
                    let content = std::fs::read_to_string(&html_path)?;
                    engine
                        .handlebars
                        .register_template_string(&format!("{}/html", template_name), content)?;
                }

                // Load text template
                let text_path = entry_path.join("text.hbs");
                if text_path.exists() {
                    let content = std::fs::read_to_string(&text_path)?;
                    engine
                        .handlebars
                        .register_template_string(&format!("{}/text", template_name), content)?;
                }

                // Load subject template
                let subject_path = entry_path.join("subject.hbs");
                if subject_path.exists() {
                    let content = std::fs::read_to_string(&subject_path)?;
                    engine
                        .handlebars
                        .register_template_string(&format!("{}/subject", template_name), content)?;
                }

                debug!(template = template_name, "Loaded email template");
            }
        }

        Ok(engine)
    }

    /// Register helpers.
    pub fn register_helper<H: handlebars::HelperDef + Send + Sync + 'static>(
        mut self,
        name: &str,
        helper: H,
    ) -> Self {
        self.handlebars.register_helper(name, Box::new(helper));
        self
    }

    /// Register a partial template.
    pub fn register_partial(mut self, name: &str, content: &str) -> Result<Self> {
        self.handlebars.register_partial(name, content)?;
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

        let text = if self.handlebars.has_template(&format!("{}/text", name)) {
            Some(self.handlebars.render(&format!("{}/text", name), context)?)
        } else {
            None
        };

        let subject = if self.handlebars.has_template(&format!("{}/subject", name)) {
            Some(
                self.handlebars
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
        self.handlebars
            .register_template_string(&format!("{}/html", name), content)?;
        Ok(())
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
            .handlebars
            .register_template_string("test/html", "<h1>Hello, {{name}}!</h1>")
            .unwrap();
        engine
            .handlebars
            .register_template_string("test/text", "Hello, {{name}}!")
            .unwrap();
        engine
            .handlebars
            .register_template_string("test/subject", "Welcome {{name}}")
            .unwrap();

        let result = engine.render("test", &json!({"name": "World"})).unwrap();

        assert_eq!(result.html.as_deref(), Some("<h1>Hello, World!</h1>"));
        assert_eq!(result.text.as_deref(), Some("Hello, World!"));
        assert_eq!(result.subject.as_deref(), Some("Welcome World"));
    }

    /// The on-disk `<name>/{subject,html,text}.hbs` layout is the documented way
    /// to ship templates, so the loader is load-bearing and was untested.
    #[test]
    fn from_directory_loads_all_three_parts() {
        let dir = std::env::temp_dir().join(format!("armature-mail-hbs-{}", uuid::Uuid::new_v4()));
        let tpl = dir.join("welcome");
        std::fs::create_dir_all(&tpl).unwrap();
        std::fs::write(tpl.join("html.hbs"), "<p>{{name}}</p>").unwrap();
        std::fs::write(tpl.join("text.hbs"), "Hi {{name}}").unwrap();
        std::fs::write(tpl.join("subject.hbs"), "Welcome {{name}}\n").unwrap();
        // A stray file at the top level must be ignored, not treated as a template.
        std::fs::write(dir.join("README.md"), "ignore me").unwrap();

        let engine = HandlebarsEngine::from_directory(&dir).unwrap();

        assert!(engine.has_template("welcome"));
        assert!(!engine.has_template("missing"));

        let result = engine.render("welcome", &json!({"name": "<b>"})).unwrap();
        // Handlebars escapes unconditionally — the behavior the Tera and
        // MiniJinja engines now match.
        assert_eq!(result.html.as_deref(), Some("<p>&lt;b&gt;</p>"));
        assert_eq!(result.text.as_deref(), Some("Hi &lt;b&gt;"));
        assert_eq!(result.subject.as_deref(), Some("Welcome &lt;b&gt;"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn from_directory_errors_when_the_path_is_missing() {
        let missing = std::env::temp_dir().join("armature-mail-hbs-does-not-exist-4f2a");
        assert!(matches!(
            HandlebarsEngine::from_directory(&missing),
            Err(MailError::Config(_))
        ));
    }
}
