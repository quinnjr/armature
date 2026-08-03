//! Router dispatch: method-indexed trees against the linear scan they replaced.
//!
//! The interesting axis is route-table size. A linear scan is competitive at
//! four routes and hopeless at four hundred; a tree is flat. Both ends are
//! measured so the claim is a curve rather than one number.

use armature_core::{Error, HttpRequest, HttpResponse, Router};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;

fn router_with(n: usize) -> Router {
    let mut r = Router::new();
    for i in 0..n {
        r.get(format!("/route{i}/:id"), |_req: HttpRequest| async {
            Ok::<_, Error>(HttpResponse::new(200))
        });
    }
    r
}

fn bench_match(c: &mut Criterion) {
    let mut g = c.benchmark_group("router/match_route");
    for n in [4usize, 32, 128, 512] {
        let router = router_with(n);
        // Match the *last* registered route: the worst case for a linear scan and
        // the same case as any other for a tree.
        let target = format!("/route{}/42", n - 1);
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let m = router.match_route("GET", black_box(&target));
                assert!(m.is_some());
                m
            })
        });
    }
    g.finish();
}

fn bench_method_miss(c: &mut Criterion) {
    // A method with no routes registered: the tree array short-circuits, where a
    // scan compared every route's method string.
    let router = router_with(128);
    c.bench_function("router/method_miss", |b| {
        b.iter(|| router.match_route("DELETE", black_box("/route0/1")))
    });
}

criterion_group!(benches, bench_match, bench_method_miss);
criterion_main!(benches);
