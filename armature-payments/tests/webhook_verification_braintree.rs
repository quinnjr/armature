#![cfg(feature = "braintree")]
//! Webhook authenticity tests.
//!
//! These are regression tests for a security bug: `PayPalProvider::verify_webhook`
//! and `BraintreeProvider::verify_webhook` previously ignored their arguments
//! and returned `Ok(())`, so `PaymentProcessor::handle_webhook` accepted any
//! attacker-forged event as genuine. Every test here fails against that code.

use armature_payments::providers::braintree::{BRAINTREE_SIGNATURE_HEADER, BraintreeProvider};
use armature_payments::{PaymentError, PaymentProcessor, PaymentProvider, WebhookHeaders};
use hmac::{Hmac, KeyInit, Mac};
use sha1::{Digest, Sha1};

const BRAINTREE_EVENT: &str =
    r#"{"kind":"transaction_settled","subject":{"transaction":{"id":"tx_1"}}}"#;

// ------------------------------------------------------------- Braintree ---

const BT_PUBLIC_KEY: &str = "pub_key_1";
const BT_PRIVATE_KEY: &str = "priv_key_1";

/// Braintree's documented scheme: HMAC-SHA1 of the payload, keyed by the
/// SHA-1 digest of the private key.
fn braintree_sign(private_key: &str, payload: &[u8]) -> String {
    let key = Sha1::digest(private_key.as_bytes());
    let mut mac = Hmac::<Sha1>::new_from_slice(&key).unwrap();
    mac.update(payload);
    hex::encode(mac.finalize().into_bytes())
}

fn braintree_provider() -> BraintreeProvider {
    BraintreeProvider::new("merchant-1", BT_PUBLIC_KEY, BT_PRIVATE_KEY).expect("HTTP client builds")
}

#[tokio::test]
async fn braintree_accepts_a_correctly_signed_webhook() {
    let provider = braintree_provider();
    let signature = format!(
        "{BT_PUBLIC_KEY}|{}",
        braintree_sign(BT_PRIVATE_KEY, BRAINTREE_EVENT.as_bytes())
    );

    provider
        .verify_webhook(
            BRAINTREE_EVENT.as_bytes(),
            &WebhookHeaders::single(BRAINTREE_SIGNATURE_HEADER, signature),
        )
        .await
        .expect("a signature produced with the merchant's private key must verify");
}

#[tokio::test]
async fn braintree_selects_the_pair_matching_its_public_key() {
    let provider = braintree_provider();
    // Braintree sends one pair per key on the account; ours is not first.
    let signature = format!(
        "other_key|{},{BT_PUBLIC_KEY}|{}",
        braintree_sign("some-other-private-key", BRAINTREE_EVENT.as_bytes()),
        braintree_sign(BT_PRIVATE_KEY, BRAINTREE_EVENT.as_bytes())
    );

    provider
        .verify_webhook(
            BRAINTREE_EVENT.as_bytes(),
            &WebhookHeaders::single(BRAINTREE_SIGNATURE_HEADER, signature),
        )
        .await
        .expect("the pair matching our public key must be selected and verified");
}

#[tokio::test]
async fn braintree_rejects_a_tampered_payload() {
    let provider = braintree_provider();
    // Signature is valid for the original event; the attacker edits the body.
    let signature = format!(
        "{BT_PUBLIC_KEY}|{}",
        braintree_sign(BT_PRIVATE_KEY, BRAINTREE_EVENT.as_bytes())
    );
    let tampered = BRAINTREE_EVENT.replace("tx_1", "tx_attacker");

    let err = provider
        .verify_webhook(
            tampered.as_bytes(),
            &WebhookHeaders::single(BRAINTREE_SIGNATURE_HEADER, signature),
        )
        .await
        .expect_err("a payload edited after signing must be rejected");
    assert!(matches!(err, PaymentError::InvalidWebhookSignature));
}

#[tokio::test]
async fn braintree_rejects_a_signature_from_the_wrong_private_key() {
    let provider = braintree_provider();
    let signature = format!(
        "{BT_PUBLIC_KEY}|{}",
        braintree_sign("attacker-guessed-key", BRAINTREE_EVENT.as_bytes())
    );

    let err = provider
        .verify_webhook(
            BRAINTREE_EVENT.as_bytes(),
            &WebhookHeaders::single(BRAINTREE_SIGNATURE_HEADER, signature),
        )
        .await
        .expect_err("a signature from an unknown key must be rejected");
    assert!(matches!(err, PaymentError::InvalidWebhookSignature));
}

#[tokio::test]
async fn braintree_rejects_when_no_pair_matches_our_public_key() {
    let provider = braintree_provider();
    let signature = format!(
        "someone_elses_key|{}",
        braintree_sign(BT_PRIVATE_KEY, BRAINTREE_EVENT.as_bytes())
    );

    let err = provider
        .verify_webhook(
            BRAINTREE_EVENT.as_bytes(),
            &WebhookHeaders::single(BRAINTREE_SIGNATURE_HEADER, signature),
        )
        .await
        .expect_err("no pair for our public key means the webhook is unverifiable");
    assert!(matches!(err, PaymentError::InvalidWebhookSignature));
}

#[tokio::test]
async fn braintree_rejects_absent_and_malformed_signatures() {
    let provider = braintree_provider();

    for headers in [
        WebhookHeaders::new(),
        WebhookHeaders::single(BRAINTREE_SIGNATURE_HEADER, ""),
        WebhookHeaders::single(BRAINTREE_SIGNATURE_HEADER, "no-pipe-separator"),
        WebhookHeaders::single(BRAINTREE_SIGNATURE_HEADER, format!("{BT_PUBLIC_KEY}|zzzz")),
    ] {
        let err = provider
            .verify_webhook(BRAINTREE_EVENT.as_bytes(), &headers)
            .await
            .expect_err("absent or malformed signatures must fail closed");
        assert!(matches!(err, PaymentError::InvalidWebhookSignature));
    }
}

#[tokio::test]
async fn braintree_rejects_empty_credentials_instead_of_a_forged_signature() {
    // With an empty public_key, `bt_signature: "|<hmac>"` (empty key half)
    // would otherwise match; and an empty private_key makes the HMAC key
    // SHA-1("") — a public constant. Either lets any forged signature verify
    // under a missing-secret misconfiguration. Both must be rejected as a
    // config error, before the (attacker-satisfiable) signature is even
    // checked.
    let provider = BraintreeProvider::new("merchant-1", "", "").expect("HTTP client builds");
    let forged_signature = format!("|{}", braintree_sign("", BRAINTREE_EVENT.as_bytes()));

    let err = provider
        .verify_webhook(
            BRAINTREE_EVENT.as_bytes(),
            &WebhookHeaders::single(BRAINTREE_SIGNATURE_HEADER, forged_signature),
        )
        .await
        .expect_err("empty Braintree credentials must be rejected as a config error");
    assert!(
        matches!(err, PaymentError::Config(_)),
        "expected Config, got {err:?}"
    );
}

#[tokio::test]
async fn processor_does_not_parse_a_forged_braintree_webhook() {
    let processor = PaymentProcessor::new(braintree_provider());

    // The attacker fabricates a settled-transaction event with no signature.
    let err = processor
        .handle_webhook(BRAINTREE_EVENT.as_bytes(), &WebhookHeaders::new())
        .await
        .expect_err("a forged Braintree webhook must never reach the parser");

    assert!(matches!(err, PaymentError::InvalidWebhookSignature));
}
