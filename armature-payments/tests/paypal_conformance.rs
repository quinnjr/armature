#![cfg(feature = "paypal")]
//! PayPal conformance.
//!
//! Regression tests for a `capture` that ignored its partial amount, a
//! `create_customer` that minted a local UUID PayPal had never seen, an
//! `update_subscription` that silently discarded the new plan, and a
//! subscription projection that invented a 30-day billing period on every read.

use armature_payments::providers::paypal::PayPalProvider;
use armature_payments::{
    ChargeRequest, ChargeStatus, CreateCustomerRequest, Currency, Money, PaymentError,
    PaymentProvider, PaymentSource, SubscriptionStatus,
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
    PayPalProvider::new("client-id", "client-secret")
        .expect("HTTP client builds")
        .with_base_url(server.url())
        .unwrap()
}

const CAPTURE_PATH: &str = "/v2/checkout/orders/ORDER1/capture";

/// A stub whose capture endpoint returns an order in `status`.
async fn capture_stub(status: &str) -> StubServer {
    StubServer::builder()
        .route("POST", "/v1/oauth2/token", token_route())
        .route(
            "POST",
            CAPTURE_PATH,
            StubResponse::json(
                200,
                format!(
                    r#"{{"id":"ORDER1","status":"{status}",
                        "purchase_units":[{{"amount":{{"currency_code":"USD","value":"99.00"}}}}]}}"#
                ),
            ),
        )
        .start()
        .await
}

/// PayPal's orders-capture endpoint has no amount field at all, so a partial
/// capture cannot be expressed there. The original code accepted the amount and
/// dropped it, capturing the *full* authorization while reporting the partial
/// figure back to the caller — an overcharge the caller could not see.
///
/// Refusing the call is the only safe answer; the error must name the
/// alternative (an AUTHORIZE-intent order captured via
/// `/v2/payments/authorizations/{id}/capture`) rather than just failing.
#[tokio::test]
async fn partial_capture_is_refused_rather_than_silently_capturing_everything() {
    let server = StubServer::builder()
        .route("POST", "/v1/oauth2/token", token_route())
        .route(
            "POST",
            CAPTURE_PATH,
            StubResponse::json(
                200,
                r#"{"id":"ORDER1","status":"COMPLETED",
                    "purchase_units":[{"amount":{"currency_code":"USD","value":"99.00"}}]}"#,
            ),
        )
        .start()
        .await;

    let err = provider(&server)
        .capture("ORDER1", Some(Money::usd(1250)))
        .await
        .expect_err("a partial capture PayPal cannot perform must not report success");

    match err {
        PaymentError::Unsupported(msg) => assert!(
            msg.contains("authorization"),
            "the error must point at the endpoint that can do this: {msg}"
        ),
        other => panic!("expected Unsupported, got {other:?}"),
    }

    assert!(
        !server.requests().iter().any(|r| r.path == CAPTURE_PATH),
        "a capture that cannot honor the requested amount must not be sent at \
         all — sending it captures the full authorization"
    );
}

/// A full capture takes no body whatsoever on this endpoint.
#[tokio::test]
async fn full_capture_sends_no_amount() {
    let server = StubServer::builder()
        .route("POST", "/v1/oauth2/token", token_route())
        .route(
            "POST",
            CAPTURE_PATH,
            StubResponse::json(
                200,
                r#"{"id":"ORDER1","status":"COMPLETED",
                    "purchase_units":[{"amount":{"currency_code":"USD","value":"99.00"}}]}"#,
            ),
        )
        .start()
        .await;

    provider(&server).capture("ORDER1", None).await.unwrap();

    let sent = server.assert_received("POST", CAPTURE_PATH);
    let body = sent.body_string();
    assert!(
        !body.contains("amount"),
        "a full capture must not pin an amount: {body}"
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

    // Either explicit-refusal variant is acceptable here; what must not happen
    // is a locally minted ID or a bare `Ok`.
    match err {
        PaymentError::Unsupported(msg) => assert!(
            msg.contains("customer"),
            "the error must explain why: {msg}"
        ),
        PaymentError::Provider { message, .. } => assert!(
            message.contains("customer"),
            "the error must explain why: {message}"
        ),
        other => panic!("expected an explicit refusal, got {other:?}"),
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

    // The shared classifier keeps the status in the message so an operator can
    // tell a rejected plan change from a missing subscription.
    match err {
        PaymentError::Provider { status, message } => {
            assert_eq!(status, Some(422), "the status must ride on the variant");
            assert!(
                message.contains("422"),
                "the status must survive: {message}"
            );
            assert!(
                message.contains("PLAN_CHANGE_NOT_ALLOWED"),
                "PayPal's reason must survive: {message}"
            );
        }
        other => panic!("expected Provider, got {other:?}"),
    }
}

/// A 404 on revise means the subscription is gone, which is a different
/// operator response from "that plan change is not allowed".
#[tokio::test]
async fn update_subscription_reports_a_missing_subscription() {
    let server = StubServer::builder()
        .route("POST", "/v1/oauth2/token", token_route())
        .route(
            "POST",
            "/v1/billing/subscriptions/I-GONE/revise",
            StubResponse::json(404, r#"{"name":"RESOURCE_NOT_FOUND"}"#),
        )
        .start()
        .await;

    let err = provider(&server)
        .update_subscription("I-GONE", "P-PLAN2")
        .await
        .unwrap_err();
    assert!(
        matches!(err, PaymentError::SubscriptionNotFound(ref id) if id == "I-GONE"),
        "a 404 must name the missing subscription, got {err:?}"
    );
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

// --------------------------------------------- response mapping (item 7) ---

/// `capture` hardcoded `status: ChargeStatus::Succeeded` and `captured: true`,
/// ignoring the order status PayPal actually returned. A PENDING capture — funds
/// authorized but under review — was reported as money in hand.
#[tokio::test]
async fn a_pending_capture_is_not_reported_as_completed() {
    let server = capture_stub("PENDING").await;

    let charge = provider(&server).capture("ORDER1", None).await.unwrap();

    assert_ne!(
        charge.status,
        ChargeStatus::Succeeded,
        "a PENDING capture has not succeeded"
    );
    assert!(
        !charge.captured,
        "a PENDING capture has captured nothing yet"
    );
}

/// A declined capture must not come back as a successful charge.
#[tokio::test]
async fn a_declined_capture_is_an_error_or_a_declined_status() {
    let server = capture_stub("DECLINED").await;

    match provider(&server).capture("ORDER1", None).await {
        Err(_) => {}
        Ok(charge) => {
            assert_ne!(
                charge.status,
                ChargeStatus::Succeeded,
                "a DECLINED capture is not a successful charge"
            );
            assert!(!charge.captured, "a DECLINED capture captured nothing");
        }
    }
}

/// Amounts were formatted with `format!("{:.2}", to_float())` regardless of
/// currency. PayPal rejects `"1000.00"` for JPY outright (`DECIMALS_NOT_SUPPORTED`),
/// so every yen transaction failed — and any gateway that accepted it would read
/// the value as a hundredfold overcharge.
#[tokio::test]
async fn a_jpy_charge_is_serialized_without_decimals() {
    let server = StubServer::builder()
        .route("POST", "/v1/oauth2/token", token_route())
        .route(
            "POST",
            "/v2/checkout/orders",
            StubResponse::json(
                200,
                r#"{"id":"ORDER1","status":"COMPLETED",
                    "purchase_units":[{"amount":{"currency_code":"JPY","value":"1000"}}]}"#,
            ),
        )
        .start()
        .await;

    provider(&server)
        .charge(ChargeRequest::new(
            Money::new(1000, Currency::JPY),
            PaymentSource::card("tok_1"),
        ))
        .await
        .unwrap();

    let sent = server.assert_received("POST", "/v2/checkout/orders");
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    let amount = &body["purchase_units"][0]["amount"];
    assert_eq!(amount["currency_code"], "JPY");
    assert_eq!(
        amount["value"],
        "1000",
        "a zero-decimal currency must not be padded: {}",
        sent.body_string()
    );
}

/// A USD amount must still carry its two decimal places.
#[tokio::test]
async fn a_usd_charge_keeps_two_decimal_places() {
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

    provider(&server)
        .charge(ChargeRequest::new(
            Money::usd(2999),
            PaymentSource::card("tok_1"),
        ))
        .await
        .unwrap();

    let body: serde_json::Value =
        serde_json::from_slice(&server.assert_received("POST", "/v2/checkout/orders").body)
            .unwrap();
    assert_eq!(body["purchase_units"][0]["amount"]["value"], "29.99");
}

/// `delete_customer` returned `Ok(())` without doing anything. PayPal has no
/// customer resource, so a caller deleting a customer for a GDPR erasure request
/// got a success response for an erasure that never happened.
#[tokio::test]
async fn delete_customer_reports_that_paypal_has_no_customer_api() {
    let server = StubServer::builder()
        .route("POST", "/v1/oauth2/token", token_route())
        .start()
        .await;

    let err = provider(&server)
        .delete_customer("cus_1")
        .await
        .expect_err("a no-op must not be reported as a successful deletion");

    assert!(
        matches!(err, PaymentError::Unsupported(_)),
        "expected Unsupported, got {err:?}"
    );
}

/// `list_payment_methods` returned `Ok(vec![])`, which reads as "this customer
/// has no saved payment methods" rather than "this provider cannot answer that".
/// A UI showing "no cards on file" for a customer who has several is a support
/// ticket at best and a duplicate-entry prompt at worst.
#[tokio::test]
async fn list_payment_methods_reports_unsupported_rather_than_an_empty_list() {
    let server = StubServer::builder()
        .route("POST", "/v1/oauth2/token", token_route())
        .start()
        .await;

    let err = provider(&server)
        .list_payment_methods("cus_1")
        .await
        .expect_err("an empty vec claims knowledge PayPal never supplied");

    assert!(
        matches!(err, PaymentError::Unsupported(_)),
        "expected Unsupported, got {err:?}"
    );
}

/// `cancel_subscription` bound its `immediate` argument to `_immediate` and
/// dropped it, so "cancel at the end of the billing period" cancelled the
/// subscription on the spot and cut off access the customer had already paid for.
#[tokio::test]
async fn cancel_subscription_does_not_silently_ignore_end_of_period() {
    let server = StubServer::builder()
        .route("POST", "/v1/oauth2/token", token_route())
        .route(
            "POST",
            "/v1/billing/subscriptions/I-SUB1/cancel",
            StubResponse::json(204, ""),
        )
        .route(
            "GET",
            "/v1/billing/subscriptions/I-SUB1",
            StubResponse::json(200, r#"{"id":"I-SUB1","status":"CANCELLED"}"#),
        )
        .start()
        .await;

    match provider(&server).cancel_subscription("I-SUB1", false).await {
        // PayPal's /cancel is unconditional; saying so is the honest outcome.
        Err(PaymentError::Unsupported(_)) => {
            assert!(
                !server
                    .requests()
                    .iter()
                    .any(|r| r.path == "/v1/billing/subscriptions/I-SUB1/cancel"),
                "an unsupported cancellation must not cancel the subscription anyway"
            );
        }
        Ok(sub) => assert_ne!(
            sub.status,
            SubscriptionStatus::Canceled,
            "cancel_subscription(_, false) must not cancel immediately"
        ),
        Err(other) => {
            panic!("expected Unsupported or genuine end-of-period handling, got {other:?}")
        }
    }
}

/// The immediate path must still work.
#[tokio::test]
async fn cancel_subscription_immediate_still_cancels() {
    let server = StubServer::builder()
        .route("POST", "/v1/oauth2/token", token_route())
        .route(
            "POST",
            "/v1/billing/subscriptions/I-SUB1/cancel",
            StubResponse::json(204, ""),
        )
        .route(
            "GET",
            "/v1/billing/subscriptions/I-SUB1",
            StubResponse::json(200, r#"{"id":"I-SUB1","status":"CANCELLED"}"#),
        )
        .start()
        .await;

    let sub = provider(&server)
        .cancel_subscription("I-SUB1", true)
        .await
        .expect("an immediate cancellation is supported");
    assert_eq!(sub.status, SubscriptionStatus::Canceled);
    server.assert_received("POST", "/v1/billing/subscriptions/I-SUB1/cancel");
}
