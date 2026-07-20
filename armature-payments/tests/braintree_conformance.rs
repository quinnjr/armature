#![cfg(feature = "braintree")]
//! Braintree conformance.
//!
//! Regression tests for `create_payment_method` sending the literal sandbox
//! string `"fake-valid-nonce"` instead of the caller's payment method, for a
//! `capture` that ignored its partial amount, and for `list_payment_methods`
//! fetching the customer and then returning an empty vector.

use armature_payments::providers::braintree::BraintreeProvider;
use armature_payments::{
    CardDetails, CreatePaymentMethodRequest, Money, PaymentError, PaymentMethodType,
    PaymentProvider,
};
use armature_testkit::http_stub::{StubResponse, StubServer};

fn provider(server: &StubServer) -> BraintreeProvider {
    BraintreeProvider::new("merchant-1", "pub_key", "priv_key").with_base_url(server.url())
}

const PM_RESPONSE: &str = r#"{"payment_method":{"token":"pm_tok_1","customer_id":"cus_1",
    "card_type":"Visa","last_4":"4242","expiration_month":12,"expiration_year":2030}}"#;

/// The hardcoded sandbox nonce is gone: a request without a client-generated
/// nonce is refused rather than silently vaulting a test card.
#[tokio::test]
async fn create_payment_method_refuses_raw_card_details() {
    let server = StubServer::start_single(StubResponse::json(200, PM_RESPONSE)).await;

    let err = provider(&server)
        .create_payment_method(CreatePaymentMethodRequest::card(CardDetails {
            number: "4111111111111111".into(),
            exp_month: 12,
            exp_year: 2030,
            cvc: "123".into(),
        }))
        .await
        .expect_err("Braintree cannot accept raw card data server-side");

    match err {
        PaymentError::Validation(msg) => {
            assert!(msg.contains("nonce"), "the error must explain why: {msg}")
        }
        other => panic!("expected an explicit Validation error, got {other:?}"),
    }
    assert!(
        server.requests().is_empty(),
        "no request may be sent without a real nonce"
    );
}

#[tokio::test]
async fn create_payment_method_forwards_the_client_nonce() {
    let server = StubServer::builder()
        .route(
            "POST",
            "/merchants/merchant-1/payment_methods",
            StubResponse::json(200, PM_RESPONSE),
        )
        .start()
        .await;

    let method = provider(&server)
        .create_payment_method(CreatePaymentMethodRequest::nonce("tokencc_bh_real_nonce"))
        .await
        .unwrap();
    assert_eq!(method.id, "pm_tok_1");

    let sent = server.assert_received("POST", "/merchants/merchant-1/payment_methods");
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    assert_eq!(
        body["payment_method"]["payment_method_nonce"], "tokencc_bh_real_nonce",
        "the caller's nonce must be forwarded verbatim"
    );
    assert!(
        !sent.body_string().contains("fake-valid-nonce"),
        "the sandbox placeholder must never be sent: {}",
        sent.body_string()
    );
}

/// A partial settlement must send the amount, or Braintree settles the full
/// authorization.
#[tokio::test]
async fn partial_capture_sends_the_amount() {
    let server = StubServer::builder()
        .route(
            "PUT",
            "/merchants/merchant-1/transactions/tx_1/submit_for_settlement",
            StubResponse::json(
                200,
                r#"{"transaction":{"id":"tx_1","amount":"12.50","status":"submitted_for_settlement",
                    "currency_iso_code":"USD","customer_id":null,"payment_method_token":null,
                    "processor_response_text":null}}"#,
            ),
        )
        .start()
        .await;

    provider(&server)
        .capture("tx_1", Some(Money::usd(1250)))
        .await
        .unwrap();

    let sent = server.assert_received(
        "PUT",
        "/merchants/merchant-1/transactions/tx_1/submit_for_settlement",
    );
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    assert_eq!(
        body["transaction"]["amount"],
        "12.50",
        "a partial capture must carry the amount: {}",
        sent.body_string()
    );
}

#[tokio::test]
async fn full_capture_omits_the_amount() {
    let server = StubServer::builder()
        .route(
            "PUT",
            "/merchants/merchant-1/transactions/tx_1/submit_for_settlement",
            StubResponse::json(
                200,
                r#"{"transaction":{"id":"tx_1","amount":"99.00","status":"settling",
                    "currency_iso_code":"USD","customer_id":null,"payment_method_token":null,
                    "processor_response_text":null}}"#,
            ),
        )
        .start()
        .await;

    provider(&server).capture("tx_1", None).await.unwrap();

    let sent = server.assert_received(
        "PUT",
        "/merchants/merchant-1/transactions/tx_1/submit_for_settlement",
    );
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    assert!(
        body["transaction"]
            .get("amount")
            .is_none_or(|a| a.is_null()),
        "a full capture must not pin an amount: {}",
        sent.body_string()
    );
}

/// `list_payment_methods` fetched the customer and then discarded every vaulted
/// method it contained.
#[tokio::test]
async fn list_payment_methods_parses_the_vaulted_methods() {
    let server = StubServer::builder()
        .route(
            "GET",
            "/merchants/merchant-1/customers/cus_1",
            StubResponse::json(
                200,
                r#"{"customer":{"id":"cus_1","email":"a@b.test","first_name":"A","last_name":"B",
                    "phone":null,
                    "credit_cards":[
                      {"token":"card_1","card_type":"Visa","last_4":"4242",
                       "expiration_month":"12","expiration_year":"2030",
                       "created_at":"2026-01-01T00:00:00Z"},
                      {"token":"card_2","card_type":"MasterCard","last_4":"5454",
                       "expiration_month":"01","expiration_year":"2029"}
                    ],
                    "paypal_accounts":[{"token":"pp_1","email":"payer@b.test"}]}}"#,
            ),
        )
        .start()
        .await;

    let methods = provider(&server)
        .list_payment_methods("cus_1")
        .await
        .unwrap();

    assert_eq!(methods.len(), 3, "every vaulted method must be returned");

    let first = &methods[0];
    assert_eq!(first.id, "card_1");
    assert_eq!(first.method_type, PaymentMethodType::Card);
    assert_eq!(first.customer_id.as_deref(), Some("cus_1"));
    let card = first.card.as_ref().expect("card details");
    assert_eq!(card.brand, "Visa");
    assert_eq!(card.last4, "4242");
    assert_eq!(card.exp_month, 12);
    assert_eq!(card.exp_year, 2030);

    let paypal = methods
        .iter()
        .find(|m| m.id == "pp_1")
        .expect("paypal account");
    assert_eq!(paypal.method_type, PaymentMethodType::Paypal);
    assert_eq!(
        paypal
            .billing_details
            .as_ref()
            .and_then(|b| b.email.as_deref()),
        Some("payer@b.test")
    );
}

#[tokio::test]
async fn list_payment_methods_surfaces_a_missing_customer() {
    let server =
        StubServer::start_single(StubResponse::json(404, r#"{"message":"not found"}"#)).await;

    let err = provider(&server)
        .list_payment_methods("cus_missing")
        .await
        .unwrap_err();
    assert!(matches!(err, PaymentError::CustomerNotFound(id) if id == "cus_missing"));
}

/// The subscription projection invented a 30-day window anchored at "now".
#[tokio::test]
async fn subscription_reports_braintrees_real_billing_period() {
    let server = StubServer::builder()
        .route(
            "GET",
            "/merchants/merchant-1/subscriptions/sub_1",
            StubResponse::json(
                200,
                r#"{"subscription":{"id":"sub_1","status":"Active","plan_id":"plan_pro",
                    "quantity":4,"billing_period_start_date":"2026-07-01",
                    "billing_period_end_date":"2026-07-31",
                    "created_at":"2026-01-01T09:00:00Z"}}"#,
            ),
        )
        .start()
        .await;

    let sub = provider(&server).get_subscription("sub_1").await.unwrap();

    assert_eq!(sub.price_id, "plan_pro");
    assert_eq!(sub.quantity, 4);
    assert_eq!(
        sub.current_period_start.map(|d| d.date_naive().to_string()),
        Some("2026-07-01".to_string())
    );
    assert_eq!(
        sub.current_period_end.map(|d| d.date_naive().to_string()),
        Some("2026-07-31".to_string())
    );
    assert_eq!(
        sub.created_at.map(|d| d.to_rfc3339()),
        Some("2026-01-01T09:00:00+00:00".to_string())
    );
}

#[tokio::test]
async fn subscription_leaves_unreported_fields_none() {
    let server = StubServer::builder()
        .route(
            "GET",
            "/merchants/merchant-1/subscriptions/sub_2",
            StubResponse::json(200, r#"{"subscription":{"id":"sub_2","status":"Pending"}}"#),
        )
        .start()
        .await;

    let sub = provider(&server).get_subscription("sub_2").await.unwrap();
    assert!(sub.current_period_start.is_none());
    assert!(sub.current_period_end.is_none());
    assert!(sub.created_at.is_none());
    assert!(sub.customer_id.is_none());
}
