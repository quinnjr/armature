#![cfg(feature = "paypal")]
//! PayPal conformance.
//!
//! Regression tests for a `capture` that ignored its partial amount, a
//! `create_customer` that minted a local UUID PayPal had never seen, an
//! `update_subscription` that silently discarded the new plan, and a
//! subscription projection that invented a 30-day billing period on every read.

use armature_payments::providers::paypal::PayPalProvider;
use armature_payments::{
    CreateCustomerRequest, Money, PaymentError, PaymentProvider, SubscriptionStatus,
};
use armature_testkit::http_stub::{StubResponse, StubServer};

const SUBSCRIPTION_BODY: &str = r#"{
    "id":"I-SUB1","status":"ACTIVE","plan_id":"P-PLAN1","quantity":"3",
    "create_time":"2026-01-05T10:00:00Z",
    "subscriber":{"payer_id":"PAYER123"},
    "billing_info":{
        "next_billing_time":"2026-08-05T10:00:00Z",
        "last_payment":{"time":"2026-07-05T10:00:00Z"}
    }
}"#;

fn token_route() -> StubResponse {
    StubResponse::json(200, r#"{"access_token":"tok","expires_in":3600}"#)
}

fn provider(server: &StubServer) -> PayPalProvider {
    PayPalProvider::new("client-id", "client-secret").with_base_url(server.url())
}

/// A partial capture must send the amount; without it PayPal captures the whole
/// authorization.
#[tokio::test]
async fn partial_capture_sends_the_amount() {
    let server = StubServer::builder()
        .route("POST", "/v1/oauth2/token", token_route())
        .route(
            "POST",
            "/v2/checkout/orders/ORDER1/capture",
            StubResponse::json(
                200,
                r#"{"id":"ORDER1","status":"COMPLETED",
                    "purchase_units":[{"amount":{"currency_code":"USD","value":"12.50"}}]}"#,
            ),
        )
        .start()
        .await;

    let charge = provider(&server)
        .capture("ORDER1", Some(Money::usd(1250)))
        .await
        .unwrap();
    assert_eq!(charge.amount.amount, 1250);

    let sent = server.assert_received("POST", "/v2/checkout/orders/ORDER1/capture");
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    assert_eq!(
        body["amount"]["value"],
        "12.50",
        "a partial capture must carry the amount: {}",
        sent.body_string()
    );
    assert_eq!(body["amount"]["currency_code"], "USD");
}

#[tokio::test]
async fn full_capture_omits_the_amount() {
    let server = StubServer::builder()
        .route("POST", "/v1/oauth2/token", token_route())
        .route(
            "POST",
            "/v2/checkout/orders/ORDER1/capture",
            StubResponse::json(
                200,
                r#"{"id":"ORDER1","status":"COMPLETED",
                    "purchase_units":[{"amount":{"currency_code":"USD","value":"99.00"}}]}"#,
            ),
        )
        .start()
        .await;

    provider(&server).capture("ORDER1", None).await.unwrap();

    let sent = server.assert_received("POST", "/v2/checkout/orders/ORDER1/capture");
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    assert!(
        body.get("amount").is_none_or(|a| a.is_null()),
        "a full capture must not pin an amount: {}",
        sent.body_string()
    );
}

/// PayPal has no customer API; returning a local UUID guaranteed that every
/// subsequent get/update failed with `CustomerNotFound`.
#[tokio::test]
async fn create_customer_returns_an_explicit_unsupported_error() {
    let server = StubServer::builder()
        .route("POST", "/v1/oauth2/token", token_route())
        .start()
        .await;

    let err = provider(&server)
        .create_customer(CreateCustomerRequest::with_email("a@b.test"))
        .await
        .expect_err("PayPal cannot create a customer; it must say so");

    match err {
        PaymentError::Provider(msg) => assert!(
            msg.contains("customer"),
            "the error must explain why: {msg}"
        ),
        other => panic!("expected an explicit Provider error, got {other:?}"),
    }
}

/// `update_subscription` previously ignored `price_id` and just re-read the
/// unchanged subscription, reporting success for a plan change that never
/// happened.
#[tokio::test]
async fn update_subscription_revises_the_plan() {
    let server = StubServer::builder()
        .route("POST", "/v1/oauth2/token", token_route())
        .route(
            "POST",
            "/v1/billing/subscriptions/I-SUB1/revise",
            StubResponse::json(200, r#"{"plan_id":"P-PLAN2"}"#),
        )
        .route(
            "GET",
            "/v1/billing/subscriptions/I-SUB1",
            StubResponse::json(200, SUBSCRIPTION_BODY.replace("P-PLAN1", "P-PLAN2")),
        )
        .start()
        .await;

    let sub = provider(&server)
        .update_subscription("I-SUB1", "P-PLAN2")
        .await
        .unwrap();
    assert_eq!(sub.price_id, "P-PLAN2");

    let sent = server.assert_received("POST", "/v1/billing/subscriptions/I-SUB1/revise");
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    assert_eq!(
        body["plan_id"], "P-PLAN2",
        "the new plan must actually be sent"
    );
}

#[tokio::test]
async fn update_subscription_surfaces_a_revise_failure() {
    let server = StubServer::builder()
        .route("POST", "/v1/oauth2/token", token_route())
        .route(
            "POST",
            "/v1/billing/subscriptions/I-SUB1/revise",
            StubResponse::json(
                422,
                r#"{"name":"PLAN_CHANGE_NOT_ALLOWED","message":"nope"}"#,
            ),
        )
        .start()
        .await;

    let err = provider(&server)
        .update_subscription("I-SUB1", "P-PLAN2")
        .await
        .expect_err("a rejected plan change must not report success");
    assert!(matches!(err, PaymentError::Provider(m) if m == "nope"));
}

/// The billing period was fabricated as `now .. now + 30d` on every read.
#[tokio::test]
async fn subscription_reports_paypals_real_billing_period() {
    let server = StubServer::builder()
        .route("POST", "/v1/oauth2/token", token_route())
        .route(
            "GET",
            "/v1/billing/subscriptions/I-SUB1",
            StubResponse::json(200, SUBSCRIPTION_BODY),
        )
        .start()
        .await;

    let sub = provider(&server).get_subscription("I-SUB1").await.unwrap();

    assert_eq!(sub.status, SubscriptionStatus::Active);
    assert_eq!(sub.price_id, "P-PLAN1");
    assert_eq!(sub.quantity, 3);
    assert_eq!(sub.customer_id.as_deref(), Some("PAYER123"));
    assert_eq!(
        sub.current_period_start.map(|d| d.to_rfc3339()),
        Some("2026-07-05T10:00:00+00:00".to_string()),
        "the period start must come from last_payment.time"
    );
    assert_eq!(
        sub.current_period_end.map(|d| d.to_rfc3339()),
        Some("2026-08-05T10:00:00+00:00".to_string()),
        "the period end must come from billing_info.next_billing_time"
    );
    assert_eq!(
        sub.created_at.map(|d| d.to_rfc3339()),
        Some("2026-01-05T10:00:00+00:00".to_string())
    );
}

/// When PayPal reports no billing info, the fields stay `None` rather than
/// being invented.
#[tokio::test]
async fn subscription_leaves_unreported_fields_none() {
    let server = StubServer::builder()
        .route("POST", "/v1/oauth2/token", token_route())
        .route(
            "GET",
            "/v1/billing/subscriptions/I-SUB2",
            StubResponse::json(200, r#"{"id":"I-SUB2","status":"APPROVAL_PENDING"}"#),
        )
        .start()
        .await;

    let sub = provider(&server).get_subscription("I-SUB2").await.unwrap();

    assert_eq!(sub.status, SubscriptionStatus::Incomplete);
    assert!(sub.current_period_start.is_none());
    assert!(sub.current_period_end.is_none());
    assert!(sub.created_at.is_none());
    assert!(sub.customer_id.is_none());
}
