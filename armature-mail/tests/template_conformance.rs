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
//!
//! The engines also disagreed on *which characters* the `html` part escapes —
//! each used its own built-in escaper, covering a different set — so the same
//! template rendered different HTML depending on the engine. They now share
//! `template_dir::escape_html`, and the fixture below covers every character
//! any of them ever escaped, so the agreement is pinned rather than assumed.

#![cfg(any(feature = "handlebars", feature = "tera", feature = "minijinja"))]

use armature_mail::{RenderedTemplate, TemplateEngine};
use serde_json::json;
use std::path::Path;

/// A value that is dangerous in HTML and completely ordinary in plain text.
///
/// The `&`, `<`, `>` triple alone is the *intersection* of what the three
/// engines escape, so a fixture limited to it agreed by construction and proved
/// nothing. `/`, `'`, `=` and a backtick are where they actually diverged:
/// Handlebars escaped `` ` `` and `=`, MiniJinja escaped `/`, Tera escaped
/// neither, and Tera spelled `'` as `&#39;` where Handlebars spelled it
/// `&#x27;`. Every one of those characters is in here.
const NAME: &str = "Bob & Alice <them> / 'q' = `x`";

/// A URL, the case the divergence actually bit: `https://example.com/x` came out
/// with escaped slashes on one engine and verbatim on another.
const URL: &str = "https://example.com/x?a=1&b=2";

/// Write the documented `<template>/{html,text,subject}.<ext>` layout.
fn write_templates(root: &Path, ext: &str) {
    let tpl = root.join("welcome");
    std::fs::create_dir_all(&tpl).unwrap();
    std::fs::write(
        tpl.join(format!("html.{ext}")),
        r#"<p>{{ name }}</p><a href="{{ url }}">go</a>"#,
    )
    .unwrap();
    std::fs::write(tpl.join(format!("text.{ext}")), "Hi {{ name }} {{ url }}").unwrap();
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

    engine
        .render("welcome", &json!({ "name": NAME, "url": URL }))
        .unwrap()
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

    // With one engine compiled in, `&all[1..]` is empty and
    // `the_engines_do_not_diverge` asserts nothing at all — it passed under the
    // default feature set (`handlebars` alone) purely by looping zero times.
    // A cross-engine claim needs at least two engines to be a claim.
    #[cfg(all(feature = "handlebars", feature = "tera", feature = "minijinja"))]
    assert_eq!(
        all.len(),
        3,
        "all three engine features are on; all three must be compared"
    );

    all
}

/// `from_directory` loads all three parts, and only the HTML part is escaped —
/// identically on every engine.
#[test]
fn every_engine_loads_all_three_parts_and_escapes_only_html() {
    for (engine, rendered) in rendered_by_every_engine() {
        assert_eq!(
            rendered.html.as_deref(),
            Some(
                "<p>Bob &amp; Alice &lt;them&gt; &#x2f; &#x27;q&#x27; &#x3d; &#x60;x&#x60;</p>\
                 <a href=\"https:&#x2f;&#x2f;example.com&#x2f;x?a&#x3d;1&amp;b&#x3d;2\">go</a>"
            ),
            "{engine}: html part must be escaped with the shared character set"
        );
        assert_eq!(
            rendered.text.as_deref(),
            Some("Hi Bob & Alice <them> / 'q' = `x` https://example.com/x?a=1&b=2"),
            "{engine}: the text/plain body is not HTML and must not be escaped"
        );
        assert_eq!(
            rendered.subject.as_deref(),
            Some("Welcome Bob & Alice <them> / 'q' = `x`"),
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
