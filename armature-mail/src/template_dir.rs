//! Shared on-disk template discovery for the three template engines.
//!
//! All three support the same layout — one directory per template, containing
//! `html.<ext>` plus optional `text.<ext>` and `subject.<ext>` — and each used
//! to reimplement the walk (Handlebars hand-unrolled the three parts that Tera
//! and MiniJinja looped over). One scanner means the engines cannot drift on
//! which files they pick up or which names they register.

use std::path::Path;

use crate::{MailError, Result};

/// The parts of a template, in registration order.
pub(crate) const PARTS: [&str; 3] = ["html", "text", "subject"];

/// The escaping applied to the `html` part by *every* engine.
///
/// Each engine ships its own `escape_html`, and the three disagree: Handlebars
/// escapes `` ` `` and `=`, MiniJinja escapes `/`, Tera escapes neither, and
/// Tera and Handlebars even spell `'` differently (`&#39;` vs `&#x27;`). So the
/// same template and the same context produced *different* HTML depending on
/// which engine loaded it, while the crate documents them as interchangeable.
///
/// This is the union of all three sets, so no engine escapes less than it did
/// before, installed on each of them as their escape function. The set matches
/// the OWASP HTML-entity-encoding recommendation.
///
/// `tests/template_conformance.rs` pins the agreement with a fixture covering
/// every character below.
pub(crate) fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    push_escaped(value, &mut out);
    out
}

/// [`escape_html`] into an existing buffer.
pub(crate) fn push_escaped(value: &str, out: &mut String) {
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            '/' => out.push_str("&#x2f;"),
            '=' => out.push_str("&#x3d;"),
            '`' => out.push_str("&#x60;"),
            _ => out.push(c),
        }
    }
}

/// One template file found on disk.
pub(crate) struct TemplatePart {
    /// Registration key: `"<template>/<part>"`.
    pub key: String,
    /// Template directory name, for logging.
    pub template: String,
    /// File contents.
    pub content: String,
}

/// Walk `path`, returning every `<template>/<part>.<ext>` file it contains.
///
/// Non-directory entries at the top level are ignored, so a stray `README.md`
/// alongside the template directories is not mistaken for a template.
pub(crate) fn scan_template_dir(path: &Path, ext: &str) -> Result<Vec<TemplatePart>> {
    if !path.exists() {
        return Err(MailError::Config(format!(
            "Template directory not found: {}",
            path.display()
        )));
    }

    let mut found = Vec::new();

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();

        if !entry_path.is_dir() {
            continue;
        }

        let template = entry_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| MailError::Config("Invalid template directory name".to_string()))?
            .to_string();

        for part in PARTS {
            let part_path = entry_path.join(format!("{part}.{ext}"));
            if part_path.exists() {
                found.push(TemplatePart {
                    key: format!("{template}/{part}"),
                    template: template.clone(),
                    content: std::fs::read_to_string(&part_path)?,
                });
            }
        }
    }

    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_parts_and_ignores_stray_files() {
        let dir = tempfile::tempdir().unwrap();
        let tpl = dir.path().join("welcome");
        std::fs::create_dir_all(&tpl).unwrap();
        std::fs::write(tpl.join("html.hbs"), "h").unwrap();
        std::fs::write(tpl.join("subject.hbs"), "s").unwrap();
        // Wrong extension and a top-level stray file: neither is a template.
        std::fs::write(tpl.join("text.tera"), "t").unwrap();
        std::fs::write(dir.path().join("README.md"), "ignore me").unwrap();

        let found = scan_template_dir(dir.path(), "hbs").unwrap();
        let keys: Vec<&str> = found.iter().map(|p| p.key.as_str()).collect();

        assert_eq!(keys, ["welcome/html", "welcome/subject"]);
        assert_eq!(found[0].template, "welcome");
        assert_eq!(found[0].content, "h");
    }

    /// The shared set is the union of what the three engines escaped
    /// individually, so switching engines cannot weaken escaping.
    #[test]
    fn the_shared_escape_covers_every_engines_set() {
        assert_eq!(
            escape_html(r#"& < > " ' / = `"#),
            "&amp; &lt; &gt; &quot; &#x27; &#x2f; &#x3d; &#x60;"
        );
        assert_eq!(escape_html("plain text"), "plain text");
    }

    #[test]
    fn a_missing_directory_is_a_config_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            scan_template_dir(&dir.path().join("nope"), "hbs"),
            Err(MailError::Config(_))
        ));
    }
}
