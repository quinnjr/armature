#![cfg(feature = "stripe")]
//! Transport-error classification, and why it is financially load-bearing.
//!
//! Regression test for `From<reqwest::Error>` collapsing every transport failure
//! into the retryable `PaymentError::Network`. A body/decode failure happens
//! *after* the gateway answered — typically a 2xx for a charge it has already
//! committed — so retrying it re-posts a transaction the customer has already
//! paid. Only failures that may never have reached the gateway (connect,
//! timeout) may be retried.

use armature_payments::providers::stripe::StripeProvider;
use armature_payments::{
    ChargeRequest, Money, PaymentError, PaymentProcessor, PaymentProvider, PaymentSource,
    ProcessorConfig,
};
use armature_testkit::http_stub::{StubResponse, StubServer};

fn charge_request() -> ChargeRequest {
    ChargeRequest::new(Money::usd(1000), PaymentSource::card("tok_visa"))
}

/// A 2xx whose body cannot be decoded means the charge very likely succeeded and
/// only the reply was unreadable. Classifying that as `Network` made it
/// retryable, so the processor re-charged a customer who had already paid.
#[tokio::test]
async fn a_decode_failure_on_a_2xx_is_serialization_and_not_retryable() {
    let server = StubServer::builder()
        .route(
            "POST",
            "/charges",
            StubResponse::json(200, "this is not the JSON you are looking for"),
        )
        .start()
        .await;

    let provider = StripeProvider::new("sk_test")
        .expect("HTTP client builds")
        .with_base_url(server.url())
        .unwrap();

    let err = provider.charge(charge_request()).await.unwrap_err();

    assert!(
        matches!(err, PaymentError::Serialization(_)),
        "an undecodable success body must not be reported as a transport \
         failure: {err:?}"
    );
    assert!(
        !err.is_retryable(),
        "retrying a charge the gateway already committed double-charges the \
         customer: {err:?}"
    );
}

/// The processor must act on that classification, not merely record it.
#[tokio::test]
async fn a_committed_charge_with_an_unreadable_reply_is_never_re_sent() {
    let server = StubServer::builder()
        .route("POST", "/charges", StubResponse::json(200, "not json"))
        .start()
        .await;

    let processor = PaymentProcessor::with_config(
        StripeProvider::new("sk_test")
            .expect("HTTP client builds")
            .with_base_url(server.url())
            .unwrap(),
        ProcessorConfig {
            retry_failed: true,
            max_retries: 3,
            retry_delay_ms: 0,
            use_idempotency: true,
            log_transactions: false,
            ..ProcessorConfig::default()
        },
    );

    processor.charge(charge_request()).await.unwrap_err();

    assert_eq!(
        server.requests().len(),
        1,
        "a decode failure must terminate the retry loop immediately; got {:?}",
        server.requests()
    );
}

/// A refused connection never reached Stripe, so no money moved and retrying is
/// both safe and correct.
#[tokio::test]
async fn a_connect_failure_is_network_and_retryable() {
    // Port 1 on loopback: nothing listens there, and `validate_base_url`
    // permits plaintext http to a loopback host.
    let provider = StripeProvider::new("sk_test")
        .expect("HTTP client builds")
        .with_base_url("http://127.0.0.1:1")
        .unwrap();

    let err = provider.charge(charge_request()).await.unwrap_err();

    assert!(
        matches!(err, PaymentError::Network(_)),
        "a connection that was never established is a transport failure: {err:?}"
    );
    assert!(
        err.is_retryable(),
        "a request that never reached the gateway must be retryable: {err:?}"
    );
}

/// The retry loop must actually re-dial on a connect failure.
#[tokio::test]
async fn a_connect_failure_is_retried_up_to_max_retries() {
    let processor = PaymentProcessor::with_config(
        StripeProvider::new("sk_test")
            .expect("HTTP client builds")
            .with_base_url("http://127.0.0.1:1")
            .unwrap(),
        ProcessorConfig {
            retry_failed: true,
            max_retries: 2,
            retry_delay_ms: 0,
            use_idempotency: true,
            log_transactions: false,
            ..ProcessorConfig::default()
        },
    );

    let err = processor.charge(charge_request()).await.unwrap_err();
    assert!(matches!(err, PaymentError::Network(_)));
}

/// The two classifications must be distinguishable, not merely both non-empty:
/// this is the assertion that fails against the old blanket `Network` mapping.
#[tokio::test]
async fn decode_and_connect_failures_classify_differently() {
    let server = StubServer::builder()
        .route("POST", "/charges", StubResponse::json(200, "not json"))
        .start()
        .await;

    let decode_err = StripeProvider::new("sk_test")
        .expect("HTTP client builds")
        .with_base_url(server.url())
        .unwrap()
        .charge(charge_request())
        .await
        .unwrap_err();

    let connect_err = StripeProvider::new("sk_test")
        .expect("HTTP client builds")
        .with_base_url("http://127.0.0.1:1")
        .unwrap()
        .charge(charge_request())
        .await
        .unwrap_err();

    assert_ne!(
        decode_err.is_retryable(),
        connect_err.is_retryable(),
        "a decode failure and a connect failure must not share a retry \
         disposition: {decode_err:?} vs {connect_err:?}"
    );
}
