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
        .unwrap()
        .with_webhook_id("WEBHOOK-ID-1")
}

/// The five headers PayPal signs. Verification must require every one of them.
const PAYPAL_SIGNED_HEADERS: [&str; 5] = [
    PAYPAL_AUTH_ALGO_HEADER,
    PAYPAL_CERT_URL_HEADER,
    PAYPAL_TRANSMISSION_ID_HEADER,
    PAYPAL_TRANSMISSION_SIG_HEADER,
    PAYPAL_TRANSMISSION_TIME_HEADER,
];

const VERIFY_PATH: &str = "/v1/notifications/verify-webhook-signature";

/// Whether the verification endpoint was called at all.
fn called_paypal(server: &StubServer) -> bool {
    server.requests().iter().any(|r| r.path == VERIFY_PATH)
}

/// A PayPal stub whose token handshake succeeds but whose verification endpoint
/// answers with `resp`.
async fn paypal_stub_responding(resp: StubResponse) -> StubServer {
    StubServer::builder()
        .route(
            "POST",
            "/v1/oauth2/token",
            StubResponse::json(200, r#"{"access_token":"tok","expires_in":3600}"#),
        )
        .route("POST", VERIFY_PATH, resp)
        .start()
        .await
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

/// Dropping *any* one of the five signed headers must fail closed, before any
/// call to PayPal.
///
/// The previous version of this test removed only
/// `PAYPAL_TRANSMISSION_SIG_HEADER`, leaving the other four untested — a
/// verifier that required the signature and ignored, say, the transmission time
/// (which is what binds the signature to a moment and stops indefinite replay)
/// passed it. It also asserted `complete == 5` against a header map the test had
/// just built itself, which checks the fixture rather than the code.
#[tokio::test]
async fn paypal_rejects_when_any_single_signed_header_is_missing() {
    let complete = paypal_headers();
    assert_eq!(
        complete.len(),
        PAYPAL_SIGNED_HEADERS.len(),
        "the fixture must supply exactly the headers under test"
    );

    for omitted in PAYPAL_SIGNED_HEADERS {
        let server = paypal_stub("SUCCESS").await;
        let provider = paypal_provider(&server);

        let headers: WebhookHeaders = PAYPAL_SIGNED_HEADERS
            .iter()
            .filter(|h| **h != omitted)
            .map(|h| (*h, complete.get(h).expect("fixture header").to_string()))
            .collect();
        assert_eq!(headers.len(), PAYPAL_SIGNED_HEADERS.len() - 1);

        let err = provider
            .verify_webhook(PAYPAL_EVENT.as_bytes(), &headers)
            .await
            .expect_err(&format!(
                "a webhook missing {omitted} must be rejected, not verified"
            ));

        assert!(
            matches!(err, PaymentError::InvalidWebhookSignature),
            "omitting {omitted} must yield InvalidWebhookSignature, got {err:?}"
        );
        assert!(
            !called_paypal(&server),
            "an incomplete header set must fail closed locally; omitting \
             {omitted} still reached PayPal"
        );
    }
}

/// PayPal answering with an error is not PayPal answering "SUCCESS". Treating a
/// 5xx or a 401 as anything but a rejection lets an attacker who can disrupt the
/// verification call have forged webhooks accepted.
#[tokio::test]
async fn paypal_rejects_when_verification_endpoint_errors() {
    for status in [500u16, 401] {
        let server =
            paypal_stub_responding(StubResponse::json(status, r#"{"message":"nope"}"#)).await;

        let err = paypal_provider(&server)
            .verify_webhook(PAYPAL_EVENT.as_bytes(), &paypal_headers())
            .await
            .expect_err("an unanswered verification must not verify anything");

        assert!(
            matches!(err, PaymentError::InvalidWebhookSignature),
            "HTTP {status} from the verifier must reject the webhook, got {err:?}"
        );
    }
}

/// A 200 carrying a body that is not the expected verification envelope must be
/// treated as a failure, not silently parsed into a default "verified".
#[tokio::test]
async fn paypal_rejects_an_unparseable_verification_response() {
    let server = paypal_stub_responding(StubResponse::json(200, "not json")).await;

    let err = paypal_provider(&server)
        .verify_webhook(PAYPAL_EVENT.as_bytes(), &paypal_headers())
        .await
        .expect_err("an undecodable verification response proves nothing");

    assert!(
        matches!(err, PaymentError::InvalidWebhookSignature),
        "expected InvalidWebhookSignature, got {err:?}"
    );
}

/// Without a webhook ID there is nothing to verify *against*: PayPal checks the
/// signature for a specific registered webhook. A provider missing one must say
/// so as a configuration fault rather than calling PayPal with an empty ID and
/// interpreting whatever comes back.
#[tokio::test]
async fn paypal_requires_a_configured_webhook_id() {
    let server = paypal_stub("SUCCESS").await;
    let provider = PayPalProvider::new("client-id", "client-secret")
        .with_base_url(server.url())
        .unwrap();

    let err = provider
        .verify_webhook(PAYPAL_EVENT.as_bytes(), &paypal_headers())
        .await
        .expect_err("verification without a webhook ID is not possible");

    assert!(
        matches!(err, PaymentError::Config(_)),
        "a missing webhook ID is a misconfiguration, not a bad signature: {err:?}"
    );
    assert!(
        server.requests().is_empty(),
        "a misconfigured provider must not call PayPal at all; got {:?}",
        server.requests()
    );
}

/// Verification POSTs the whole event body back to PayPal, so an unbounded
/// payload lets an unauthenticated caller make this process allocate — and
/// upload — arbitrarily much. The cap must be enforced before any of that.
#[tokio::test]
async fn paypal_rejects_an_oversized_payload() {
    const CAP: usize = 256 * 1024;
    let filler = "A".repeat(CAP);
    let oversized = format!(r#"{{"id":"WH-1","event_type":"X","resource":{{"n":"{filler}"}}}}"#);
    assert!(oversized.len() > CAP, "the fixture must exceed the cap");

    let server = paypal_stub("SUCCESS").await;

    let err = paypal_provider(&server)
        .verify_webhook(oversized.as_bytes(), &paypal_headers())
        .await
        .expect_err("a payload beyond the cap must be refused");

    assert!(
        matches!(
            err,
            PaymentError::InvalidWebhookSignature | PaymentError::Validation(_)
        ),
        "an oversized payload must be refused explicitly, got {err:?}"
    );
    assert!(
        server.requests().is_empty(),
        "an oversized payload must be refused before any network call; got {:?}",
        server.requests()
    );
}

/// A payload just under the cap is ordinary traffic and must still verify.
#[tokio::test]
async fn paypal_accepts_a_payload_within_the_cap() {
    let filler = "A".repeat(1024);
    let payload = format!(r#"{{"id":"WH-1","event_type":"X","resource":{{"n":"{filler}"}}}}"#);

    let server = paypal_stub("SUCCESS").await;
    paypal_provider(&server)
        .verify_webhook(payload.as_bytes(), &paypal_headers())
        .await
        .expect("a normal-sized payload must not be caught by the size cap");
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
