//! Queue and cron primitive benchmarks.
//!
//! Times `armature-queue` job/config construction and serialization, and
//! `armature-cron` expression parsing and next-occurrence computation. No
//! broker, scheduler or storage backend is involved - these are the in-process
//! data structures only.
//!
//! ```bash
//! cargo bench --bench data_benchmarks
//! ```

#![allow(deprecated)]
#![allow(clippy::needless_question_mark)]

use armature_cron::{CronExpression, expression::CronPresets};
use armature_queue::{Job as QueueJob, JobPriority, QueueConfig};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use serde_json::json;
use std::hint::black_box;

fn bench_queue_job_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("queue_job");

    group.bench_function("job_new", |b| {
        b.iter(|| {
            QueueJob::new(
                black_box("default"),
                black_box("send_email"),
                black_box(json!({"to": "user@example.com"})),
            )
        })
    });

    group.bench_function("uuid_generation", |b| {
        b.iter(|| {
            let _id = uuid::Uuid::new_v4();
        })
    });

    group.bench_function("json_serialization", |b| {
        let data = json!({"type": "test", "payload": {"key": "value"}});
        b.iter(|| serde_json::to_string(black_box(&data)).unwrap())
    });

    group.bench_function("json_parsing", |b| {
        let json_str = r#"{"type":"test","payload":{"key":"value"}}"#;
        b.iter(|| serde_json::from_str::<serde_json::Value>(black_box(json_str)).unwrap())
    });

    group.finish();
}

fn bench_queue_config(c: &mut Criterion) {
    c.bench_function("queue_config_new", |b| {
        b.iter(|| QueueConfig::new(black_box("redis://localhost:6379"), black_box("default")))
    });
}

fn bench_cron_expression_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("cron_expression");

    let expressions = vec![
        "0 0 * * * *",        // Every hour
        "*/5 * * * * *",      // Every 5 seconds
        "0 0 12 * * MON-FRI", // Weekdays at noon
        "0 0 0 1 * *",        // First of month
    ];

    for expr in expressions {
        group.bench_with_input(BenchmarkId::from_parameter(expr), expr, |b, expr| {
            b.iter(|| CronExpression::parse(black_box(expr)).unwrap())
        });
    }

    group.bench_function("parse_preset_hourly", |b| {
        b.iter(|| CronExpression::parse(black_box(CronPresets::EVERY_HOUR)).unwrap())
    });

    group.bench_function("parse_preset_daily", |b| {
        b.iter(|| CronExpression::parse(black_box(CronPresets::DAILY)).unwrap())
    });

    group.bench_function("string_split", |b| {
        let expr = "0 0 * * * *";
        b.iter(|| {
            let parts: Vec<&str> = black_box(expr).split_whitespace().collect();
            black_box(parts);
        })
    });

    group.finish();
}

fn bench_job_priority(c: &mut Criterion) {
    c.bench_function("enum_comparison", |b| {
        b.iter(|| {
            let p1 = JobPriority::Critical;
            let p2 = JobPriority::High;
            black_box(p1 == p2);
        })
    });
}

criterion_group!(
    data_benches,
    bench_queue_job_creation,
    bench_queue_config,
    bench_cron_expression_parsing,
    bench_job_priority,
);

criterion_main!(data_benches);
