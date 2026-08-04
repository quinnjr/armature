//! Fuzz webhook signature verification.
//!
//! The signature header is supplied by whoever is posting to the endpoint, so
//! `verify` is the boundary between "a webhook we issued a secret for" and
//! anyone on the internet. Not panicking is the least of it; the properties
//! that matter are the two directions of the authentication decision:
//!
//! * **Soundness** — a signature the fuzzer made up must never verify. This is
//!   the one that matters: a false accept is an authentication bypass.
//! * **Completeness** — a signature this crate produced for this payload must
//!   verify. A false reject is not a security hole, but it is a webhook
//!   endpoint that rejects its own sender.
//!
//! Both are checked against the same secret, so any accept of an arbitrary
//! string is a genuine forgery rather than a key mismatch.
//!
//! `verify` accepts two schemes, and completeness is checked for both. The
//! Stripe-style `t=..,v1=..` header is what `sign` emits, so that direction is
//! covered by round-tripping the crate against itself. The GitHub-style
//! `sha256=<hex>` header has no signer in this crate at all, so the harness
//! constructs it here from `hmac`/`sha2` directly — an independent
//! re-implementation rather than a round trip, which is what makes it worth
//! asserting.

#![no_main]

use arbitrary::Arbitrary;
use armature_webhooks::WebhookSignature;
use hmac::{Hmac, KeyInit, Mac};
use libfuzzer_sys::fuzz_target;
use sha2::Sha256;

#[derive(Debug, Arbitrary)]
struct Case<'a> {
    secret: &'a str,
    payload: &'a [u8],
    /// A signature header chosen by the fuzzer — i.e. by an attacker.
    forged: &'a str,
}

fuzz_target!(|case: Case<'_>| {
    if case.secret.is_empty() || case.payload.len() > 4096 || case.forged.len() > 1024 {
        return;
    }

    let signer = WebhookSignature::new(case.secret);

    // Soundness. A tolerance large enough that the timestamped scheme cannot
    // reject on age instead of on the MAC — otherwise this would pass for the
    // wrong reason, never actually reaching the comparison.
    let forged_verdict = signer.verify(case.payload, case.forged, u64::MAX);
    if matches!(forged_verdict, Ok(true)) {
        // Accepting is only legitimate if the fuzzer happened to reproduce a
        // signature this crate would itself have issued for this payload.
        let genuine = signer.sign(case.payload);
        assert!(
            case.forged == genuine
                || is_genuine_timestamped(&signer, case.payload, case.forged)
                || case.forged == github_signature(case.secret, case.payload),
            "an arbitrary signature was accepted: secret={:?} payload_len={} sig={:?}",
            case.secret,
            case.payload.len(),
            case.forged,
        );
    }

    // Completeness, timestamped scheme. `sign` stamps the current time, so this
    // is the `t=..,v1=..` branch of `verify`.
    let issued = signer.sign(case.payload);
    assert_eq!(
        signer.verify(case.payload, &issued, u64::MAX).ok(),
        Some(true),
        "a signature this crate issued did not verify: secret={:?} sig={issued:?}",
        case.secret,
    );

    // A payload the signature was not issued over must not verify under it.
    // Extending the payload is enough to be a different message, and avoids
    // relying on the fuzzer to supply two distinct byte strings.
    let mut tampered = case.payload.to_vec();
    tampered.push(0xAA);
    assert_ne!(
        signer.verify(&tampered, &issued, u64::MAX).ok(),
        Some(true),
        "a signature verified against a payload it was not issued over",
    );

    // Completeness, GitHub scheme. Nothing in this crate emits a `sha256=`
    // header, so without this the accept side of that branch is never reached:
    // the fuzzer only ever feeds it garbage, which tests false-accepts only. A
    // silent regression there would strand every GitHub-style sender.
    let github = github_signature(case.secret, case.payload);
    assert_eq!(
        signer.verify(case.payload, &github, u64::MAX).ok(),
        Some(true),
        "a well-formed GitHub-style signature did not verify: secret={:?} sig={github:?}",
        case.secret,
    );

    // The mirror of the above: the same header one hex digit off must not
    // verify. Without this, a `verify` that accepted any `sha256=` header at all
    // would still satisfy the completeness assertion.
    let mut wrong = github.into_bytes();
    let last = wrong.len() - 1;
    wrong[last] = if wrong[last] == b'0' { b'1' } else { b'0' };
    let wrong = String::from_utf8(wrong).expect("hex digits are ASCII");
    assert_ne!(
        signer.verify(case.payload, &wrong, u64::MAX).ok(),
        Some(true),
        "a GitHub-style signature verified with an altered hex digit: {wrong:?}",
    );
});

/// The GitHub-style header for `payload` under `secret`: a raw HMAC-SHA256 over
/// the body, hex-encoded, with no timestamp mixed in.
///
/// Built from the primitives rather than from the crate, because the crate has
/// no signer for this scheme — which is the point: an agreement here is two
/// independent constructions matching, not `verify` agreeing with itself.
fn github_signature(secret: &str, payload: &[u8]) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(payload);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

/// Whether `candidate` is the timestamped signature this crate would issue for
/// `payload`, for the timestamp `candidate` itself carries.
///
/// The fuzzer can in principle land on a valid `t=..,v1=..` header, and that is
/// a correct accept rather than a forgery. Re-deriving it from the same secret
/// is the only way to tell the two apart.
fn is_genuine_timestamped(signer: &WebhookSignature, payload: &[u8], candidate: &str) -> bool {
    let Some(timestamp) = candidate
        .split(',')
        .find_map(|part| part.trim().strip_prefix("t="))
    else {
        return false;
    };
    signer.sign_with_timestamp(payload, timestamp) == candidate
}
