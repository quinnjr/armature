//! Response-serialization microbenchmarks.
//!
//! `write_u64` is compared against `format!` deliberately. The crate reimplements
//! decimal formatting for `content-length` and chunk sizes, and a reimplementation
//! that is not measurably faster than the obvious one is just more code to
//! maintain — so the comparison is here, in a form that fails to justify itself if
//! the gap ever closes.

use armature_h1::write::{write_chunk, write_last_chunk, write_u64};
use armature_h1::{HeaderId, HeaderVec, OutBody, ResponseHead, Version, write::write_head};
use bytes::{BufMut, Bytes, BytesMut};
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

const DATE: &[u8] = b"Wed, 29 Jul 2026 23:59:59 GMT";

fn plain_ok() -> ResponseHead {
    ResponseHead {
        status: 200,
        headers: HeaderVec::new(),
    }
}

/// A response as an application would actually send it: content type, caching,
/// and a couple of custom headers on top of what the crate emits itself.
fn typical_ok() -> ResponseHead {
    let mut headers = HeaderVec::new();
    headers.push((
        HeaderId::ContentType,
        Bytes::from_static(b"application/json"),
    ));
    headers.push((
        HeaderId::CacheControl,
        Bytes::from_static(b"no-cache, no-store"),
    ));
    headers.push((
        HeaderId::Other(armature_h1::ByteStr::from_static("x-request-id")),
        Bytes::from_static(b"01JEXAMPLE0000000000000000"),
    ));
    headers.push((
        HeaderId::Other(armature_h1::ByteStr::from_static("x-served-by")),
        Bytes::from_static(b"armature-h1"),
    ));
    ResponseHead {
        status: 200,
        headers,
    }
}

fn bench_write_head(c: &mut Criterion) {
    let plain = plain_ok();
    let typical = typical_ok();
    let body = OutBody::Fixed(Bytes::from_static(b"hello"));

    let mut g = c.benchmark_group("write_head");
    // The buffer is cleared rather than reallocated, matching the connection's
    // reuse of one write buffer per connection. Measuring with a fresh BytesMut
    // per iteration would price an allocation the server never pays.
    let mut out = BytesMut::with_capacity(1024);
    g.bench_function("200_no_handler_headers", |b| {
        b.iter(|| {
            out.clear();
            write_head(
                black_box(&mut out),
                Version::Http11,
                black_box(&plain),
                black_box(&body),
                DATE,
                true,
            );
        })
    });
    g.bench_function("200_four_handler_headers", |b| {
        b.iter(|| {
            out.clear();
            write_head(
                black_box(&mut out),
                Version::Http11,
                black_box(&typical),
                black_box(&body),
                DATE,
                true,
            );
        })
    });
    g.bench_function("204_no_framing_field", |b| {
        // 204 forbids both framing fields, so this is the path that skips them.
        let head = ResponseHead {
            status: 204,
            headers: HeaderVec::new(),
        };
        b.iter(|| {
            out.clear();
            write_head(
                black_box(&mut out),
                Version::Http11,
                black_box(&head),
                black_box(&OutBody::None),
                DATE,
                true,
            );
        })
    });
    g.bench_function("chunked", |b| {
        b.iter(|| {
            out.clear();
            write_head(
                black_box(&mut out),
                Version::Http11,
                black_box(&plain),
                black_box(&OutBody::Chunked),
                DATE,
                true,
            );
        })
    });
    g.finish();
}

fn bench_write_u64(c: &mut Criterion) {
    let mut g = c.benchmark_group("write_u64_vs_format");
    let mut out = BytesMut::with_capacity(64);

    // Three magnitudes: a one-digit chunk size, a typical content-length, and
    // u64::MAX, which exercises the full 20-digit scratch buffer.
    for v in [5u64, 32_768, u64::MAX] {
        g.bench_with_input(format!("write_u64/{v}"), &v, |b, &v| {
            b.iter(|| {
                out.clear();
                write_u64(black_box(&mut out), black_box(v));
            })
        });
        g.bench_with_input(format!("format/{v}"), &v, |b, &v| {
            b.iter(|| {
                out.clear();
                out.put_slice(format!("{}", black_box(v)).as_bytes());
            })
        });
    }
    g.finish();
}

fn bench_chunks(c: &mut Criterion) {
    let mut g = c.benchmark_group("chunked_write");
    let mut out = BytesMut::with_capacity(64 * 1024);
    let small = vec![b'x'; 64];
    let large = vec![b'x'; 16 * 1024];
    let no_trailers = HeaderVec::new();

    g.throughput(criterion::Throughput::Bytes(small.len() as u64));
    g.bench_function("write_chunk/64B", |b| {
        b.iter(|| {
            out.clear();
            write_chunk(black_box(&mut out), black_box(&small));
        })
    });
    g.throughput(criterion::Throughput::Bytes(large.len() as u64));
    g.bench_function("write_chunk/16KiB", |b| {
        b.iter(|| {
            out.clear();
            write_chunk(black_box(&mut out), black_box(&large));
        })
    });
    g.throughput(criterion::Throughput::Elements(1));
    g.bench_function("write_last_chunk/no_trailers", |b| {
        b.iter(|| {
            out.clear();
            write_last_chunk(black_box(&mut out), black_box(&no_trailers));
        })
    });
    g.finish();
}

criterion_group!(benches, bench_write_head, bench_write_u64, bench_chunks);
criterion_main!(benches);
