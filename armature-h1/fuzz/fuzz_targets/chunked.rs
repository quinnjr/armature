//! Fuzz the chunked decoder, including split-invariance.
//!
//! The interesting property is not "does not panic" — it is that the *semantics*
//! are invariant under how the input is split across reads. A state-machine bug
//! that mishandles a chunk straddling two reads passes any single-shot fuzz run
//! and fails here.
//!
//! Note what is compared and what is not. `Data` events are explicitly allowed to
//! carry partial chunks, so feeding one byte at a time legitimately produces more,
//! smaller events than feeding everything at once. Comparing event *sequences*
//! therefore fails on correct behavior — the fuzzer demonstrated this within
//! seconds. What must match is the concatenated body, the trailers, and the
//! terminal outcome.

#![no_main]

use armature_h1::{ChunkEvent, ChunkedDecoder, Limits};
use bytes::Bytes;
use libfuzzer_sys::fuzz_target;

/// The semantics of one decode run, independent of event granularity.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    /// Every body byte, concatenated.
    body: Vec<u8>,
    /// Trailer fields as `name=value` pairs, in order.
    trailers: Vec<(String, Vec<u8>)>,
    /// How the run ended.
    end: End,
}

#[derive(Debug, PartialEq, Eq)]
enum End {
    /// Reached the terminating chunk.
    Complete,
    /// Ran out of input before finishing.
    Incomplete,
    /// Rejected.
    Error,
}

/// Decode `data`, feeding it in slices of `step` bytes.
fn decode(data: &[u8], step: usize, limits: &Limits) -> Outcome {
    let mut d = ChunkedDecoder::new(limits);
    let mut buf = Bytes::new();
    let mut body = Vec::new();
    let mut trailers = Vec::new();
    let mut fed = 0;

    loop {
        loop {
            match d.poll(&mut buf) {
                Err(_) => {
                    return Outcome {
                        body,
                        trailers,
                        end: End::Error,
                    };
                }
                Ok(None) => break,
                Ok(Some(ChunkEvent::Data(b))) => {
                    assert!(
                        d.decoded_len() <= limits.max_body_bytes,
                        "decoded {} exceeds the limit {}",
                        d.decoded_len(),
                        limits.max_body_bytes
                    );
                    body.extend_from_slice(&b);
                }
                Ok(Some(ChunkEvent::Trailers(t))) => {
                    for (id, v) in t.iter() {
                        trailers.push((id.as_str().to_owned(), v.to_vec()));
                    }
                }
                Ok(Some(ChunkEvent::End)) => {
                    return Outcome {
                        body,
                        trailers,
                        end: End::Complete,
                    };
                }
            }
        }

        if fed >= data.len() {
            return Outcome {
                body,
                trailers,
                end: End::Incomplete,
            };
        }
        let take = step.min(data.len() - fed);
        let mut next = Vec::with_capacity(buf.len() + take);
        next.extend_from_slice(&buf);
        next.extend_from_slice(&data[fed..fed + take]);
        buf = Bytes::from(next);
        fed += take;
    }
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() || data.len() > 4096 {
        return;
    }

    let limits = Limits {
        // Small enough that the running body limit is actually exercised rather
        // than being unreachable on inputs this size.
        max_body_bytes: 1024,
        ..Default::default()
    };

    let whole = decode(data, data.len(), &limits);

    // Split-invariance: the semantics must not depend on how the bytes arrived.
    for step in [1usize, 2, 3, 7, 13] {
        let split = decode(data, step, &limits);
        assert_eq!(
            whole, split,
            "semantics differed with step {step}; the state machine mishandles a \
             read boundary. input: {data:?}"
        );
    }
});
