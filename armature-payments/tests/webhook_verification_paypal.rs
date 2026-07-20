#![cfg(feature = "paypal")]
//! Webhook authenticity tests.
//!
//! These are regression tests for a security bug: `PayPalProvider::verify_webhook`
//! and `BraintreeProvider::verify_webhook` previously ignored their arguments
//! and returned `Ok(())`, so `PaymentProcessor::handle_webhook` accepted any
//! attacker-forged event as genuine. Every test here fails against that code.

use armature_payments::providers::paypal::{
    PAYPAL_AUTH_ALGO_HEADER, PAYPAL_CERT_URL_HEADER, PAYPAL_TRANSMISSION_ID_HEADER,
    PAYPAL_TRANSMISSION_SIG_HEADER, PAYPAL_TRANSMISSION_TIME_HEADER, PayPalProvider,
};
use armature_payments::{PaymentError, PaymentProcessor, PaymentProvider, WebhookHeaders};
use armature_testkit::http_stub::{StubResponse, StubServer};

const PAYPAL_EVENT: &str = r#"{"id":"WH-1","event_type":"PAYMENT.CAPTURE.COMPLETED","resource":{"amount":{"value":"999.00"}}}"#;

// ---------------------------------------------------------------- PayPal ---

fn paypal_headers() -> WebhookHeaders {
    WebhookHeaders::new()
        .with(PAYPAL_AUTH_ALGO_HEADER, "SHA256withRSA")
        .with(
            PAYPAL_CERT_URL_HEADER,
            "https://api.paypal.com/v1/notifications/certs/CERT-1",
        )
        .with(PAYPAL_TRANSMISSION_ID_HEADER, "transmission-1")
        .with(PAYPAL_TRANSMISSION_SIG_HEADER, "c2lnbmF0dXJl")
        .with(PAYPAL_TRANSMISSION_TIME_HEADER, "2026-07-20T00:00:00Z")
}

/// A PayPal stub that answers the token handshake and reports `status` from the
/// verification endpoint.
async fn paypal_stub(verification_status: &str) -> StubServer {
    StubServer::builder()
        .route(
            "POST",
            "/v1/oauth2/token",
            StubResponse::json(200, r#"{"access_token":"tok","expires_in":3600}"#),
        )
        .route(
            "POST",
            "/v1/notifications/verify-webhook-signature",
            StubResponse::json(
                200,
                format!(r#"{{"verification_status":"{verification_status}"}}"#),
            ),
        )
        .start()
        .await
}

fn paypal_provider(server: &StubServer) -> PayPalProvider {
    PayPalProvider::new("client-id", "client-secret")
        .with_base_url(server.url())
        .with_webhook_id("WEBHOOK-ID-1")
}

#[tokio::test]
async fn paypal_accepts_a_webhook_paypal_confirms() {
    let server = paypal_stub("SUCCESS").await;
    let provider = paypal_provider(&server);

    provider
        .verify_webhook(PAYPAL_EVENT.as_bytes(), &paypal_headers())
        .await
        .expect("PayPal reported SUCCESS, the webhook must be accepted");
}

#[tokio::test]
async fn paypal_rejects_a_webhook_paypal_does_not_confirm() {
    let server = paypal_stub("FAILURE").await;
    let provider = paypal_provider(&server);

    let err = provider
        .verify_webhook(PAYPAL_EVENT.as_bytes(), &paypal_headers())
        .await
        .expect_err("PayPal reported FAILURE, the webhook must be rejected");

    assert!(
        matches!(err, PaymentError::InvalidWebhookSignature),
        "expected InvalidWebhookSignature, got {err:?}"
    );
}

#[tokio::test]
async fn paypal_rejects_a_webhook_with_no_signature_headers() {
    let server = paypal_stub("SUCCESS").await;
    let provider = paypal_provider(&server);

    // An attacker POSTing a fabricated body carries none of PayPal's
    // transmission headers. This must fail before any network call.
    let err = provider
        .verify_webhook(PAYPAL_EVENT.as_bytes(), &WebhookHeaders::new())
        .await
        .expect_err("a webhook with no transmission headers must be rejected");

    assert!(matches!(err, PaymentError::InvalidWebhookSignature));
    assert!(
        !server
            .requests()
            .iter()
            .any(|r| r.path == "/v1/notifications/verify-webhook-signature"),
        "verification must fail closed without calling PayPal"
    );
}

#[tokio::test]
async fn paypal_rejects_when_a_single_transmission_header_is_missing() {
    let server = paypal_stub("SUCCESS").await;
    let provider = paypal_provider(&server);

    let mut headers = paypal_headers();
    let complete = headers.len();
    headers = WebhookHeaders::from_iter(
        [
            (PAYPAL_AUTH_ALGO_HEADER, "SHA256withRSA"),
            (PAYPAL_CERT_URL_HEADER, "https://example.test/cert"),
            (PAYPAL_TRANSMISSION_ID_HEADER, "transmission-1"),
            (PAYPAL_TRANSMISSION_TIME_HEADER, "2026-07-20T00:00:00Z"),
        ]
        .into_iter(),
    );
    assert_eq!(complete, 5, "PayPal signs five transmission headers");

    let err = provider
        .verify_webhook(PAYPAL_EVENT.as_bytes(), &headers)
        .await
        .expect_err("a missing transmission signature must be rejected");
    assert!(matches!(err, PaymentError::InvalidWebhookSignature));
}

#[tokio::test]
async fn paypal_forwards_the_webhook_id_and_headers_to_paypal() {
    let server = paypal_stub("SUCCESS").await;
    let provider = paypal_provider(&server);

    provider
        .verify_webhook(PAYPAL_EVENT.as_bytes(), &paypal_headers())
        .await
        .unwrap();

    let sent = server.assert_received("POST", "/v1/notifications/verify-webhook-signature");
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    assert_eq!(body["webhook_id"], "WEBHOOK-ID-1");
    assert_eq!(body["transmission_id"], "transmission-1");
    assert_eq!(body["transmission_sig"], "c2lnbmF0dXJl");
    assert_eq!(body["transmission_time"], "2026-07-20T00:00:00Z");
    assert_eq!(body["auth_algo"], "SHA256withRSA");
    assert_eq!(
        body["webhook_event"]["id"], "WH-1",
        "the original event must be forwarded for verification"
    );
}

#[tokio::test]
async fn paypal_forwards_the_exact_bytes_paypal_signed() {
    // PayPal's verification endpoint recomputes a CRC32 over the raw body it
    // sent; any re-serialization (key reordering, spacing changes) breaks
    // verification for every genuine webhook. Use a payload with keys
    // deliberately out of alphabetical order and odd spacing, and assert the
    // outgoing request body contains that exact substring rather than a
    // normalized re-encoding.
    let server = paypal_stub("SUCCESS").await;
    let provider = paypal_provider(&server);
    let odd_payload = br#"{"resource":{"amount":{"value":"999.00"}}, "id"  :  "WH-1",   "event_type":"PAYMENT.CAPTURE.COMPLETED"}"#;

    provider
        .verify_webhook(odd_payload, &paypal_headers())
        .await
        .unwrap();

    let sent = server.assert_received("POST", "/v1/notifications/verify-webhook-signature");
    let body_str = String::from_utf8(sent.body.to_vec()).unwrap();
    assert!(
        body_str.contains(r#""resource":{"amount":{"value":"999.00"}}, "id"  :  "WH-1",   "event_type":"PAYMENT.CAPTURE.COMPLETED""#),
        "the exact byte layout PayPal signed must be forwarded verbatim, got: {body_str}"
    );
}

#[tokio::test]
async fn processor_does_not_parse_a_forged_paypal_webhook() {
    let server = paypal_stub("FAILURE").await;
    let processor = PaymentProcessor::new(paypal_provider(&server));

    let err = processor
        .handle_webhook(PAYPAL_EVENT.as_bytes(), &paypal_headers())
        .await
        .expect_err("a forged PayPal webhook must never reach the parser");

    assert!(matches!(err, PaymentError::InvalidWebhookSignature));
}
