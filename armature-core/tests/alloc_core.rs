//! Allocation budget for the migrated request path.
//!
//! Unlike `armature-h1`'s `alloc_regression.rs`, this asserts a *budget* rather
//! than zero. `armature-core` still allocates on this path by construction —
//! hyper's types, `Arc`-based DI, boxed handler futures — and removing those is
//! later work. What must hold now is that the migrated types no longer
//! contribute, so each budget below says what it covers. A number that only ever
//! goes down is worth having; a threshold nobody revisits is not.

use armature_core::{Error, HttpRequest, HttpResponse, Router};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

// Per-thread rather than process-wide. `cargo test` runs test functions on
// separate threads concurrently, and a shared static counter would attribute
// every other test's allocations to whichever test happens to be armed.
//
// `const`-initialized so the TLS slot needs no lazy-init check and registers no
// destructor: both would themselves allocate, from inside the allocator.
thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
    static COUNTING: Cell<bool> = const { Cell::new(false) };
}

/// Count one allocation against the current thread, if it is armed.
///
/// `try_with` rather than `with`: during TLS teardown the slot is gone, and a
/// panic out of `GlobalAlloc` would abort the process.
fn tick() {
    if COUNTING.try_with(Cell::get).unwrap_or(false) {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
    }
}

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        tick();
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        tick();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        tick();
        unsafe { System.alloc_zeroed(layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Allocations performed while running `f`, on this thread.
///
/// The result is dropped after counting stops, so a deallocation-triggered
/// realloc in someone else's `Drop` cannot be charged to the measurement.
fn count<T>(f: impl FnOnce() -> T) -> u64 {
    ALLOCS.with(|c| c.set(0));
    COUNTING.with(|c| c.set(true));
    let out = f();
    COUNTING.with(|c| c.set(false));
    drop(out);
    ALLOCS.with(Cell::get)
}

/// Constructing a request from static strings: one `ByteStr` copy of the target,
/// and nothing else. `GET` is a unit variant so the method costs nothing, and
/// `HeaderMap`, `RouteParams` and `Extensions` are all inline with a cold query
/// cache.
const BUDGET_CONSTRUCT: u64 = 1;

#[test]
fn constructing_a_request_costs_only_its_target() {
    let n = count(|| HttpRequest::new("GET", "/users/42"));
    println!("construct: {n} allocations");
    assert!(
        n <= BUDGET_CONSTRUCT,
        "constructing a request cost {n} allocations, budget is {BUDGET_CONSTRUCT}"
    );
}

/// Reading a query the handler ignores must cost nothing at all.
#[test]
fn an_unread_query_string_costs_nothing() {
    let req = HttpRequest::new("GET", "/s?a=1&b=2&c=hello%20world");
    let n = count(|| req.headers.len());
    println!("unread query: {n} allocations");
    assert_eq!(
        n, 0,
        "a query string no handler reads must not be parsed or decoded"
    );
}

/// Six typical request headers, all with well-known names.
///
/// One `Bytes` copy per value and zero for the names — `HeaderId` interning
/// resolves all six to enum variants, and the `SmallVec` holds them inline.
const BUDGET_SIX_HEADERS: u64 = 6;

#[test]
fn well_known_header_names_cost_no_allocation() {
    let mut req = HttpRequest::new("GET", "/");
    let n = count(|| {
        req.headers.insert("host", "a.example");
        req.headers.insert("accept", "*/*");
        req.headers.insert("accept-encoding", "gzip");
        req.headers.insert("user-agent", "curl/8");
        req.headers.insert("connection", "keep-alive");
        req.headers.insert("content-length", "0");
    });
    println!("six headers: {n} allocations");
    assert!(
        n <= BUDGET_SIX_HEADERS,
        "six headers cost {n} allocations, budget is {BUDGET_SIX_HEADERS}: \
         interning a well-known name must not allocate"
    );
}

#[test]
fn cloning_a_request_does_not_copy_its_body_or_target() {
    let mut req = HttpRequest::new("POST", "/upload");
    req.set_body_bytes(bytes::Bytes::from(vec![0u8; 1024 * 1024]));
    // The first clone is not the measurement. `Bytes` built from a `Vec` starts
    // in a representation that owns the buffer outright; the first clone
    // promotes it to a shared one, which allocates the refcount cell. That
    // happens once per buffer, not once per clone, and both the target and the
    // body pay it here.
    let first = req.clone();
    assert_eq!(
        first.body.as_ptr(),
        req.body.as_ptr(),
        "the promotion must share the buffer, not copy it"
    );

    let n = count(|| req.clone());
    println!("clone: {n} allocations");
    // A megabyte body and a target, cloned for the price of two refcount bumps.
    assert_eq!(n, 0, "cloning a request cost {n} allocations");
}

/// Dispatch through the method-indexed trees, with one captured parameter.
///
/// A budget rather than zero: the handler future is boxed, and un-boxing it is
/// later work. What this pins down is that a match no longer builds a `HashMap`
/// and that the captured span is not copied into owned `String`s.
const BUDGET_DISPATCH: u64 = 4;

#[test]
fn dispatch_allocates_only_for_captured_params() {
    // A current-thread runtime driven from this thread, so the counter sees the
    // whole dispatch rather than a slice of it on some worker.
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");

    let mut router = Router::new();
    router.get("/users/:id", |_req: HttpRequest| async {
        Ok::<_, Error>(HttpResponse::new(200))
    });
    // Warm the lazily built index outside the count.
    rt.block_on(router.route(HttpRequest::new("GET", "/users/1")))
        .expect("warm-up dispatch");

    let n = count(|| {
        rt.block_on(router.route(HttpRequest::new("GET", "/users/42")))
            .expect("dispatch")
    });
    println!("dispatch: {n} allocations");
    assert!(
        n <= BUDGET_DISPATCH,
        "dispatch cost {n} allocations, budget is {BUDGET_DISPATCH}"
    );
}
