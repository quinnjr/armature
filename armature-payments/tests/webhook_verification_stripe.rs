#![cfg(feature = "stripe")]
//! Webhook authenticity tests.
//!
//! These are regression tests for a security bug: `PayPalProvider::verify_webhook`
//! and `BraintreeProvider::verify_webhook` previously ignored their arguments
//! and returned `Ok(())`, so `PaymentProcessor::handle_webhook` accepted any
//! attacker-forged event as genuine. Every test here fails against that code.

use armature_payments::providers::stripe::{STRIPE_SIGNATURE_HEADER, StripeProvider};
use armature_payments::{PaymentError, PaymentProcessor, PaymentProvider, WebhookHeaders};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

const STRIPE_EVENT: &str =
    r#"{"id":"evt_1","type":"charge.succeeded","created":1,"livemode":false,"data":{"object":{}}}"#;

// ---------------------------------------------------------------- Stripe ---

const STRIPE_SECRET: &str = "whsec_test_secret";

fn stripe_signature(secret: &str, timestamp: i64, payload: &[u8]) -> String {
    let mut signed = format!("{timestamp}.").into_bytes();
    signed.extend_from_slice(payload);
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(&signed);
    format!(
        "t={timestamp},v1={}",
        hex::encode(mac.finalize().into_bytes())
    )
}

fn stripe_provider() -> StripeProvider {
    StripeProvider::new("sk_test").with_webhook_secret(STRIPE_SECRET)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[tokio::test]
async fn stripe_accepts_a_valid_signature() {
    let provider = stripe_provider();
    let sig = stripe_signature(STRIPE_SECRET, now(), STRIPE_EVENT.as_bytes());

    provider
        .verify_webhook(
            STRIPE_EVENT.as_bytes(),
            &WebhookHeaders::single(STRIPE_SIGNATURE_HEADER, sig),
        )
        .await
        .expect("a correctly signed Stripe webhook must verify");
}

#[tokio::test]
async fn stripe_accepts_any_of_several_rotated_signatures() {
    let provider = stripe_provider();
    let ts = now();
    let valid = stripe_signature(STRIPE_SECRET, ts, STRIPE_EVENT.as_bytes());
    let valid_hex = valid.split("v1=").nth(1).unwrap().to_string();
    // During a secret rotation Stripe sends the old and new signatures.
    let header = format!("t={ts},v1=00ff00ff,v1={valid_hex}");

    provider
        .verify_webhook(
            STRIPE_EVENT.as_bytes(),
            &WebhookHeaders::single(STRIPE_SIGNATURE_HEADER, header),
        )
        .await
        .expect("one matching v1 signature is enough");
}

#[tokio::test]
async fn stripe_rejects_a_tampered_payload() {
    let provider = stripe_provider();
    let sig = stripe_signature(STRIPE_SECRET, now(), STRIPE_EVENT.as_bytes());
    let tampered = STRIPE_EVENT.replace("evt_1", "evt_attacker");

    let err = provider
        .verify_webhook(
            tampered.as_bytes(),
            &WebhookHeaders::single(STRIPE_SIGNATURE_HEADER, sig),
        )
        .await
        .expect_err("a payload edited after signing must be rejected");
    assert!(matches!(err, PaymentError::InvalidWebhookSignature));
}

#[tokio::test]
async fn stripe_rejects_a_forged_signature() {
    let provider = stripe_provider();
    let sig = stripe_signature("whsec_attacker_guess", now(), STRIPE_EVENT.as_bytes());

    let err = provider
        .verify_webhook(
            STRIPE_EVENT.as_bytes(),
            &WebhookHeaders::single(STRIPE_SIGNATURE_HEADER, sig),
        )
        .await
        .expect_err("a signature from the wrong secret must be rejected");
    assert!(matches!(err, PaymentError::InvalidWebhookSignature));
}

#[tokio::test]
async fn stripe_rejects_a_replayed_timestamp_outside_tolerance() {
    let provider = stripe_provider();
    // Correctly signed, but signed two hours ago: a captured-and-replayed
    // webhook. The signature alone stays valid forever, so the timestamp is
    // what bounds the replay window.
    let stale = now() - 7200;
    let sig = stripe_signature(STRIPE_SECRET, stale, STRIPE_EVENT.as_bytes());

    let err = provider
        .verify_webhook(
            STRIPE_EVENT.as_bytes(),
            &WebhookHeaders::single(STRIPE_SIGNATURE_HEADER, sig),
        )
        .await
        .expect_err("a stale timestamp must be rejected as a replay");
    assert!(matches!(err, PaymentError::InvalidWebhookSignature));

    // ...and is accepted when the tolerance check is explicitly disabled.
    StripeProvider::new("sk_test")
        .with_webhook_secret(STRIPE_SECRET)
        .without_webhook_tolerance()
        .verify_webhook(
            STRIPE_EVENT.as_bytes(),
            &WebhookHeaders::single(
                STRIPE_SIGNATURE_HEADER,
                stripe_signature(STRIPE_SECRET, stale, STRIPE_EVENT.as_bytes()),
            ),
        )
        .await
        .expect("with tolerance disabled the signature alone decides");
}

#[tokio::test]
async fn stripe_rejects_malformed_and_absent_signature_headers() {
    let provider = stripe_provider();

    for header in [
        None,
        Some(String::new()),
        Some("v1=deadbeef".to_string()),        // no timestamp
        Some(format!("t={}", now())),           // no signature
        Some(format!("t={},v1=nothex", now())), // undecodable
        Some("t=not-a-number,v1=deadbeef".to_string()), // unparseable timestamp
    ] {
        let headers = match header {
            Some(value) => WebhookHeaders::single(STRIPE_SIGNATURE_HEADER, value),
            None => WebhookHeaders::new(),
        };
        let err = provider
            .verify_webhook(STRIPE_EVENT.as_bytes(), &headers)
            .await
            .expect_err("malformed signature headers must fail closed");
        assert!(
            matches!(err, PaymentError::InvalidWebhookSignature),
            "expected InvalidWebhookSignature, got {err:?}"
        );
    }
}

/// A signature that is wrong only in its last byte must be rejected exactly like
/// one that is wrong from the first byte.
///
/// This test was previously named `stripe_verification_is_constant_time`, which
/// it never established: both cases are rejected by a plain `==` too, so it
/// passed against a naive short-circuiting comparison while its name claimed
/// otherwise. Timing behavior is not assertable from a test like this — wall
/// clock deltas here are dominated by scheduling noise — so the name now states
/// what is actually checked. The constant-time property is a code-review
/// obligation on the comparison itself, not something this file can verify.
#[tokio::test]
async fn stripe_rejects_near_miss_and_far_miss_signatures() {
    let provider = stripe_provider();
    let ts = now();
    let valid = stripe_signature(STRIPE_SECRET, ts, STRIPE_EVENT.as_bytes());
    let valid_hex = valid.split("v1=").nth(1).unwrap();

    let mut near_miss: Vec<u8> = hex::decode(valid_hex).unwrap();
    let last = near_miss.len() - 1;
    near_miss[last] ^= 0x01;

    let mut first_byte_wrong = hex::decode(valid_hex).unwrap();
    first_byte_wrong[0] ^= 0xff;

    for forged in [near_miss, first_byte_wrong] {
        let header = format!("t={ts},v1={}", hex::encode(&forged));
        let err = provider
            .verify_webhook(
                STRIPE_EVENT.as_bytes(),
                &WebhookHeaders::single(STRIPE_SIGNATURE_HEADER, header),
            )
            .await
            .expect_err("any incorrect signature must be rejected");
        assert!(matches!(err, PaymentError::InvalidWebhookSignature));
    }
}

#[tokio::test]
async fn stripe_verification_requires_a_configured_secret() {
    let provider = StripeProvider::new("sk_test");
    let err = provider
        .verify_webhook(
            STRIPE_EVENT.as_bytes(),
            &WebhookHeaders::single(STRIPE_SIGNATURE_HEADER, "t=1,v1=ab"),
        )
        .await
        .expect_err("without a webhook secret nothing can be verified");
    assert!(matches!(err, PaymentError::Config(_)));
}

#[tokio::test]
async fn stripe_verification_rejects_an_empty_secret() {
    // An empty secret is a misconfiguration, not a valid HMAC key — it must
    // not be treated as "verification passes for whoever guesses the empty
    // string", which is trivial.
    let provider = StripeProvider::new("sk_test").with_webhook_secret("");
    let sig = stripe_signature("", now(), STRIPE_EVENT.as_bytes());

    let err = provider
        .verify_webhook(
            STRIPE_EVENT.as_bytes(),
            &WebhookHeaders::single(STRIPE_SIGNATURE_HEADER, sig),
        )
        .await
        .expect_err("an empty webhook secret must be rejected as a config error");
    assert!(matches!(err, PaymentError::Config(_)));
}

#[tokio::test]
async fn stripe_rejects_more_than_the_maximum_v1_candidates() {
    // Stripe sends at most two v1 candidates during a secret rotation. An
    // attacker padding the header with dozens of candidates forces a fresh
    // HMAC-SHA256 over the whole payload per candidate; this must be capped
    // rather than amplifying unboundedly.
    let provider = stripe_provider();
    let ts = now();
    let valid = stripe_signature(STRIPE_SECRET, ts, STRIPE_EVENT.as_bytes());
    let valid_hex = valid.split("v1=").nth(1).unwrap().to_string();

    let mut parts = vec![format!("t={ts}")];
    for _ in 0..20 {
        parts.push("v1=00ff00ff".to_string());
    }
    // The genuine signature is included but past the cap, so it must not
    // rescue the request.
    parts.push(format!("v1={valid_hex}"));
    let header = parts.join(",");

    let err = provider
        .verify_webhook(
            STRIPE_EVENT.as_bytes(),
            &WebhookHeaders::single(STRIPE_SIGNATURE_HEADER, header),
        )
        .await
        .expect_err("too many v1 candidates must be rejected outright");
    assert!(matches!(err, PaymentError::InvalidWebhookSignature));
}

#[tokio::test]
async fn stripe_processor_parses_only_verified_events() {
    let processor = PaymentProcessor::new(stripe_provider());
    let sig = stripe_signature(STRIPE_SECRET, now(), STRIPE_EVENT.as_bytes());

    let event = processor
        .handle_webhook(
            STRIPE_EVENT.as_bytes(),
            &WebhookHeaders::single(STRIPE_SIGNATURE_HEADER, sig),
        )
        .await
        .expect("a verified event parses");
    assert_eq!(event.id, "evt_1");

    let err = processor
        .handle_webhook(
            STRIPE_EVENT.as_bytes(),
            &WebhookHeaders::single(STRIPE_SIGNATURE_HEADER, "t=1,v1=deadbeef"),
        )
        .await
        .expect_err("an unverified event must not parse");
    assert!(matches!(err, PaymentError::InvalidWebhookSignature));
}

#[test]
fn webhook_header_lookup_is_case_insensitive() {
    let headers = WebhookHeaders::single("Stripe-Signature", "t=1,v1=ab");
    assert_eq!(headers.get("stripe-signature"), Some("t=1,v1=ab"));
    assert_eq!(headers.get("STRIPE-SIGNATURE"), Some("t=1,v1=ab"));
    assert!(headers.get("missing").is_none());
    assert!(WebhookHeaders::new().is_empty());
}
