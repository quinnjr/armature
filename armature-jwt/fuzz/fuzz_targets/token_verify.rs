//! Fuzz JWT verification.
//!
//! A bearer token is supplied by the caller, so `verify` is an authentication
//! boundary and the interesting property is the same one webhook signatures
//! have: a token the fuzzer made up must never verify. A false accept here is
//! an authentication bypass, and JWT has a well-known family of ways to get
//! one — `alg: none`, algorithm confusion, an unsigned third segment, a
//! signature lifted from a token issued under a different key.
//!
//! `decode_unverified` is exercised alongside it, because it is the function
//! whose *name* is the safety property: it must never be reachable from
//! `verify`, and it must not panic on arbitrary input either, since callers
//! reach for it to inspect claims before deciding what to do with a token.

#![no_main]

use arbitrary::Arbitrary;
use armature_jwt::{JwtConfig, JwtService};
use libfuzzer_sys::fuzz_target;
use serde::{Deserialize, Serialize};

/// Claims small enough that the fuzzer can plausibly produce a valid body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct TestClaims {
    sub: String,
    exp: usize,
}

#[derive(Debug, Arbitrary)]
struct Case<'a> {
    secret: &'a str,
    /// A token chosen by the fuzzer — i.e. by an attacker.
    forged: &'a str,
    subject: &'a str,
}

fuzz_target!(|case: Case<'_>| {
    if case.secret.is_empty() || case.forged.len() > 4096 || case.subject.len() > 256 {
        return;
    }

    let Ok(service) = JwtService::new(JwtConfig::new(case.secret.to_string())) else {
        return;
    };

    // `decode_unverified` reads the payload without checking the signature, so
    // it is expected to accept things `verify` rejects. It must still not
    // panic: callers use it to decide how to handle a token.
    let _ = service.decode_unverified::<TestClaims>(case.forged);

    // Soundness. An arbitrary string must not verify unless the fuzzer
    // reproduced a token this service would itself have issued.
    if let Ok(claims) = service.verify::<TestClaims>(case.forged) {
        let reissued = service.sign(&claims);
        assert_eq!(
            reissued.ok().as_deref(),
            Some(case.forged),
            "a token that this service did not issue verified: secret={:?} token={:?}",
            case.secret,
            case.forged,
        );
    }

    // Completeness. A token this service issued, with a comfortably future
    // expiry, must verify and round-trip its claims unchanged.
    let claims = TestClaims {
        sub: case.subject.to_owned(),
        // Far enough out that the clock cannot expire it mid-run, and small
        // enough not to overflow the `exp` handling.
        exp: 4_102_444_800, // 2100-01-01
    };
    let Ok(issued) = service.sign(&claims) else {
        return;
    };
    let verified = service
        .verify::<TestClaims>(&issued)
        .expect("a token this service issued must verify");
    assert_eq!(
        verified, claims,
        "claims did not survive a sign/verify round trip",
    );

    // Tampering with any segment must invalidate the token. Flipping a
    // character in the payload is the cheapest way to be sure the signature is
    // actually consulted rather than the token merely being well-formed.
    if let Some((head, last)) = issued.rsplit_once('.')
        && !last.is_empty()
    {
        let flipped = if last.starts_with('A') { 'B' } else { 'A' };
        let tampered = format!("{head}.{flipped}{}", &last[1..]);
        assert!(
            service.verify::<TestClaims>(&tampered).is_err(),
            "a token verified after its signature was altered: {tampered:?}",
        );
    }
});
