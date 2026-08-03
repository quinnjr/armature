//! Parse-path microbenchmarks: `parse_head`, header interning, and framing.
//!
//! These three run on every request in that order, so they are the crate's hot
//! path in the literal sense. The request shapes are chosen to separate fixed
//! cost from per-header cost: a minimal GET, a browser-sized GET, and a
//! 64-header request that sits at the `max_headers` default. If the browser case
//! costs proportionally more than the minimal one per header, header projection
//! has regressed into copying — which the allocation test asserts and this
//! measures the price of.

use armature_h1::{HeaderId, Limits, framing, parse_head};
use bytes::Bytes;
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

const MINIMAL_GET: &[u8] = b"GET / HTTP/1.1\r\nHost: a.example\r\n\r\n";

const BROWSER_GET: &[u8] = b"GET /index.html?q=1 HTTP/1.1\r\n\
Host: a.example\r\n\
User-Agent: Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36\r\n\
Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8\r\n\
Accept-Language: en-US,en;q=0.9\r\n\
Accept-Encoding: gzip, deflate, br, zstd\r\n\
Connection: keep-alive\r\n\
Upgrade-Insecure-Requests: 1\r\n\
Sec-Fetch-Dest: document\r\n\
Sec-Fetch-Mode: navigate\r\n\
Sec-Fetch-Site: none\r\n\
Cache-Control: max-age=0\r\n\
\r\n";

/// A request at the `max_headers` default, to price the per-header slope where
/// `HeaderVec`'s 16 inline slots have long since spilled to the heap.
fn many_headers() -> Bytes {
    let mut buf = Vec::with_capacity(4096);
    buf.extend_from_slice(b"GET / HTTP/1.1\r\nHost: a.example\r\n");
    for i in 0..63 {
        buf.extend_from_slice(format!("x-custom-header-{i}: value-{i}\r\n").as_bytes());
    }
    buf.extend_from_slice(b"\r\n");
    Bytes::from(buf)
}

fn bench_parse_head(c: &mut Criterion) {
    let limits = Limits::default();
    let minimal = Bytes::from_static(MINIMAL_GET);
    let browser = Bytes::from_static(BROWSER_GET);
    let many = many_headers();

    let mut g = c.benchmark_group("parse_head");
    // Throughput in bytes makes the three shapes comparable despite differing
    // wildly in size; ns/iter alone says only that a longer request takes longer.
    g.throughput(criterion::Throughput::Bytes(minimal.len() as u64));
    g.bench_function("minimal_get", |b| {
        b.iter(|| parse_head(black_box(&minimal), black_box(&limits)).expect("parses"))
    });
    g.throughput(criterion::Throughput::Bytes(browser.len() as u64));
    g.bench_function("browser_get_11_headers", |b| {
        b.iter(|| parse_head(black_box(&browser), black_box(&limits)).expect("parses"))
    });
    g.throughput(criterion::Throughput::Bytes(many.len() as u64));
    g.bench_function("64_headers", |b| {
        b.iter(|| parse_head(black_box(&many), black_box(&limits)).expect("parses"))
    });
    g.finish();
}

fn bench_header_id(c: &mut Criterion) {
    let mut g = c.benchmark_group("HeaderId::from_bytes");
    // A well-known name resolves through the length-bucketed match; a custom one
    // falls through every arm and returns None, leaving the caller to intern it.
    // The gap between these two is the price of an unrecognized header name.
    g.bench_function("well_known_content_length", |b| {
        b.iter(|| HeaderId::from_bytes(black_box(b"content-length")))
    });
    g.bench_function("well_known_host", |b| {
        b.iter(|| HeaderId::from_bytes(black_box(b"host")))
    });
    g.bench_function("custom_same_length_as_known", |b| {
        // Same length as "content-length", so it reaches that bucket's
        // comparisons before failing — the worst case for the lookup.
        b.iter(|| HeaderId::from_bytes(black_box(b"x-tenant-idxxx")))
    });
    g.bench_function("custom_short", |b| {
        b.iter(|| HeaderId::from_bytes(black_box(b"x-req")))
    });
    g.finish();
}

fn bench_framing(c: &mut Criterion) {
    let limits = Limits::default();
    let cases: [(&str, &[u8]); 4] = [
        ("no_body", MINIMAL_GET),
        (
            "content_length",
            b"POST / HTTP/1.1\r\nHost: a.example\r\nContent-Length: 128\r\n\r\n",
        ),
        (
            "chunked",
            b"POST / HTTP/1.1\r\nHost: a.example\r\nTransfer-Encoding: chunked\r\n\r\n",
        ),
        (
            "browser_get",
            // Eleven headers to scan for framing fields rather than two: `decide`
            // walks the header list, so its cost tracks header count, not body.
            BROWSER_GET,
        ),
    ];

    let mut g = c.benchmark_group("framing::decide");
    for (name, raw) in cases {
        let buf = Bytes::from_static(raw);
        let (head, _) = parse_head(&buf, &limits)
            .expect("parses")
            .expect("complete head");
        g.bench_function(name, |b| {
            b.iter(|| framing::decide(black_box(&head), black_box(&limits)))
        });
    }
    g.finish();
}

criterion_group!(benches, bench_parse_head, bench_header_id, bench_framing);
criterion_main!(benches);
