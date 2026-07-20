#![cfg(feature = "stripe")]
//! Stripe request/response conformance.
//!
//! Regression tests for methods that called `response.json()` without checking
//! the status (turning a real decline into `Serialization`), for the
//! subscription projection that hardcoded `price_id: ""` / `quantity: 1`, and
//! for the `PaymentSource` variants that were silently dropped.

use armature_payments::providers::stripe::StripeProvider;
use armature_payments::{
    ChargeRequest, CreateCustomerRequest, CreateSubscriptionRequest, Money, PaymentError,
    PaymentProvider, PaymentSource, RefundRequest, UpdateCustomerRequest,
};
use armature_testkit::http_stub::{StubResponse, StubServer};

const CARD_DECLINED: &str = r#"{"error":{"type":"card_error","code":"card_declined","decline_code":"generic_decline","message":"Your card was declined."}}"#;

fn provider(server: &StubServer) -> StripeProvider {
    StripeProvider::new("sk_test")
        .expect("HTTP client builds")
        .with_base_url(server.url())
        .expect("a loopback stub URL is a valid base URL")
}

async fn erroring_server(status: u16, body: &'static str) -> StubServer {
    StubServer::start_single(StubResponse::json(status, body)).await
}

/// Every fallible Stripe call must surface the provider's own error, not a
/// deserialization failure caused by trying to parse an error body as a
/// success body.
#[tokio::test]
async fn non_2xx_surfaces_the_real_provider_error_not_serialization() {
    let server = erroring_server(402, CARD_DECLINED).await;
    let p = provider(&server);

    let errors: Vec<PaymentError> = vec![
        p.refund(RefundRequest::new("ch_1")).await.unwrap_err(),
        p.capture("ch_1", None).await.unwrap_err(),
        p.create_customer(CreateCustomerRequest::with_email("a@b.test"))
            .await
            .unwrap_err(),
        p.get_customer("cus_1").await.unwrap_err(),
        p.update_customer("cus_1", UpdateCustomerRequest::default())
            .await
            .unwrap_err(),
        p.create_payment_method(armature_payments::CreatePaymentMethodRequest::card(
            armature_payments::CardDetails {
                number: "4242424242424242".into(),
                exp_month: 12,
                exp_year: 2030,
                cvc: "123".into(),
            },
        ))
        .await
        .unwrap_err(),
        p.attach_payment_method("pm_1", "cus_1").await.unwrap_err(),
        p.detach_payment_method("pm_1").await.unwrap_err(),
        p.list_payment_methods("cus_1").await.unwrap_err(),
        p.create_subscription(CreateSubscriptionRequest::new("cus_1", "price_1"))
            .await
            .unwrap_err(),
        p.get_subscription("sub_1").await.unwrap_err(),
        p.update_subscription("sub_1", "price_2").await.unwrap_err(),
        p.cancel_subscription("sub_1", false).await.unwrap_err(),
        p.cancel_subscription("sub_1", true).await.unwrap_err(),
        p.resume_subscription("sub_1").await.unwrap_err(),
    ];

    for err in errors {
        assert!(
            !matches!(err, PaymentError::Serialization(_)),
            "a declined request must not surface as Serialization: {err:?}"
        );
        assert!(
            matches!(err, PaymentError::CardDeclined(ref m) if m.contains("declined")),
            "expected the Stripe message to be preserved, got {err:?}"
        );
    }
}

/// `delete_customer` discarded the response entirely, so a 404/403/500 was
/// reported to the caller as a successful deletion.
///
/// This previously asserted only the negative — "not `Serialization`" — which
/// every one of `CustomerNotFound`, `Authentication`, `Provider` *and* a
/// wrong-but-plausible mapping satisfies. Pinning the exact variant is what
/// makes the test able to catch a regression: a deletion that failed because the
/// API key was revoked must not be reported as "that customer is already gone",
/// because the caller's correct response to those two differs.
#[tokio::test]
async fn delete_customer_checks_the_response_status() {
    type Check = fn(&PaymentError) -> bool;
    let cases: Vec<(u16, &'static str, Check, &'static str)> = vec![
        (
            // Refined at the call site: Stripe reports a missing resource as a
            // generic `invalid_request_error`, so only `delete_customer` knows
            // the 404 means "customer". Matches the other `/customers/{id}`
            // methods and the PayPal/Braintree providers.
            404,
            r#"{"error":{"type":"invalid_request_error","message":"No such customer"}}"#,
            |e| matches!(e, PaymentError::CustomerNotFound(id) if id == "cus_missing"),
            "CustomerNotFound naming the customer",
        ),
        (
            401,
            r#"{"error":{"type":"invalid_request_error","message":"Invalid API key"}}"#,
            |e| matches!(e, PaymentError::Authentication(_)),
            "Authentication",
        ),
        (
            // A 5xx from Stripe carries no usable error envelope, so the status
            // itself must survive into the message.
            500,
            "upstream exploded",
            |e| matches!(e, PaymentError::Provider(m) if m.contains("500")),
            "Provider containing \"500\"",
        ),
    ];

    for (status, body, check, expected) in cases {
        let server = StubServer::start_single(StubResponse::json(status, body)).await;

        let err = provider(&server)
            .delete_customer("cus_missing")
            .await
            .unwrap_err();
        assert!(
            check(&err),
            "HTTP {status} on delete_customer must map to {expected}, got {err:?}"
        );
    }

    let ok_server =
        StubServer::start_single(StubResponse::json(200, r#"{"id":"cus_1","deleted":true}"#)).await;
    provider(&ok_server)
        .delete_customer("cus_1")
        .await
        .expect("a 200 deletes");
}

/// A 404 on `/customers/{id}` means that customer is gone, and the caller's
/// correct response to that differs from a generic upstream failure: deleting an
/// already-deleted customer is usually success, whereas a `Provider` error is
/// something to alert on.
///
/// Stripe's shared error classifier cannot know this on its own — it sees only a
/// status and a generic `invalid_request_error` envelope. The resource-specific
/// mapping therefore has to be applied at the call site, the same pattern
/// `BraintreeProvider::list_payment_methods` and `paypal_error_not_found` use:
/// pass a `CustomerNotFound(id)` for the 404 case and let the classifier handle
/// every other status.
///
/// Done via `stripe_unit_as`, the unit-response counterpart to `stripe_json_as`,
/// so `delete_customer` refines its 404 the same way the other seven
/// `/customers/{id}` call sites already did.
#[tokio::test]
async fn delete_customer_reports_a_missing_customer_as_customer_not_found() {
    let server = StubServer::start_single(StubResponse::json(
        404,
        r#"{"error":{"type":"invalid_request_error","message":"No such customer: cus_missing"}}"#,
    ))
    .await;

    let err = provider(&server)
        .delete_customer("cus_missing")
        .await
        .unwrap_err();

    assert!(
        matches!(err, PaymentError::CustomerNotFound(ref id) if id == "cus_missing"),
        "a 404 on a customer endpoint must name the missing customer, got {err:?}"
    );
}

#[tokio::test]
async fn error_statuses_map_to_typed_errors() {
    type Check = fn(&PaymentError) -> bool;
    let cases: Vec<(u16, &'static str, Check)> = vec![
        (
            429,
            r#"{"error":{"type":"rate_limit_error","message":"Too many requests"}}"#,
            |e| matches!(e, PaymentError::RateLimited(_)),
        ),
        (
            401,
            r#"{"error":{"type":"authentication_error","message":"Invalid API key"}}"#,
            |e| matches!(e, PaymentError::Authentication(_)),
        ),
        (
            402,
            r#"{"error":{"type":"card_error","code":"expired_card","message":"Card expired"}}"#,
            |e| matches!(e, PaymentError::CardExpired),
        ),
        (
            402,
            r#"{"error":{"type":"card_error","decline_code":"insufficient_funds","message":"No funds"}}"#,
            |e| matches!(e, PaymentError::InsufficientFunds),
        ),
        (
            500,
            "not json at all",
            |e| matches!(e, PaymentError::Provider(m) if m.contains("500")),
        ),
    ];

    for (status, body, check) in cases {
        let server = StubServer::start_single(StubResponse::json(status, body)).await;
        let err = provider(&server).get_customer("cus_1").await.unwrap_err();
        assert!(check(&err), "unexpected mapping for {status}: {err:?}");
    }
}

/// The `From<StripeSubscription>` impl hardcoded `price_id: String::new()` and
/// `quantity: 1`, discarding what the customer is actually subscribed to.
#[tokio::test]
async fn subscription_reads_price_and_quantity_from_items() {
    let body = r#"{
        "id":"sub_1","customer":"cus_1","status":"active",
        "current_period_start":1750000000,"current_period_end":1752592000,
        "trial_end":null,"cancel_at_period_end":false,"canceled_at":null,
        "created":1749000000,
        "items":{"data":[{"price":{"id":"price_pro_monthly"},"quantity":7}]}
    }"#;
    let server = StubServer::start_single(StubResponse::json(200, body)).await;

    let sub = provider(&server).get_subscription("sub_1").await.unwrap();

    assert_eq!(sub.price_id, "price_pro_monthly");
    assert_eq!(sub.quantity, 7);
    assert_eq!(sub.customer_id.as_deref(), Some("cus_1"));
    assert_eq!(
        sub.current_period_start.map(|d| d.timestamp()),
        Some(1750000000)
    );
    assert_eq!(sub.created_at.map(|d| d.timestamp()), Some(1749000000));
}

#[tokio::test]
async fn subscription_without_items_falls_back_without_inventing_a_price() {
    let body = r#"{
        "id":"sub_1","customer":"cus_1","status":"active",
        "current_period_start":1,"current_period_end":2,
        "cancel_at_period_end":false,"created":1,"items":{"data":[]}
    }"#;
    let server = StubServer::start_single(StubResponse::json(200, body)).await;
    let sub = provider(&server).get_subscription("sub_1").await.unwrap();
    assert!(sub.price_id.is_empty());
    assert_eq!(sub.quantity, 1);
}

/// `PaymentSource::PaymentMethod` fell through a `_ => {}` arm on the Charges
/// API, so the charge was submitted with no payment source at all.
#[tokio::test]
async fn payment_method_source_routes_through_payment_intents() {
    let server = StubServer::builder()
        .route(
            "POST",
            "/payment_intents",
            StubResponse::json(
                200,
                r#"{"id":"pi_1","amount":2999,"currency":"usd","status":"succeeded",
                    "client_secret":"cs_1","customer":"cus_1","payment_method":"pm_1",
                    "created":1750000000,"amount_received":2999}"#,
            ),
        )
        .start()
        .await;

    let charge = provider(&server)
        .charge(ChargeRequest::new(
            Money::usd(2999),
            PaymentSource::payment_method("pm_1"),
        ))
        .await
        .expect("a PaymentMethod source must be charged, not dropped");

    assert_eq!(charge.id, "pi_1");
    assert_eq!(charge.amount.amount, 2999);
    assert!(charge.captured);
    assert_eq!(charge.payment_method.as_deref(), Some("pm_1"));

    let sent = server.assert_received("POST", "/payment_intents");
    let body = sent.body_string();
    assert!(
        body.contains("payment_method=pm_1"),
        "the payment method must reach Stripe: {body}"
    );
    assert!(
        !server.requests().iter().any(|r| r.path == "/charges"),
        "a PaymentMethod must not be sent to the legacy Charges API"
    );
}

/// `PaymentSource::Bank` was also dropped; a bank-account token is a `source`
/// on the Charges API.
#[tokio::test]
async fn bank_source_is_sent_as_a_charge_source() {
    let server = StubServer::builder()
        .route(
            "POST",
            "/charges",
            StubResponse::json(
                200,
                r#"{"id":"ch_1","amount":5000,"currency":"usd","status":"pending",
                    "customer":null,"payment_method":null,"description":null,
                    "receipt_url":null,"failure_message":null,"captured":false,
                    "refunded":false,"disputed":false,"created":1750000000}"#,
            ),
        )
        .start()
        .await;

    provider(&server)
        .charge(ChargeRequest::new(
            Money::usd(5000),
            PaymentSource::Bank {
                token: "btok_1".into(),
            },
        ))
        .await
        .expect("a bank source must be charged, not dropped");

    let body = server.assert_received("POST", "/charges").body_string();
    assert!(
        body.contains("source=btok_1"),
        "the bank token must reach Stripe: {body}"
    );
}

#[tokio::test]
async fn payment_intents_reject_raw_card_and_bank_tokens_explicitly() {
    let server = StubServer::start_single(StubResponse::json(200, "{}")).await;
    let p = provider(&server);

    for source in [
        PaymentSource::card("tok_visa"),
        PaymentSource::Bank {
            token: "btok_1".into(),
        },
    ] {
        let err = p
            .create_payment_intent(ChargeRequest::new(Money::usd(100), source))
            .await
            .unwrap_err();
        assert!(
            matches!(err, PaymentError::Validation(_)),
            "unsupported intent sources must be rejected explicitly: {err:?}"
        );
    }
    assert!(
        server.requests().is_empty(),
        "an unsupported source must not reach Stripe"
    );
}

/// `ChargeRequest::idempotency_key` and `statement_descriptor` were read by no
/// provider.
#[tokio::test]
async fn charge_sends_idempotency_key_statement_descriptor_and_metadata() {
    let server = StubServer::builder()
        .route(
            "POST",
            "/charges",
            StubResponse::json(
                200,
                r#"{"id":"ch_1","amount":1000,"currency":"usd","status":"succeeded",
                    "customer":null,"payment_method":null,"description":null,
                    "receipt_url":null,"failure_message":null,"captured":true,
                    "refunded":false,"disputed":false,"created":1750000000}"#,
            ),
        )
        .start()
        .await;

    let mut request = ChargeRequest::new(Money::usd(1000), PaymentSource::card("tok_visa"))
        .metadata("order_id", "A-1");
    request.idempotency_key = Some("idem-key-123".into());
    request.statement_descriptor = Some("ACME STORE".into());

    provider(&server).charge(request).await.unwrap();

    let sent = server.assert_received("POST", "/charges");
    assert_eq!(
        sent.header("Idempotency-Key"),
        Some("idem-key-123"),
        "the idempotency key must be sent as a header"
    );
    let body = sent.body_string();
    assert!(
        body.contains("statement_descriptor=ACME+STORE") || body.contains("ACME%20STORE"),
        "statement descriptor missing from {body}"
    );
    assert!(
        body.contains("metadata%5Border_id%5D=A-1"),
        "metadata missing from {body}"
    );
}

/// `list_payment_methods` must follow Stripe's pagination.
///
/// Stripe's list endpoints default to `limit: 10` and report `has_more`
/// alongside a `starting_after` cursor. The previous implementation issued one
/// unpaginated GET and read neither, so a customer with 11 or more saved cards
/// silently lost every card past the tenth — a wallet UI rendered an incomplete
/// list with no error to say so.
#[tokio::test]
async fn list_payment_methods_requests_a_full_page_and_stops_when_told() {
    let server = StubServer::start_single(StubResponse::json(
        200,
        r#"{"object":"list","has_more":false,"data":[
            {"id":"pm_1","type":"card","customer":"cus_1","created":1700000000},
            {"id":"pm_2","type":"card","customer":"cus_1","created":1700000001}]}"#,
    ))
    .await;

    let methods = provider(&server)
        .list_payment_methods("cus_1")
        .await
        .unwrap();
    assert_eq!(methods.len(), 2);

    let requests = server.requests();
    assert_eq!(
        requests.len(),
        1,
        "has_more:false means one request; got {requests:?}"
    );

    let query = requests[0].query.as_deref().unwrap_or_default();
    assert!(
        query.contains("limit=100"),
        "an explicit limit must be sent or Stripe defaults to 10; got {query:?}"
    );
    assert!(
        !query.contains("starting_after"),
        "the first page carries no cursor; got {query:?}"
    );
}

/// When Stripe reports `has_more`, the next page must be requested with the last
/// object's id as the cursor — and the walk must terminate rather than trusting
/// a gateway that keeps saying there is more.
#[tokio::test]
async fn list_payment_methods_follows_the_cursor_and_terminates() {
    let server = StubServer::start_single(StubResponse::json(
        200,
        r#"{"object":"list","has_more":true,"data":[
            {"id":"pm_1","type":"card","customer":"cus_1","created":1700000000},
            {"id":"pm_last","type":"card","customer":"cus_1","created":1700000001}]}"#,
    ))
    .await;

    let methods = provider(&server)
        .list_payment_methods("cus_1")
        .await
        .unwrap();

    let requests = server.requests();
    assert!(
        requests.len() > 1,
        "has_more:true must drive a second request; got {requests:?}"
    );
    assert!(
        requests.len() <= 20,
        "a gateway that always answers has_more must not spin forever; \
         got {} requests",
        requests.len()
    );
    assert_eq!(
        methods.len(),
        requests.len() * 2,
        "every page's objects must be kept, not just the last page's"
    );

    let second = requests[1].query.as_deref().unwrap_or_default();
    assert!(
        second.contains("starting_after=pm_last"),
        "the cursor is the id of the last object on the previous page; got {second:?}"
    );
}
