//! Fuzz JWT verification.
//!
//! A bearer token is supplied by the caller, so `verify` is an authentication
//! boundary and the interesting property is the same one webhook signatures
//! have: a token the fuzzer made up must never verify. A false accept here is
//! an authentication bypass.
//!
//! An unstructured string almost never reaches the interesting code — two
//! base64url-encoded JSON documents is far past what a fuzzer stumbles into, so
//! a free-form input mostly exercises the parser and stops there. The harness
//! therefore drives `verify` from three directions:
//!
//! * The fuzzer's free-form string, which is what finds parser panics.
//! * A token *assembled* from fuzzer-chosen parts — `b64(header_json)`,
//!   `b64(payload_json)`, `b64(sig)` — so the fuzzer is choosing header and
//!   claim fields rather than trying to invent base64. If one of these verifies,
//!   the header must have declared the configured algorithm and the third
//!   segment must be non-empty; anything else is an algorithm-pinning or
//!   unsigned-token bypass.
//! * Fixed forgeries spliced from a genuinely issued token, which is the only
//!   way to reach these at all: `alg: none` with the signature stripped,
//!   `alg: none` with the real signature still attached, a header re-declaring a
//!   different HMAC algorithm than the verifier is pinned to, a stripped third
//!   segment, and a signature lifted from a token issued under a different
//!   secret. Each must be rejected.
//!
//! `decode_unverified` is exercised alongside it. It reads claims without
//! checking the signature, so it is *expected* to accept what `verify` rejects;
//! what is asserted is that it does not panic on arbitrary input, and that it
//! agrees with `verify` whenever `verify` accepts — a token cannot verify as one
//! set of claims and inspect as another.

#![no_main]

use arbitrary::Arbitrary;
use armature_jwt::{JwtConfig, JwtService};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
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
    /// The three segments of a token the fuzzer assembles, pre-base64. Handing
    /// it the *decoded* parts is what puts `alg` and the claim set under its
    /// control instead of behind an encoding it would have to guess.
    header_json: &'a str,
    payload_json: &'a str,
    sig: &'a [u8],
}

fuzz_target!(|case: Case<'_>| {
    if case.secret.is_empty() || case.forged.len() > 4096 || case.subject.len() > 256 {
        return;
    }
    if case.header_json.len() > 1024 || case.payload_json.len() > 1024 || case.sig.len() > 512 {
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

    // The assembled token. Same soundness question, but now reachable: the
    // fuzzer picks the header and the claims directly.
    let assembled = format!(
        "{}.{}.{}",
        URL_SAFE_NO_PAD.encode(case.header_json),
        URL_SAFE_NO_PAD.encode(case.payload_json),
        URL_SAFE_NO_PAD.encode(case.sig),
    );
    let _ = service.decode_unverified::<TestClaims>(&assembled);
    if let Ok(claims) = service.verify::<TestClaims>(&assembled) {
        // Re-signing is not a usable oracle here: the fuzzer's header need not
        // be byte-identical to the one this crate emits even when it names the
        // same algorithm. What must hold regardless of formatting is that the
        // verifier only ever honoured its own configured algorithm, and that it
        // demanded a signature at all.
        let header: serde_json::Value =
            serde_json::from_str(case.header_json).expect("a verified token has a JSON header");
        assert_eq!(
            header.get("alg").and_then(|alg| alg.as_str()),
            Some("HS256"),
            "a token verified under an algorithm the service is not configured for: {:?}",
            case.header_json,
        );
        assert!(
            !case.sig.is_empty(),
            "a token with no signature verified: {:?}",
            case.payload_json,
        );
        assert_eq!(
            service.decode_unverified::<TestClaims>(&assembled).ok(),
            Some(claims),
            "verify and decode_unverified disagreed about the claims in {assembled:?}",
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

    // Tampering with any segment must invalidate the token. `rsplit_once` puts
    // the signature in `last`, so flipping a character there is the cheapest way
    // to be sure the signature is actually consulted rather than the token
    // merely being well-formed.
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

    // The other end of the token: `split_once` leaves the payload at the front
    // of `rest`, so this alters the claims rather than the signature. Either it
    // stops being valid base64/JSON or it stops matching the signature — both
    // are errors, which is why the assertion holds without inspecting which.
    if let Some((header_seg, rest)) = issued.split_once('.')
        && !rest.is_empty()
    {
        let flipped = if rest.starts_with('A') { 'B' } else { 'A' };
        let tampered = format!("{header_seg}.{flipped}{}", &rest[1..]);
        assert!(
            service.verify::<TestClaims>(&tampered).is_err(),
            "a token verified after its payload was altered: {tampered:?}",
        );
    }

    // The classic JWT forgeries. These are spliced onto a genuinely issued
    // token because that is the only way they are reachable — none of them are
    // something a fuzzer arrives at on its own, and all of them are what an
    // attacker holding one valid token would try first.
    let segments: Vec<&str> = issued.split('.').collect();
    let [header_seg, payload_seg, sig_seg] = segments.as_slice() else {
        return;
    };
    let alg_none = URL_SAFE_NO_PAD.encode(r#"{"alg":"none","typ":"JWT"}"#);
    let alg_hs512 = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS512","typ":"JWT"}"#);

    for (attack, token) in [
        // `alg: none` with the signature dropped: the unsigned-token bypass.
        ("alg:none, unsigned", format!("{alg_none}.{payload_seg}.")),
        // `alg: none` with the real signature left in place, for verifiers that
        // skip the check on `none` but still expect the segment to be present.
        (
            "alg:none, signature retained",
            format!("{alg_none}.{payload_seg}.{sig_seg}"),
        ),
        // A header naming a different HMAC algorithm than the verifier is
        // configured for: it must trust its own configuration, not the token.
        (
            "algorithm swapped in the header",
            format!("{alg_hs512}.{payload_seg}.{sig_seg}"),
        ),
        // A genuine header and payload with the signature stripped.
        (
            "signature stripped",
            format!("{header_seg}.{payload_seg}."),
        ),
    ] {
        assert!(
            service.verify::<TestClaims>(&token).is_err(),
            "{attack} was accepted: {token:?}",
        );
    }

    // A signature lifted from a token issued under a different secret. Appending
    // to the secret guarantees a different key without needing the fuzzer to
    // supply two distinct non-empty strings.
    let Ok(other) = JwtService::new(JwtConfig::new(format!("{}#", case.secret))) else {
        return;
    };
    let Ok(other_issued) = other.sign(&claims) else {
        return;
    };
    if let Some((_, other_sig)) = other_issued.rsplit_once('.') {
        let lifted = format!("{header_seg}.{payload_seg}.{other_sig}");
        assert!(
            service.verify::<TestClaims>(&lifted).is_err(),
            "a signature issued under a different secret was accepted: {lifted:?}",
        );
    }
});
