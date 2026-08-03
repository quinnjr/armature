//! Differential fuzzing: our framing decision against hyper's.
//!
//! This is the highest-value target in the crate. Request smuggling *is* two HTTP
//! implementations disagreeing about where one message ends and the next begins.
//! So the property under test is not "we are correct in isolation" — it is "we and
//! hyper reach the same accept/reject verdict, and when we both accept, we agree
//! on the body length."
//!
//! A divergence found here is a smuggling vector by construction, and it is
//! exactly the class of defect that code review reliably misses: both
//! implementations look reasonable, and only the comparison reveals the gap.
//!
//! ## What this target asserts, and what it deliberately does not
//!
//! The fatal condition is narrow on purpose: **when both implementations accept a
//! request, they must agree on the body length.** That is the smuggling-relevant
//! property. A length disagreement means one hop believes the next message starts
//! where the other believes body bytes continue, which is the whole attack.
//!
//! Accept/reject asymmetries are *reported but not fatal*, because they are not
//! smuggling vectors on their own and chasing them is a rabbit hole:
//!
//! - **We reject, hyper accepts.** Safe by construction — nothing gets past a hop
//!   that closed the connection. This is expected and intentional: we reject bare
//!   LF, which RFC 9112 §2.2 permits.
//! - **We accept, hyper rejects.** Usually hyper being stricter about something
//!   orthogonal to framing — token validation in a method or field name, say. In a
//!   proxy chain the stricter hop rejects and nothing is smuggled either way.
//!
//! That said, both asymmetry directions have already paid for themselves. Chasing
//! them produced two real hardening changes to this crate: request-target
//! character validation, and request-target *form* validation per RFC 9112 §3.2.
//! They are logged rather than dropped so a future run can still mine them.

#![no_main]

use armature_h1::{BodyKind, Limits, parse_head};
use bytes::Bytes;
use libfuzzer_sys::fuzz_target;

/// The verdict an implementation reaches about one request.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Accepted, with this body length in bytes.
    Accepted(u64),
    /// Accepted with a chunked body of this decoded length.
    AcceptedChunked(u64),
    /// Rejected.
    Rejected,
    /// Not enough input to decide.
    Incomplete,
}

/// Our verdict, from the pure framing path.
fn ours(data: &[u8], limits: &Limits) -> Verdict {
    let buf = Bytes::copy_from_slice(data);
    let (head, head_len) = match parse_head(&buf, limits) {
        Ok(Some(pair)) => pair,
        Ok(None) => return Verdict::Incomplete,
        Err(_) => return Verdict::Rejected,
    };

    match armature_h1::framing::decide(&head, limits) {
        Err(_) => Verdict::Rejected,
        Ok(BodyKind::None) => Verdict::Accepted(0),
        Ok(BodyKind::Length(n)) => {
            let available = (data.len() - head_len) as u64;
            if available < n {
                Verdict::Incomplete
            } else {
                Verdict::Accepted(n)
            }
        }
        Ok(BodyKind::Chunked) => {
            let mut d = armature_h1::ChunkedDecoder::new(limits);
            let mut rest = buf.slice(head_len..);
            let mut total = 0u64;
            loop {
                match d.poll(&mut rest) {
                    Err(_) => return Verdict::Rejected,
                    Ok(None) => return Verdict::Incomplete,
                    Ok(Some(armature_h1::ChunkEvent::Data(b))) => total += b.len() as u64,
                    Ok(Some(armature_h1::ChunkEvent::Trailers(_))) => {}
                    Ok(Some(armature_h1::ChunkEvent::End)) => {
                        return Verdict::AcceptedChunked(total);
                    }
                }
            }
        }
    }
}

/// hyper's verdict, from driving its HTTP/1 server over an in-memory transport.
fn theirs(data: &[u8], limits: &Limits) -> Verdict {
    use http_body_util::BodyExt;
    use hyper::service::service_fn;
    use std::sync::{Arc, Mutex};

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    // What hyper decided, recorded from inside its service.
    let seen: Arc<Mutex<Option<Verdict>>> = Arc::new(Mutex::new(None));
    let chunked = Arc::new(Mutex::new(false));

    let outcome = rt.block_on(async {
        let (mut client, server) = tokio::io::duplex(1 << 20);
        let io = hyper_util::rt::TokioIo::new(server);

        let seen2 = seen.clone();
        let chunked2 = chunked.clone();
        let svc = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
            let seen = seen2.clone();
            let chunked = chunked2.clone();
            async move {
                let te_chunked = req
                    .headers()
                    .get(hyper::header::TRANSFER_ENCODING)
                    .is_some();
                *chunked.lock().expect("lock") = te_chunked;
                let verdict = match req.into_body().collect().await {
                    Ok(collected) => {
                        let n = collected.to_bytes().len() as u64;
                        if te_chunked {
                            Verdict::AcceptedChunked(n)
                        } else {
                            Verdict::Accepted(n)
                        }
                    }
                    // A body that fails mid-read is a rejection: hyper decided the
                    // framing was not what the head claimed.
                    Err(_) => Verdict::Rejected,
                };
                *seen.lock().expect("lock") = Some(verdict);
                Ok::<_, std::convert::Infallible>(hyper::Response::new(http_body_util::Full::new(
                    bytes::Bytes::new(),
                )))
            }
        });

        let mut builder = hyper::server::conn::http1::Builder::new();
        builder
            .max_headers(limits.max_headers)
            .header_read_timeout(None);

        let conn = builder.serve_connection(io, svc);

        let write = async {
            use tokio::io::AsyncWriteExt;
            let _ = client.write_all(data).await;
            let _ = client.flush().await;
            // Half-close so hyper sees the end of input rather than waiting.
            let _ = client.shutdown().await;
        };

        let (result, _) = tokio::join!(
            tokio::time::timeout(std::time::Duration::from_secs(2), conn),
            write
        );
        result
    });

    let recorded = seen.lock().expect("lock").take();
    match (recorded, outcome) {
        // The service ran: whatever it recorded is hyper's verdict.
        (Some(v), _) => v,
        // No service call and the connection ended in error: hyper rejected the
        // head before dispatch.
        (None, Ok(Err(_))) => Verdict::Rejected,
        // No service call, clean end: hyper never had a complete request.
        (None, Ok(Ok(()))) => Verdict::Incomplete,
        // Timed out waiting for more input.
        (None, Err(_)) => Verdict::Incomplete,
    }
}

fuzz_target!(|data: &[u8]| {
    // Bound the input: hyper drives a real runtime per case, so huge inputs buy
    // coverage at a poor rate.
    if data.is_empty() || data.len() > 2048 {
        return;
    }

    let limits = Limits::default();
    let mine = ours(data, &limits);
    let hyper_verdict = theirs(data, &limits);

    if mine == hyper_verdict {
        return;
    }

    // Incomplete boundaries differ harmlessly: hyper sees a half-closed socket,
    // while we are handed a finite slice.
    if mine == Verdict::Incomplete || hyper_verdict == Verdict::Incomplete {
        return;
    }

    let both_accepted = matches!(
        (&mine, &hyper_verdict),
        (Verdict::Accepted(_), Verdict::Accepted(_))
            | (Verdict::AcceptedChunked(_), Verdict::AcceptedChunked(_))
            | (Verdict::Accepted(_), Verdict::AcceptedChunked(_))
            | (Verdict::AcceptedChunked(_), Verdict::Accepted(_))
    );

    if both_accepted {
        // The fatal case. Both hops will forward this request, and they disagree
        // about where its body ends — so they disagree about where the next
        // request begins. That is request smuggling, by definition.
        panic!(
            "BODY LENGTH DIVERGENCE — ours: {mine:?}, hyper: {hyper_verdict:?}\n\
             Both implementations accepted this request and disagree about its body \
             length. This is a smuggling vector.\n\
             input: {:?}",
            String::from_utf8_lossy(data)
        );
    }

    // An accept/reject asymmetry. Not a smuggling vector on its own — see the
    // module docs — but worth surfacing, since mining these produced the
    // request-target character and form validation this crate now performs.
    eprintln!(
        "[asymmetry] ours: {mine:?}, hyper: {hyper_verdict:?} — input: {:?}",
        String::from_utf8_lossy(data)
    );
});
