//! Validator benchmarks.
//!
//! Times the `armature-validation` built-ins - email, URL, UUID, string length
//! and character-class checks, numeric ranges, custom regex patterns - and the
//! `ValidationRules`/`ValidationBuilder` layer that drives them. Every
//! validator here is synchronous and CPU-bound; no request or I/O is involved.
//!
//! ```bash
//! cargo bench --bench validation_benchmarks
//! ```

#![allow(deprecated)]
#![allow(clippy::needless_question_mark)]

use armature_validation::*;
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

fn bench_email_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("email_validation");

    let valid_emails = vec![
        "user@example.com",
        "test.user+tag@domain.co.uk",
        "admin@subdomain.example.com",
    ];

    let invalid_emails = vec!["invalid-email", "@example.com", "user@"];

    group.bench_function("valid_emails", |b| {
        b.iter(|| {
            for email in &valid_emails {
                IsEmail::validate(black_box(email), "email").unwrap();
            }
        })
    });

    group.bench_function("invalid_emails", |b| {
        b.iter(|| {
            for email in &invalid_emails {
                let _ = IsEmail::validate(black_box(email), "email");
            }
        })
    });

    group.finish();
}

fn bench_url_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("url_validation");

    let valid_urls = vec![
        "https://example.com",
        "http://localhost:8080/path",
        "https://subdomain.example.com/path?query=value",
    ];

    group.bench_function("validate_urls", |b| {
        b.iter(|| {
            for url in &valid_urls {
                IsUrl::validate(black_box(url), "url").unwrap();
            }
        })
    });

    group.finish();
}

fn bench_string_validators(c: &mut Criterion) {
    let mut group = c.benchmark_group("string_validators");

    let text = "Hello World 123";

    group.bench_function("min_length", |b| {
        let validator = MinLength(5);
        b.iter(|| validator.validate(black_box(text), "text"))
    });

    group.bench_function("max_length", |b| {
        let validator = MaxLength(100);
        b.iter(|| validator.validate(black_box(text), "text"))
    });

    group.bench_function("is_alpha", |b| {
        b.iter(|| IsAlpha::validate(black_box("HelloWorld"), "text"))
    });

    group.bench_function("is_alphanumeric", |b| {
        b.iter(|| IsAlphanumeric::validate(black_box("Hello123"), "text"))
    });

    group.bench_function("is_numeric", |b| {
        b.iter(|| IsNumeric::validate(black_box("12345"), "text"))
    });

    group.bench_function("not_empty", |b| {
        b.iter(|| NotEmpty::validate(black_box(text), "text"))
    });

    group.bench_function("string_length", |b| {
        b.iter(|| {
            let len = black_box(text).len();
            black_box(len);
        })
    });

    group.finish();
}

fn bench_numeric_validators(c: &mut Criterion) {
    let mut group = c.benchmark_group("numeric_validators");

    group.bench_function("min_i32", |b| {
        let validator = Min(10i32);
        b.iter(|| validator.validate(black_box(50i32), "value"))
    });

    group.bench_function("max_i32", |b| {
        let validator = Max(100i32);
        b.iter(|| validator.validate(black_box(50i32), "value"))
    });

    group.bench_function("in_range_i32", |b| {
        let validator = InRange {
            min: 1i32,
            max: 100i32,
        };
        b.iter(|| validator.validate(black_box(50i32), "value"))
    });

    group.bench_function("is_positive_f64", |b| {
        b.iter(|| IsPositive::validate_f64(black_box(42.0), "value"))
    });

    group.finish();
}

fn bench_pattern_validation(c: &mut Criterion) {
    use regex::Regex;

    let mut group = c.benchmark_group("pattern_validation");

    let phone_regex = Regex::new(r"^\d{3}-\d{3}-\d{4}$").unwrap();
    let matches_validator = Matches(phone_regex);

    group.bench_function("matches_phone", |b| {
        b.iter(|| matches_validator.validate(black_box("123-456-7890"), "phone"))
    });

    group.bench_function("is_uuid", |b| {
        b.iter(|| IsUuid::validate(black_box("550e8400-e29b-41d4-a716-446655440000"), "uuid"))
    });

    group.bench_function("regex_compile", |b| {
        b.iter(|| {
            let _regex = Regex::new(black_box(r"^\d{3}-\d{3}-\d{4}$")).unwrap();
        })
    });

    group.finish();
}

fn bench_validation_rules(c: &mut Criterion) {
    let mut group = c.benchmark_group("validation_rules");

    let rules = ValidationRules::for_field("username");
    let rules = rules.add(|value: &str, field: &str| {
        if value.len() < 3 {
            Err(ValidationError::new(field, "must be at least 3 characters"))
        } else {
            Ok(())
        }
    });
    let rules = rules.add(|value: &str, field: &str| {
        if value.len() > 50 {
            Err(ValidationError::new(field, "must be at most 50 characters"))
        } else {
            Ok(())
        }
    });

    group.bench_function("single_rule", |b| {
        b.iter(|| rules.validate(black_box("john_doe123")))
    });

    group.finish();
}

criterion_group!(
    validation_benches,
    bench_email_validation,
    bench_url_validation,
    bench_string_validators,
    bench_numeric_validators,
    bench_pattern_validation,
    bench_validation_rules,
);

criterion_main!(validation_benches);
