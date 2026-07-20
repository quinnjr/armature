//! The three template engines must agree.
//!
//! Each engine had its own `from_directory` test asserting its own escaping
//! behavior, and those tests *encoded a divergence*: Handlebars escaped all
//! three parts while Tera and MiniJinja escaped only `html`. So a user named
//! `Bob & Alice` reached the `text/plain` body and the `Subject:` header as
//! `Bob &amp; Alice` on the default engine, and correctly on the other two —
//! and the tests called that expected.
//!
//! This is the single test that replaces those three. It loads the same
//! directory layout into every compiled-in engine and asserts they produce
//! identical output for all three parts.

#![cfg(any(feature = "handlebars", feature = "tera", feature = "minijinja"))]

use armature_mail::{RenderedTemplate, TemplateEngine};
use serde_json::json;
use std::path::Path;

/// A value that is dangerous in HTML and completely ordinary in plain text.
const NAME: &str = "Bob & Alice <them>";

/// Write the documented `<template>/{html,text,subject}.<ext>` layout.
fn write_templates(root: &Path, ext: &str) {
    let tpl = root.join("welcome");
    std::fs::create_dir_all(&tpl).unwrap();
    std::fs::write(tpl.join(format!("html.{ext}")), "<p>{{ name }}</p>").unwrap();
    std::fs::write(tpl.join(format!("text.{ext}")), "Hi {{ name }}").unwrap();
    std::fs::write(tpl.join(format!("subject.{ext}")), "Welcome {{ name }}\n").unwrap();
    // A stray file at the top level must be ignored, not treated as a template.
    std::fs::write(root.join("README.md"), "ignore me").unwrap();
}

/// Load `welcome` from a fresh temp directory and render it.
///
/// `tempfile` rather than a hand-rolled `std::env::temp_dir()` path: each of the
/// three old tests built its own uuid-suffixed directory and cleaned up with a
/// best-effort `remove_dir_all` that a failing assertion skipped entirely,
/// leaking a temp tree on every red run.
fn render_with<E, F>(ext: &str, from_directory: F) -> RenderedTemplate
where
    E: TemplateEngine,
    F: FnOnce(&Path) -> E,
{
    let dir = tempfile::tempdir().unwrap();
    write_templates(dir.path(), ext);

    let engine = from_directory(dir.path());
    assert!(engine.has_template("welcome"));
    assert!(!engine.has_template("missing"));

    engine.render("welcome", &json!({ "name": NAME })).unwrap()
}

/// Every compiled-in engine, as `(name, rendered)`.
fn rendered_by_every_engine() -> Vec<(&'static str, RenderedTemplate)> {
    let mut all: Vec<(&'static str, RenderedTemplate)> = Vec::new();

    #[cfg(feature = "handlebars")]
    all.push((
        "handlebars",
        render_with("hbs", |p| {
            armature_mail::HandlebarsEngine::from_directory(p).unwrap()
        }),
    ));

    #[cfg(feature = "tera")]
    all.push((
        "tera",
        render_with("tera", |p| {
            armature_mail::TeraEngine::from_directory(p).unwrap()
        }),
    ));

    #[cfg(feature = "minijinja")]
    all.push((
        "minijinja",
        render_with("jinja", |p| {
            armature_mail::MiniJinjaEngine::from_directory(p).unwrap()
        }),
    ));

    assert!(!all.is_empty(), "no template engine feature is enabled");
    all
}

/// `from_directory` loads all three parts, and only the HTML part is escaped —
/// identically on every engine.
#[test]
fn every_engine_loads_all_three_parts_and_escapes_only_html() {
    for (engine, rendered) in rendered_by_every_engine() {
        assert_eq!(
            rendered.html.as_deref(),
            Some("<p>Bob &amp; Alice &lt;them&gt;</p>"),
            "{engine}: html part must be escaped"
        );
        assert_eq!(
            rendered.text.as_deref(),
            Some("Hi Bob & Alice <them>"),
            "{engine}: the text/plain body is not HTML and must not be escaped"
        );
        assert_eq!(
            rendered.subject.as_deref(),
            Some("Welcome Bob & Alice <them>"),
            "{engine}: the Subject header is not HTML and must not be escaped"
        );
    }
}

/// The decisive assertion: no engine may differ from any other on any part.
#[test]
fn the_engines_do_not_diverge() {
    let all = rendered_by_every_engine();
    let (first_name, first) = &all[0];

    for (engine, rendered) in &all[1..] {
        assert_eq!(
            rendered.html, first.html,
            "{engine} and {first_name} disagree on the html part"
        );
        assert_eq!(
            rendered.text, first.text,
            "{engine} and {first_name} disagree on the text part"
        );
        assert_eq!(
            rendered.subject, first.subject,
            "{engine} and {first_name} disagree on the subject part"
        );
    }
}
