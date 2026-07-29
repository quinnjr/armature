#![cfg(feature = "braintree")]
//! Braintree conformance.
//!
//! Regression tests for `create_payment_method` sending the literal sandbox
//! string `"fake-valid-nonce"` instead of the caller's payment method, for a
//! `capture` that ignored its partial amount, and for `list_payment_methods`
//! fetching the customer and then returning an empty vector.

use armature_payments::providers::braintree::BraintreeProvider;
use armature_payments::{
    CardDetails, ChargeRequest, ChargeStatus, CreatePaymentMethodRequest, Currency, Money,
    PaymentError, PaymentMethodType, PaymentProvider, PaymentSource, RefundRequest, RefundStatus,
};
use armature_testkit::http_stub::{StubResponse, StubServer};

fn provider(server: &StubServer) -> BraintreeProvider {
    BraintreeProvider::new("merchant-1", "pub_key", "priv_key")
        .expect("HTTP client builds")
        .with_base_url(server.url())
        .unwrap()
}

const TX_PATH: &str = "/merchants/merchant-1/transactions";

/// A settlement stub returning one transaction verbatim.
async fn settlement_stub(body: &'static str) -> StubServer {
    StubServer::builder()
        .route(
            "PUT",
            "/merchants/merchant-1/transactions/tx_1/submit_for_settlement",
            StubResponse::json(200, body),
        )
        .start()
        .await
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

// --------------------------------------------- response mapping (item 7) ---
//
// The tests above assert request *shape*. These assert the returned value,
// which is where the money bugs live: `capture` and `refund` built their result
// from hardcoded literals rather than from Braintree's response, so a wrong
// currency, a decline, or a pending settlement were all reported as a
// successful USD transaction.

/// `capture` constructed its `Money` with a literal `Currency::USD`, discarding
/// `currency_iso_code`. A €12.50 settlement was reported as $12.50 — the same
/// integer minor units under a different currency, so nothing downstream could
/// notice.
#[tokio::test]
async fn capture_reads_the_currency_from_the_transaction() {
    let server = settlement_stub(
        r#"{"transaction":{"id":"tx_1","amount":"12.50","status":"settling",
            "currency_iso_code":"EUR","customer_id":null,"payment_method_token":null,
            "processor_response_text":null}}"#,
    )
    .await;

    let charge = provider(&server).capture("tx_1", None).await.unwrap();

    assert_eq!(
        charge.amount.currency,
        Currency::EUR,
        "the settled currency must come from currency_iso_code, not a literal USD"
    );
    assert_eq!(charge.amount.amount, 1250);
}

/// `capture` hardcoded `status: ChargeStatus::Succeeded`, so a settlement the
/// processor declined was handed back as a completed charge and the order
/// shipped against a payment that never cleared.
#[tokio::test]
async fn capture_does_not_report_a_declined_settlement_as_succeeded() {
    let server = settlement_stub(
        r#"{"transaction":{"id":"tx_1","amount":"12.50","status":"processor_declined",
            "currency_iso_code":"USD","customer_id":null,"payment_method_token":null,
            "processor_response_text":"Insufficient Funds"}}"#,
    )
    .await;

    let result = provider(&server).capture("tx_1", None).await;

    match result {
        Err(_) => {}
        Ok(charge) => assert_ne!(
            charge.status,
            ChargeStatus::Succeeded,
            "a processor_declined settlement is not a successful charge"
        ),
    }
}

/// `capture` also hardcoded `captured: true`.
#[tokio::test]
async fn capture_does_not_claim_a_declined_settlement_was_captured() {
    let server = settlement_stub(
        r#"{"transaction":{"id":"tx_1","amount":"12.50","status":"processor_declined",
            "currency_iso_code":"USD","customer_id":null,"payment_method_token":null,
            "processor_response_text":"Insufficient Funds"}}"#,
    )
    .await;

    if let Ok(charge) = provider(&server).capture("tx_1", None).await {
        assert!(!charge.captured, "a declined settlement captured no funds");
    }
}

/// `refund` hardcoded `RefundStatus::Succeeded`. Braintree returns
/// `submitted_for_settlement` for a refund that has been accepted but not yet
/// settled; reporting it as succeeded tells the caller the customer has their
/// money back when the transfer has not happened.
#[tokio::test]
async fn refund_reports_a_pending_settlement_as_pending() {
    let server = StubServer::builder()
        .route(
            "POST",
            "/merchants/merchant-1/transactions/tx_1/refund",
            StubResponse::json(
                200,
                r#"{"transaction":{"id":"rf_1","amount":"12.50",
                    "status":"submitted_for_settlement","currency_iso_code":"USD",
                    "customer_id":null,"payment_method_token":null,
                    "processor_response_text":null}}"#,
            ),
        )
        .start()
        .await;

    let refund = provider(&server)
        .refund(RefundRequest::new("tx_1"))
        .await
        .unwrap();

    assert_eq!(
        refund.status,
        RefundStatus::Pending,
        "a refund awaiting settlement has not succeeded yet"
    );
}

/// `refund` built its `Money` with a literal `Currency::USD` too.
#[tokio::test]
async fn refund_reads_the_currency_from_the_transaction() {
    let server = StubServer::builder()
        .route(
            "POST",
            "/merchants/merchant-1/transactions/tx_1/refund",
            StubResponse::json(
                200,
                r#"{"transaction":{"id":"rf_1","amount":"12.50","status":"settled",
                    "currency_iso_code":"EUR","customer_id":null,
                    "payment_method_token":null,"processor_response_text":null}}"#,
            ),
        )
        .start()
        .await;

    let refund = provider(&server)
        .refund(RefundRequest::new("tx_1"))
        .await
        .unwrap();

    assert_eq!(
        refund.status,
        RefundStatus::Succeeded,
        "a settled refund has succeeded"
    );
    assert_eq!(
        refund.amount.currency,
        Currency::EUR,
        "refunding a EUR transaction must not report USD"
    );
}

/// Amounts were serialized with `format!("{:.2}", to_float())`, which is wrong
/// for every zero-decimal currency. ¥1000 is `Money::new(1000, JPY)` — 1000 yen,
/// not 1000 sen — so sending "1000.00" asks Braintree for ¥1000.00, which the
/// gateway reads as ¥100000 once it applies its own scaling.
#[tokio::test]
async fn a_jpy_charge_is_serialized_without_decimals() {
    let server = StubServer::builder()
        .route(
            "POST",
            TX_PATH,
            StubResponse::json(
                200,
                r#"{"transaction":{"id":"tx_1","amount":"1000","status":"submitted_for_settlement",
                    "currency_iso_code":"JPY","customer_id":null,"payment_method_token":null,
                    "processor_response_text":null}}"#,
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

    let sent = server.assert_received("POST", TX_PATH);
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    assert_eq!(
        body["transaction"]["amount"],
        "1000",
        "a zero-decimal currency must not be padded to two decimal places: {}",
        sent.body_string()
    );
}

/// A USD amount must still carry its two decimal places.
#[tokio::test]
async fn a_usd_charge_keeps_two_decimal_places() {
    let server = StubServer::builder()
        .route(
            "POST",
            TX_PATH,
            StubResponse::json(
                200,
                r#"{"transaction":{"id":"tx_1","amount":"10.00","status":"submitted_for_settlement",
                    "currency_iso_code":"USD","customer_id":null,"payment_method_token":null,
                    "processor_response_text":null}}"#,
            ),
        )
        .start()
        .await;

    provider(&server)
        .charge(ChargeRequest::new(
            Money::usd(1000),
            PaymentSource::card("tok_1"),
        ))
        .await
        .unwrap();

    let body: serde_json::Value =
        serde_json::from_slice(&server.assert_received("POST", TX_PATH).body).unwrap();
    assert_eq!(body["transaction"]["amount"], "10.00");
}

/// `list_payment_methods` mapped *every* non-2xx to `CustomerNotFound`, so a
/// revoked API key or a throttle looked exactly like a deleted customer. That
/// misleads the operator and, because `CustomerNotFound` is not retryable, it
/// also silently discards a 429 that should have been retried.
#[tokio::test]
async fn list_payment_methods_distinguishes_auth_and_throttle_from_a_missing_customer() {
    type Check = fn(&PaymentError) -> bool;
    let cases: Vec<(u16, Check, &'static str)> = vec![
        (
            401,
            |e| matches!(e, PaymentError::Authentication(_)),
            "Authentication",
        ),
        (
            429,
            |e| matches!(e, PaymentError::RateLimited(_)),
            "RateLimited",
        ),
        (
            404,
            |e| matches!(e, PaymentError::CustomerNotFound(_)),
            "CustomerNotFound",
        ),
    ];

    for (status, check, expected) in cases {
        let server =
            StubServer::start_single(StubResponse::json(status, r#"{"message":"x"}"#)).await;
        let err = provider(&server)
            .list_payment_methods("cus_1")
            .await
            .unwrap_err();
        assert!(
            check(&err),
            "HTTP {status} must map to {expected}, got {err:?}"
        );
    }
}

/// `charge` matched only `PaymentSource::Card`, sending every other variant with
/// no payment method at all — Braintree then charged the customer's vaulted
/// default, or nothing.
#[tokio::test]
async fn charge_sends_a_vaulted_payment_method_as_a_token() {
    let server = StubServer::builder()
        .route(
            "POST",
            TX_PATH,
            StubResponse::json(
                200,
                r#"{"transaction":{"id":"tx_1","amount":"10.00","status":"submitted_for_settlement",
                    "currency_iso_code":"USD","customer_id":null,"payment_method_token":"pm_tok_9",
                    "processor_response_text":null}}"#,
            ),
        )
        .start()
        .await;

    provider(&server)
        .charge(ChargeRequest::new(
            Money::usd(1000),
            PaymentSource::PaymentMethod {
                id: "pm_tok_9".into(),
            },
        ))
        .await
        .expect("a vaulted payment method must be charged, not dropped");

    let sent = server.assert_received("POST", TX_PATH);
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    assert_eq!(
        body["transaction"]["payment_method_token"],
        "pm_tok_9",
        "a vaulted method is a token, not a nonce: {}",
        sent.body_string()
    );
    assert!(
        body["transaction"]
            .get("payment_method_nonce")
            .is_none_or(|n| n.is_null()),
        "a token must not also be sent as a nonce: {}",
        sent.body_string()
    );
}

/// Braintree has no bank-token charge on this API. Dropping the source silently
/// submitted a transaction with no funding instrument; refusing it explicitly is
/// the only safe answer.
#[tokio::test]
async fn charge_rejects_a_bank_source_explicitly() {
    let server = StubServer::start_single(StubResponse::json(200, "{}")).await;

    let err = provider(&server)
        .charge(ChargeRequest::new(
            Money::usd(1000),
            PaymentSource::Bank {
                token: "btok_1".into(),
            },
        ))
        .await
        .expect_err("an unsupported funding source must not be silently dropped");

    assert!(
        matches!(err, PaymentError::Validation(_)),
        "expected an explicit Validation error, got {err:?}"
    );
    assert!(
        server.requests().is_empty(),
        "a request with no usable funding source must not reach Braintree"
    );
}

/// `cancel_subscription` ignored its `immediate` flag entirely, so
/// `cancel_subscription(id, false)` — cancel at period end, the flow every
/// "keep access until you have paid through" UI depends on — terminated the
/// subscription immediately instead.
#[tokio::test]
async fn cancel_subscription_does_not_silently_ignore_end_of_period() {
    let server = StubServer::builder()
        .route(
            "PUT",
            "/merchants/merchant-1/subscriptions/sub_1/cancel",
            StubResponse::json(
                200,
                r#"{"subscription":{"id":"sub_1","status":"Canceled","plan_id":"plan_pro"}}"#,
            ),
        )
        .start()
        .await;

    let result = provider(&server).cancel_subscription("sub_1", false).await;

    match result {
        // Braintree's API cannot express "cancel at period end"; saying so is
        // the honest outcome.
        Err(PaymentError::Unsupported(_)) => {
            assert!(
                server.requests().is_empty(),
                "an unsupported cancellation must not cancel the subscription anyway"
            );
        }
        // Otherwise the provider must have implemented genuine end-of-period
        // behavior, which cannot be an immediate cancel.
        Ok(sub) => assert_ne!(
            sub.status,
            armature_payments::SubscriptionStatus::Canceled,
            "cancel_subscription(_, false) must not cancel immediately"
        ),
        Err(other) => {
            panic!("expected Unsupported or genuine end-of-period handling, got {other:?}")
        }
    }
}

/// A transaction amount Braintree sends in a shape we cannot parse must surface
/// as an error, not as a `Charge` reporting `$0.00`.
///
/// The old code did `txn.amount.parse::<f64>().unwrap_or(0.0)`, so a malformed
/// amount produced a *successful* charge for zero dollars: the money moved, the
/// caller recorded nothing, and no receipt, ledger entry, or reconciliation run
/// had any signal that the figure was invented. A charge that fails loudly can
/// be investigated; a charge that succeeds with the wrong amount cannot.
#[tokio::test]
async fn a_malformed_transaction_amount_fails_instead_of_reporting_zero() {
    for bad_amount in [r#""N/A""#, r#""""#, r#""1,234.00""#, r#""$120.00""#] {
        let body = Box::leak(
            format!(
                r#"{{"transaction":{{"id":"tx_1","amount":{bad_amount},"status":"settled",
                    "currency_iso_code":"USD"}}}}"#
            )
            .into_boxed_str(),
        );
        let server = StubServer::builder()
            .route("POST", TX_PATH, StubResponse::json(201, &*body))
            .start()
            .await;

        let err = provider(&server)
            .charge(ChargeRequest::new(
                Money::usd(12000),
                PaymentSource::card("nonce_1"),
            ))
            .await
            .expect_err("an unparseable amount must not be reported as a successful $0.00 charge");

        match err {
            PaymentError::Serialization(msg) => {
                assert!(
                    msg.contains("unparseable") && msg.contains("tx_1"),
                    "the error must name the problem and the transaction: {msg}"
                );
                assert!(
                    msg.contains("reconcile"),
                    "the caller must be told the charge may have gone through: {msg}"
                );
            }
            other => panic!("expected Serialization for {bad_amount}, got {other:?}"),
        }
    }
}

/// The same guard on the refund path: a refund reporting `$0.00` for money
/// returned to the customer is the identical defect.
#[tokio::test]
async fn a_malformed_refund_amount_fails_instead_of_reporting_zero() {
    let server = StubServer::builder()
        .route(
            "POST",
            "/merchants/merchant-1/transactions/tx_1/refund",
            StubResponse::json(
                201,
                r#"{"transaction":{"id":"rf_1","amount":"not-a-number","status":"settled",
                    "currency_iso_code":"USD"}}"#,
            ),
        )
        .start()
        .await;

    let err = provider(&server)
        .refund(RefundRequest::new("tx_1"))
        .await
        .expect_err("an unparseable refund amount must not become $0.00");
    assert!(matches!(err, PaymentError::Serialization(_)), "got {err:?}");
}

/// A well-formed amount is projected exactly, including through the decimal
/// values a binary float cannot represent.
#[tokio::test]
async fn a_well_formed_transaction_amount_is_exact() {
    let server = StubServer::builder()
        .route(
            "POST",
            TX_PATH,
            StubResponse::json(
                201,
                r#"{"transaction":{"id":"tx_1","amount":"0.29","status":"settled",
                    "currency_iso_code":"EUR"}}"#,
            ),
        )
        .start()
        .await;

    let charge = provider(&server)
        .charge(ChargeRequest::new(
            Money::eur(29),
            PaymentSource::card("nonce_1"),
        ))
        .await
        .unwrap();

    // 0.29 * 100.0 in binary floating point is 28.999999999999996.
    assert_eq!(charge.amount, Money::eur(29));
    assert_eq!(charge.amount.currency, Currency::EUR);
    assert_eq!(charge.status, ChargeStatus::Succeeded);
}
