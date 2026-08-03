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

#![no_main]

use arbitrary::Arbitrary;
use armature_webhooks::WebhookSignature;
use libfuzzer_sys::fuzz_target;

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
            case.forged == genuine || is_genuine_timestamped(&signer, case.payload, case.forged),
            "an arbitrary signature was accepted: secret={:?} payload_len={} sig={:?}",
            case.secret,
            case.payload.len(),
            case.forged,
        );
    }

    // Completeness, timestampless scheme.
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
});

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
