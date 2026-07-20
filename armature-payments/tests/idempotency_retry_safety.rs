//! Retry safety against real providers.
//!
//! Regression tests for the highest-severity defect in this crate:
//! `PaymentProcessor::with_retries` re-sent money-moving requests to providers
//! that never forwarded an idempotency key. With the default
//! `retry_failed: true, max_retries: 3`, one ambiguous timeout against Braintree
//! or PayPal could produce up to four *real, distinct* charges.
//!
//! The fix gates retrying on [`PaymentProvider::supports_idempotency`] and makes
//! each provider actually forward its gateway's dedup header. These tests pin
//! both halves: a provider that cannot deduplicate must be attempted exactly
//! once, and a provider that can must repeat the *same* key on every attempt.

use armature_payments::{
    ChargeRequest, Money, PaymentError, PaymentProcessor, PaymentProvider, PaymentSource,
    ProcessorConfig, RefundRequest,
};
use armature_testkit::http_stub::{StubResponse, StubServer};

/// Retry aggressively with no backoff, so a test that *does* retry finishes
/// fast and a test that must not retry fails loudly rather than flakily.
fn retrying_config() -> ProcessorConfig {
    ProcessorConfig {
        retry_failed: true,
        max_retries: 3,
        retry_delay_ms: 0,
        use_idempotency: true,
        log_transactions: false,
        ..ProcessorConfig::default()
    }
}

/// A 429 is the cleanest way to drive the retry path through a real HTTP stub:
/// every provider must map it to the retryable `RateLimited`, so whether a retry
/// happens is decided purely by `supports_idempotency()`.
fn throttled() -> StubResponse {
    StubResponse::json(
        429,
        r#"{"error":{"type":"rate_limit_error","message":"slow down"}}"#,
    )
}

fn charge_request() -> ChargeRequest {
    ChargeRequest::new(Money::usd(2999), PaymentSource::card("tok_visa"))
}

// --------------------------------------------------------------- Braintree ---

#[cfg(feature = "braintree")]
mod braintree {
    use super::*;
    use armature_payments::providers::braintree::BraintreeProvider;

    fn provider(server: &StubServer) -> BraintreeProvider {
        BraintreeProvider::new("merchant-1", "pub_key", "priv_key")
            .with_base_url(server.url())
            .unwrap()
    }

    /// Braintree's transaction API has no idempotency key. Retrying a charge
    /// whose outcome is unknown is therefore indistinguishable from authorizing
    /// a second payment, so the processor must attempt it exactly once even
    /// though the error is classified retryable.
    #[tokio::test]
    async fn a_provider_without_idempotency_is_never_retried() {
        let server = StubServer::builder()
            .route("POST", "/merchants/merchant-1/transactions", throttled())
            .start()
            .await;

        assert!(
            !provider(&server).supports_idempotency(),
            "Braintree cannot deduplicate a replayed transaction"
        );

        let processor = PaymentProcessor::with_config(provider(&server), retrying_config());
        let err = processor.charge(charge_request()).await.unwrap_err();

        // Guard against passing for the wrong reason: if 429 were misclassified
        // as a non-retryable error, the single-request assertion below would
        // hold even with the retry gate removed.
        assert!(
            matches!(err, PaymentError::RateLimited(_)),
            "a 429 must classify as the retryable RateLimited, else this test \
             proves nothing about the retry gate; got {err:?}"
        );
        assert!(err.is_retryable(), "RateLimited is retryable by definition");

        assert_eq!(
            server.requests().len(),
            1,
            "a provider that cannot deduplicate must be charged exactly once, \
             whatever the retry policy says; got {:?}",
            server.requests()
        );
    }

    /// The same gate must protect refunds: a replayed refund pays the customer
    /// twice.
    #[tokio::test]
    async fn a_refund_is_not_retried_without_idempotency_support() {
        let server = StubServer::builder()
            .route(
                "POST",
                "/merchants/merchant-1/transactions/tx_1/refund",
                throttled(),
            )
            .start()
            .await;

        let processor = PaymentProcessor::with_config(provider(&server), retrying_config());
        processor
            .refund(RefundRequest::new("tx_1"))
            .await
            .unwrap_err();

        assert_eq!(
            server.requests().len(),
            1,
            "a refund must not be replayed against a gateway that cannot dedup it"
        );
    }
}

// ------------------------------------------------------------------ Stripe ---

#[cfg(feature = "stripe")]
mod stripe {
    use super::*;
    use armature_payments::providers::stripe::StripeProvider;

    fn provider(server: &StubServer) -> StripeProvider {
        StripeProvider::new("sk_test")
            .with_base_url(server.url())
            .unwrap()
    }

    /// Stripe deduplicates on `Idempotency-Key`, so retrying is safe *provided*
    /// the key is actually resent. A retry that omits it, or mints a fresh one,
    /// is a double charge.
    #[tokio::test]
    async fn stripe_retries_and_repeats_one_idempotency_key() {
        let server = StubServer::builder()
            .route("POST", "/charges", throttled())
            .start()
            .await;

        assert!(
            provider(&server).supports_idempotency(),
            "Stripe deduplicates on Idempotency-Key"
        );

        let processor = PaymentProcessor::with_config(provider(&server), retrying_config());
        processor.charge(charge_request()).await.unwrap_err();

        let requests = server.requests();
        assert_eq!(
            requests.len(),
            4,
            "max_retries=3 means one attempt plus three retries; got {requests:?}"
        );

        let keys: Vec<Option<&str>> = requests
            .iter()
            .map(|r| r.header("Idempotency-Key"))
            .collect();
        let first = keys[0].expect("the processor must generate a key when none is supplied");
        assert!(
            !first.is_empty(),
            "an empty Idempotency-Key deduplicates nothing"
        );
        assert!(
            keys.iter().all(|k| *k == Some(first)),
            "every retry must carry the same Idempotency-Key or Stripe treats it \
             as a new charge; got {keys:?}"
        );
    }

    /// `ProcessorConfig::use_idempotency` generated a key for charges only, so a
    /// retried refund reached Stripe unkeyed and could refund twice.
    #[tokio::test]
    async fn a_refund_key_is_generated_and_repeated_across_retries() {
        let server = StubServer::builder()
            .route("POST", "/refunds", throttled())
            .start()
            .await;

        let processor = PaymentProcessor::with_config(provider(&server), retrying_config());
        processor
            .refund(RefundRequest::new("ch_1"))
            .await
            .unwrap_err();

        let requests = server.requests();
        assert_eq!(requests.len(), 4, "Stripe refunds are safe to retry");

        let keys: Vec<Option<&str>> = requests
            .iter()
            .map(|r| r.header("Idempotency-Key"))
            .collect();
        let first = keys[0].expect("use_idempotency must generate a key for refunds too");
        assert!(
            keys.iter().all(|k| *k == Some(first)),
            "a retried refund must reuse its key; got {keys:?}"
        );
    }

    /// A caller-supplied refund key must survive verbatim: callers use it to tie
    /// the refund to their own ledger entry.
    #[tokio::test]
    async fn a_caller_supplied_refund_key_reaches_stripe_unchanged() {
        let server = StubServer::builder()
            .route("POST", "/refunds", throttled())
            .start()
            .await;

        let mut request = RefundRequest::new("ch_1");
        request.idempotency_key = Some("ledger-entry-77".into());

        let processor = PaymentProcessor::with_config(provider(&server), retrying_config());
        processor.refund(request).await.unwrap_err();

        for sent in server.requests() {
            assert_eq!(
                sent.header("Idempotency-Key"),
                Some("ledger-entry-77"),
                "the caller's refund key must not be replaced"
            );
        }
    }
}

// ------------------------------------------------------------------ PayPal ---

#[cfg(feature = "paypal")]
mod paypal {
    use super::*;
    use armature_payments::providers::paypal::PayPalProvider;

    fn token_route() -> StubResponse {
        StubResponse::json(200, r#"{"access_token":"tok","expires_in":3600}"#)
    }

    fn provider(server: &StubServer) -> PayPalProvider {
        PayPalProvider::new("client-id", "client-secret")
            .with_base_url(server.url())
            .unwrap()
    }

    /// PayPal deduplicates order creation on `PayPal-Request-Id`. Without it a
    /// retried order create opens a second order for the same purchase.
    #[tokio::test]
    async fn order_create_forwards_paypal_request_id() {
        let server = StubServer::builder()
            .route("POST", "/v1/oauth2/token", token_route())
            .route(
                "POST",
                "/v2/checkout/orders",
                StubResponse::json(
                    200,
                    r#"{"id":"ORDER1","status":"COMPLETED",
                        "purchase_units":[{"amount":{"currency_code":"USD","value":"29.99"}}]}"#,
                ),
            )
            .start()
            .await;

        let mut request = charge_request();
        request.idempotency_key = Some("order-key-1".into());

        provider(&server).charge(request).await.unwrap();

        let sent = server.assert_received("POST", "/v2/checkout/orders");
        assert_eq!(
            sent.header("PayPal-Request-Id"),
            Some("order-key-1"),
            "PayPal's dedup header must carry the request's idempotency key"
        );
    }

    /// Whatever key the processor generates must be repeated on every attempt,
    /// exactly as for Stripe.
    #[tokio::test]
    async fn paypal_repeats_one_request_id_across_retries() {
        let server = StubServer::builder()
            .route("POST", "/v1/oauth2/token", token_route())
            .route("POST", "/v2/checkout/orders", throttled())
            .start()
            .await;

        let supports = provider(&server).supports_idempotency();
        let processor = PaymentProcessor::with_config(provider(&server), retrying_config());
        processor.charge(charge_request()).await.unwrap_err();

        let orders: Vec<_> = server
            .requests()
            .into_iter()
            .filter(|r| r.path == "/v2/checkout/orders")
            .collect();

        if !supports {
            assert_eq!(
                orders.len(),
                1,
                "a provider that reports no idempotency support must not be retried"
            );
            return;
        }

        assert_eq!(orders.len(), 4, "max_retries=3 means four attempts");
        let ids: Vec<Option<&str>> = orders
            .iter()
            .map(|r| r.header("PayPal-Request-Id"))
            .collect();
        let first = ids[0].expect("a request id must be generated");
        assert!(
            ids.iter().all(|k| *k == Some(first)),
            "every retried order create must reuse one PayPal-Request-Id; got {ids:?}"
        );
    }
}
