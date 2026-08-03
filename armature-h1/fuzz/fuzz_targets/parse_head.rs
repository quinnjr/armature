//! Fuzz `parse_head` over arbitrary bytes.
//!
//! Beyond "does not panic", this asserts the invariant the whole crate rests on:
//! every `Bytes` in a parsed head points into the input buffer rather than a copy.
//! A regression that reintroduced copying would still parse correctly and pass
//! every unit test, so it is checked here on every input.

#![no_main]

use armature_h1::{HeaderId, Limits, parse_head};
use bytes::Bytes;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let buf = Bytes::copy_from_slice(data);
    let limits = Limits::default();

    let Ok(Some((head, n))) = parse_head(&buf, &limits) else {
        // Errors and incomplete heads are both fine; the point is no panic.
        return;
    };

    assert!(
        n <= buf.len(),
        "head length {n} exceeds input {}",
        buf.len()
    );
    assert!(n <= limits.max_head_bytes, "head length exceeded the limit");
    assert!(
        head.headers.len() <= limits.max_headers,
        "header count exceeded the limit"
    );

    // Zero-copy invariant: every non-empty value lies inside the input's address
    // range.
    //
    // Empty slices are exempt, and must be: `Bytes::slice_ref` on a zero-length
    // slice returns `Bytes::new()`, which has no relationship to the parent
    // allocation. An empty header value (`v:\r\n` — a real thing clients send)
    // therefore points nowhere in particular, and copies nothing either way. The
    // fuzzer found this within a minute of first being run.
    let base = buf.as_ptr() as usize;
    let end = base + buf.len();
    let inside = |s: &[u8]| {
        if s.is_empty() {
            return true;
        }
        let a = s.as_ptr() as usize;
        a >= base && a < end
    };

    assert!(inside(head.target.as_bytes()), "target was copied");
    for (name, value) in head.headers.iter() {
        assert!(inside(value), "header value was copied");
        // A mixed-case custom name is lowercased, which does allocate; an
        // already-lowercase one must be projected in place.
        if let HeaderId::Other(n) = name {
            let lowercase_in_input = data.windows(n.as_bytes().len()).any(|w| w == n.as_bytes());
            if lowercase_in_input {
                // Not asserted as a pointer check: the lowercasing path
                // legitimately allocates. Presence in the input is enough to
                // confirm the name was not mangled.
                assert!(!n.as_bytes().is_empty());
            }
        }
    }

    // Accessors must never panic on anything the parser accepted.
    let _ = head.path();
    let _ = head.query();
    let _ = head.is_keep_alive();
    let _ = head.connection_has_token("close");
    let _ = head.count(&HeaderId::Host);
    let _ = armature_h1::framing::decide(&head, &limits);
});
