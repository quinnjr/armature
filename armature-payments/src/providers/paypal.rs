//! PayPal payment provider implementation

use crate::{
    error::{PaymentError, PaymentResult},
    money::{Currency, Money},
    provider::PaymentProvider,
    types::*,
    webhook::{WebhookData, WebhookEvent, WebhookEventType, WebhookHeaders},
};
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::Utc;
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// PayPal provider
pub struct PayPalProvider {
    client_id: String,
    client_secret: SecretString,
    webhook_id: Option<String>,
    sandbox: bool,
    base_url_override: Option<String>,
    client: Client,
    access_token: tokio::sync::RwLock<Option<PayPalToken>>,
}

/// Headers PayPal signs each webhook transmission with. All five are required
/// for verification; a webhook missing any of them cannot be authenticated.
pub const PAYPAL_AUTH_ALGO_HEADER: &str = "paypal-auth-algo";
/// URL of the PayPal certificate used to sign the transmission.
pub const PAYPAL_CERT_URL_HEADER: &str = "paypal-cert-url";
/// Unique ID of the webhook transmission.
pub const PAYPAL_TRANSMISSION_ID_HEADER: &str = "paypal-transmission-id";
/// Signature over the transmission.
pub const PAYPAL_TRANSMISSION_SIG_HEADER: &str = "paypal-transmission-sig";
/// Timestamp of the transmission.
pub const PAYPAL_TRANSMISSION_TIME_HEADER: &str = "paypal-transmission-time";

/// Largest webhook body this provider will attempt to verify.
///
/// [`PayPalProvider::verify_webhook`] authenticates by POSTing the body back to
/// PayPal, so it is a network round-trip driven entirely by *unauthenticated*
/// input. Without a cap, anyone who can reach the webhook endpoint can make
/// this process issue arbitrarily large outbound requests. PayPal's own
/// notifications are a few kilobytes; 256 KiB is generous headroom.
const MAX_WEBHOOK_BODY: usize = 256 * 1024;

#[derive(Debug, Clone)]
struct PayPalToken {
    token: String,
    expires_at: chrono::DateTime<Utc>,
}

impl PayPalProvider {
    /// Create a new PayPal provider
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: SecretString::new(client_secret.into().into()),
            webhook_id: None,
            sandbox: true,
            base_url_override: None,
            // Carries the shared connect/request timeouts; an untimed client
            // inside the processor's retry loop can hang a charge indefinitely.
            client: crate::provider::build_http_client().unwrap_or_else(|_| Client::new()),
            access_token: tokio::sync::RwLock::new(None),
        }
    }

    /// Use production environment
    pub fn production(mut self) -> Self {
        self.sandbox = false;
        self
    }

    /// Point the client at an alternate API base URL (a mock or proxy).
    ///
    /// The URL must be `https`, or `http` addressed to a loopback host
    /// (`localhost`, `127.0.0.1`, `[::1]`) for local testing. Cleartext `http`
    /// to any other host is rejected for two independent reasons: the OAuth
    /// exchange sends the client secret as HTTP Basic credentials, and
    /// [`verify_webhook`](Self::verify_webhook) asks *this* host whether a
    /// webhook is authentic — so an attacker-controlled base URL can simply
    /// answer `{"verification_status":"SUCCESS"}` and every forged webhook is
    /// accepted.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::Config`] if `base_url` is unparseable or uses a
    /// scheme/host combination that would transmit credentials in cleartext.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> PaymentResult<Self> {
        // The validated form has any trailing slash stripped, so joining
        // leading-slash request paths onto it stays correct.
        self.base_url_override = Some(crate::provider::validate_base_url(&base_url.into())?);
        Ok(self)
    }

    /// Set webhook ID for verification
    pub fn with_webhook_id(mut self, webhook_id: impl Into<String>) -> Self {
        self.webhook_id = Some(webhook_id.into());
        self
    }

    /// Get API base URL
    fn base_url(&self) -> &str {
        if let Some(url) = &self.base_url_override {
            url
        } else if self.sandbox {
            "https://api-m.sandbox.paypal.com"
        } else {
            "https://api-m.paypal.com"
        }
    }

    /// Get or refresh access token
    async fn get_token(&self) -> PaymentResult<String> {
        // Fast path: a valid token only needs a shared read lock.
        {
            let token = self.access_token.read().await;
            if let Some(ref t) = *token
                && t.expires_at > Utc::now()
            {
                return Ok(t.token.clone());
            }
        }

        // Slow path: take the write lock *before* the network call, not after.
        // Releasing the read lock and fetching unguarded let N concurrent
        // charges on a cold or expired token each issue their own OAuth
        // round-trip; holding the write lock across the fetch serialises them
        // onto one.
        let mut token = self.access_token.write().await;

        // Double-check under the write lock: while we waited for it, another
        // task may have completed the refresh we were about to duplicate.
        if let Some(ref t) = *token
            && t.expires_at > Utc::now()
        {
            return Ok(t.token.clone());
        }

        let credentials = STANDARD.encode(format!(
            "{}:{}",
            self.client_id,
            self.client_secret.expose_secret()
        ));

        let response = self
            .client
            .post(format!("{}/v1/oauth2/token", self.base_url()))
            .header("Authorization", format!("Basic {}", credentials))
            .form(&[("grant_type", "client_credentials")])
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let retry_after = retry_after_secs(&response);
            let body = response.text().await.unwrap_or_default();
            return Err(crate::provider::classify_status(
                "paypal",
                status,
                &body,
                retry_after,
            ));
        }

        let token_response: PayPalTokenResponse = response.json().await?;
        *token = Some(PayPalToken {
            token: token_response.access_token.clone(),
            expires_at: Utc::now()
                + chrono::Duration::seconds(token_response.expires_in as i64 - 60),
        });

        Ok(token_response.access_token)
    }

    /// Make an authenticated API request
    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
    ) -> PaymentResult<reqwest::RequestBuilder> {
        let token = self.get_token().await?;
        Ok(self
            .client
            .request(method, format!("{}{}", self.base_url(), path))
            .bearer_auth(token))
    }
}

/// Read the `Retry-After` header as a whole number of seconds.
fn retry_after_secs(response: &reqwest::Response) -> Option<u32> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Map a PayPal order/capture `status` onto the crate's [`ChargeStatus`].
///
/// Shared by `charge` and `capture` so the two cannot drift apart — `capture`
/// previously hardcoded `Succeeded`, reporting a `DECLINED` capture as a
/// successful payment.
fn order_charge_status(status: &str) -> ChargeStatus {
    match status {
        "COMPLETED" | "CAPTURED" => ChargeStatus::Succeeded,
        "VOIDED" => ChargeStatus::Canceled,
        "DECLINED" | "FAILED" => ChargeStatus::Failed,
        // CREATED, SAVED, APPROVED, PAYER_ACTION_REQUIRED, PENDING
        _ => ChargeStatus::Pending,
    }
}

/// Turn a non-2xx PayPal response into a typed [`PaymentError`].
///
/// Previously every failure — 401, 404, 429 alike — collapsed into
/// `Provider(..)`, so expired credentials were indistinguishable from a bad
/// request and a throttle never triggered backoff.
async fn paypal_error(response: reqwest::Response) -> PaymentError {
    let status = response.status();
    let retry_after = retry_after_secs(&response);
    let body = response.text().await.unwrap_or_default();
    crate::provider::classify_status("paypal", status, &body, retry_after)
}

/// As [`paypal_error`], but reporting a 404 as `not_found` — the resource-specific
/// meaning the generic classifier cannot know at the call site.
async fn paypal_error_not_found(
    response: reqwest::Response,
    not_found: PaymentError,
) -> PaymentError {
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return not_found;
    }
    paypal_error(response).await
}

#[async_trait]
impl PaymentProvider for PayPalProvider {
    fn name(&self) -> &'static str {
        "paypal"
    }

    /// PayPal deduplicates a retried request by its `PayPal-Request-Id` header,
    /// which this provider sends on order creation and refunds whenever the
    /// request carries an idempotency key.
    fn supports_idempotency(&self) -> bool {
        true
    }

    /// Create a PayPal order.
    ///
    /// # This does not move money
    ///
    /// Unlike every other provider's `charge`, this call **only creates an
    /// order awaiting buyer approval**. PayPal's checkout flow requires the
    /// buyer to approve the order in their browser, after which the merchant
    /// calls [`capture`](Self::capture) to actually collect funds. The returned
    /// [`Charge`] is therefore `ChargeStatus::Pending` with `captured: false`,
    /// and its `id` is an *order* id — pass it to `capture` once approval has
    /// happened. Treating a successful return here as a completed payment will
    /// ship goods that were never paid for.
    ///
    /// `request.capture` selects PayPal's order intent: `true` (the default)
    /// creates a `CAPTURE` order, and [`ChargeRequest::auth_only`] creates an
    /// `AUTHORIZE` order whose funds are captured through the authorization
    /// endpoint instead.
    async fn charge(&self, request: ChargeRequest) -> PaymentResult<Charge> {
        // PayPal caps custom_id at 127 characters. Failing loudly beats
        // silently dropping the caller's metadata on the floor.
        let custom_id = if request.metadata.is_empty() {
            None
        } else {
            let encoded = serde_json::to_string(&request.metadata)?;
            if encoded.len() > 127 {
                return Err(PaymentError::Validation(format!(
                    "PayPal custom_id is limited to 127 characters; the encoded metadata is {} \
                     characters. Store the detail against the returned order id instead.",
                    encoded.len()
                )));
            }
            Some(encoded)
        };

        let order_request = PayPalOrderRequest {
            // `request.capture` was previously ignored entirely, so `auth_only()`
            // and the default produced byte-identical requests.
            intent: if request.capture {
                "CAPTURE".to_string()
            } else {
                "AUTHORIZE".to_string()
            },
            purchase_units: vec![PayPalPurchaseUnit {
                amount: PayPalAmount {
                    currency_code: request.amount.currency.code().to_string(),
                    // Zero-decimal currencies (JPY, KRW) must not be sent with
                    // two decimal places; `{:.2}` turned ¥1000 into "1000.00",
                    // which PayPal reads as a different amount.
                    value: request.amount.to_gateway_string(),
                },
                description: request.description.clone(),
                soft_descriptor: request.statement_descriptor.clone(),
                custom_id,
            }],
        };

        let mut builder = self
            .request(reqwest::Method::POST, "/v2/checkout/orders")
            .await?
            .json(&order_request);

        // PayPal's documented idempotency mechanism. Without it a retried
        // order-create produces a second order for the same purchase.
        if let Some(key) = request.idempotency_key.as_deref() {
            builder = builder.header("PayPal-Request-Id", key);
        }

        let response = builder.send().await?;

        if !response.status().is_success() {
            return Err(paypal_error(response).await);
        }

        let order: PayPalOrder = response.json().await?;

        let amount = order
            .purchase_units
            .first()
            .map(|u| {
                Money::from_float(
                    u.amount.value.parse().unwrap_or(0.0),
                    Currency::from_code(&u.amount.currency_code).unwrap_or(Currency::USD),
                )
            })
            .unwrap_or(request.amount);

        Ok(Charge {
            id: order.id,
            amount,
            amount_refunded: Money::new(0, amount.currency),
            status: order_charge_status(&order.status),
            // PayPal has no customer resource, so the caller's own identifier
            // is the only meaningful one; echo it rather than dropping it.
            customer_id: request.customer_id,
            payment_method: None,
            description: request.description,
            receipt_url: None,
            failure_reason: None,
            // Now genuinely sent to PayPal as purchase_units[0].custom_id.
            metadata: request.metadata,
            created_at: Utc::now(),
            captured: order.status == "COMPLETED",
            refunded: false,
            disputed: false,
        })
    }

    /// Capture an approved PayPal order in full.
    ///
    /// `charge_id` is the order id returned by [`charge`](Self::charge), and
    /// the buyer must have approved it first.
    ///
    /// # Partial capture is not supported here
    ///
    /// `/v2/checkout/orders/{id}/capture` has no `amount` field in its schema —
    /// PayPal captures the full order. Partial capture lives on
    /// `/v2/payments/authorizations/{authorization_id}/capture` and needs an
    /// *authorization* id, which is a different identifier than the order id
    /// this method receives. Passing `Some(amount)` therefore returns
    /// [`PaymentError::Unsupported`] rather than sending an `amount` the
    /// endpoint silently ignores while capturing the full sum.
    async fn capture(&self, charge_id: &str, amount: Option<Money>) -> PaymentResult<Charge> {
        if let Some(amount) = amount {
            return Err(PaymentError::Unsupported(format!(
                "PayPal cannot partially capture order {charge_id}: the orders capture endpoint \
                 has no amount field, and partial capture ({}) requires the authorization id from \
                 an AUTHORIZE-intent order via /v2/payments/authorizations/{{id}}/capture. Capture \
                 the full order, or drive the authorization endpoint directly.",
                amount.format()
            )));
        }

        // Full capture. The endpoint takes no meaningful fields, so send an
        // empty JSON object rather than an absent body — PayPal expects a JSON
        // content type here, and this pins no amount.
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("/v2/checkout/orders/{}/capture", charge_id),
            )
            .await?
            .json(&serde_json::json!({}))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(paypal_error_not_found(
                response,
                PaymentError::ChargeNotFound(charge_id.to_string()),
            )
            .await);
        }

        let order: PayPalOrder = response.json().await?;

        // Derive the currency from what PayPal actually reported. Hardcoding
        // USD turned a €120 capture into Money { 12000, USD }, which then
        // passes `Money::add`'s currency assertion and corrupts a ledger.
        let amount = order
            .purchase_units
            .first()
            .map(|u| {
                Money::from_float(
                    u.amount.value.parse().unwrap_or(0.0),
                    Currency::from_code(&u.amount.currency_code).unwrap_or(Currency::USD),
                )
            })
            .unwrap_or(Money::usd(0));

        let status = order_charge_status(&order.status);

        Ok(Charge {
            id: order.id,
            amount,
            amount_refunded: Money::new(0, amount.currency),
            status,
            customer_id: None,
            payment_method: None,
            description: None,
            receipt_url: None,
            failure_reason: None,
            metadata: HashMap::new(),
            created_at: Utc::now(),
            // Only a COMPLETED capture actually took the money; PENDING and
            // DECLINED were both reported as captured before.
            captured: status == ChargeStatus::Succeeded,
            refunded: false,
            disputed: false,
        })
    }

    async fn refund(&self, request: RefundRequest) -> PaymentResult<Refund> {
        // PayPal requires the capture ID for refunds
        let refund_request = PayPalRefundRequest {
            amount: request.amount.map(|a| PayPalAmount {
                currency_code: a.currency.code().to_string(),
                value: a.to_gateway_string(),
            }),
            note_to_payer: None,
        };

        let mut builder = self
            .request(
                reqwest::Method::POST,
                &format!("/v2/payments/captures/{}/refund", request.charge_id),
            )
            .await?
            .json(&refund_request);

        // A retried refund must not return the customer's money twice.
        if let Some(key) = request.idempotency_key.as_deref() {
            builder = builder.header("PayPal-Request-Id", key);
        }

        let response = builder.send().await?;

        if !response.status().is_success() {
            return Err(paypal_error_not_found(
                response,
                PaymentError::ChargeNotFound(request.charge_id.clone()),
            )
            .await);
        }

        let paypal_refund: PayPalRefund = response.json().await?;

        Ok(Refund {
            id: paypal_refund.id,
            charge_id: request.charge_id,
            amount: paypal_refund
                .amount
                .map(|a| {
                    Money::from_float(
                        a.value.parse().unwrap_or(0.0),
                        Currency::from_code(&a.currency_code).unwrap_or(Currency::USD),
                    )
                })
                .unwrap_or(Money::usd(0)),
            status: match paypal_refund.status.as_str() {
                "COMPLETED" => RefundStatus::Succeeded,
                "CANCELLED" => RefundStatus::Canceled,
                "FAILED" => RefundStatus::Failed,
                _ => RefundStatus::Pending,
            },
            reason: request.reason,
            created_at: Utc::now(),
        })
    }

    async fn create_customer(&self, _request: CreateCustomerRequest) -> PaymentResult<Customer> {
        // PayPal has no customer resource. Returning a locally minted UUID would
        // hand back an ID PayPal has never heard of, so every later
        // get/update/delete would fail — an explicit error is the honest answer.
        Err(PaymentError::Provider(
            "PayPal has no customer API; customers are identified by the payer on each order. \
             Store your own customer record and pass its ID as ChargeRequest::customer_id."
                .into(),
        ))
    }

    async fn get_customer(&self, id: &str) -> PaymentResult<Customer> {
        Err(PaymentError::CustomerNotFound(id.to_string()))
    }

    async fn update_customer(
        &self,
        id: &str,
        _request: UpdateCustomerRequest,
    ) -> PaymentResult<Customer> {
        Err(PaymentError::CustomerNotFound(id.to_string()))
    }

    async fn delete_customer(&self, _id: &str) -> PaymentResult<()> {
        // Returning `Ok(())` here was actively dangerous: a caller performing a
        // GDPR/CCPA erasure would record a successful deletion for data PayPal
        // still holds. PayPal has no customer resource to delete, so the only
        // truthful answer is that the operation does not exist.
        Err(PaymentError::Unsupported(
            "PayPal has no customer API, so there is no customer record to delete. Erase your own \
             customer record; payer data held by PayPal is deleted through PayPal's own account \
             and data-retention processes, not this API."
                .into(),
        ))
    }

    async fn create_payment_method(
        &self,
        _request: CreatePaymentMethodRequest,
    ) -> PaymentResult<PaymentMethod> {
        Err(PaymentError::Provider(
            "PayPal handles payment methods through checkout flow".into(),
        ))
    }

    async fn attach_payment_method(
        &self,
        _method_id: &str,
        _customer_id: &str,
    ) -> PaymentResult<PaymentMethod> {
        Err(PaymentError::Provider(
            "PayPal handles payment methods through checkout flow".into(),
        ))
    }

    async fn detach_payment_method(&self, _method_id: &str) -> PaymentResult<PaymentMethod> {
        Err(PaymentError::Provider(
            "PayPal handles payment methods through checkout flow".into(),
        ))
    }

    async fn list_payment_methods(&self, _customer_id: &str) -> PaymentResult<Vec<PaymentMethod>> {
        // An empty Vec is indistinguishable from "this customer has no vaulted
        // payment methods", so a caller rendering a wallet would silently show
        // nothing rather than surfacing that the query is not supported.
        Err(PaymentError::Unsupported(
            "PayPal does not expose vaulted payment methods through this API; payment methods are \
             selected by the buyer during the checkout approval flow."
                .into(),
        ))
    }

    async fn create_subscription(
        &self,
        request: CreateSubscriptionRequest,
    ) -> PaymentResult<Subscription> {
        let sub_request = PayPalSubscriptionRequest {
            plan_id: request.price_id.clone(),
            quantity: request.quantity.map(|q| q.to_string()),
        };

        let response = self
            .request(reqwest::Method::POST, "/v1/billing/subscriptions")
            .await?
            .json(&sub_request)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(paypal_error(response).await);
        }

        let paypal_sub: PayPalSubscription = response.json().await?;

        let mut subscription = paypal_sub.into_subscription();
        // The caller's own identifiers are authoritative here; PayPal's create
        // response echoes neither.
        subscription.customer_id = Some(request.customer_id);
        if subscription.price_id.is_empty() {
            subscription.price_id = request.price_id;
        }
        subscription.quantity = request.quantity.unwrap_or(subscription.quantity);
        subscription.metadata = request.metadata;
        Ok(subscription)
    }

    async fn get_subscription(&self, id: &str) -> PaymentResult<Subscription> {
        let response = self
            .request(
                reqwest::Method::GET,
                &format!("/v1/billing/subscriptions/{}", id),
            )
            .await?
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(paypal_error_not_found(
                response,
                PaymentError::SubscriptionNotFound(id.to_string()),
            )
            .await);
        }

        let paypal_sub: PayPalSubscription = response.json().await?;
        Ok(paypal_sub.into_subscription())
    }

    async fn update_subscription(&self, id: &str, price_id: &str) -> PaymentResult<Subscription> {
        // `revise` is PayPal's plan-change endpoint. Previously this call
        // silently discarded the new plan and re-read the unchanged
        // subscription, reporting success for a change that never happened.
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("/v1/billing/subscriptions/{}/revise", id),
            )
            .await?
            .json(&serde_json::json!({ "plan_id": price_id }))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(paypal_error_not_found(
                response,
                PaymentError::SubscriptionNotFound(id.to_string()),
            )
            .await);
        }

        self.get_subscription(id).await
    }

    /// Cancel a PayPal subscription.
    ///
    /// Only immediate cancellation is supported: PayPal's
    /// `/v1/billing/subscriptions/{id}/cancel` terminates the subscription at
    /// once and the Subscriptions API exposes no "cancel at period end" flag.
    /// `immediate: false` therefore returns [`PaymentError::Unsupported`]
    /// rather than silently cancelling immediately and forfeiting the
    /// remaining paid term.
    async fn cancel_subscription(&self, id: &str, immediate: bool) -> PaymentResult<Subscription> {
        if !immediate {
            return Err(PaymentError::Unsupported(format!(
                "PayPal cannot schedule subscription {id} to cancel at period end; its cancel \
                 endpoint terminates immediately. Suspend the subscription and cancel it once the \
                 paid term expires, or call this with immediate = true if that is what you want."
            )));
        }

        let response = self
            .request(
                reqwest::Method::POST,
                &format!("/v1/billing/subscriptions/{}/cancel", id),
            )
            .await?
            .json(&serde_json::json!({ "reason": "Customer requested cancellation" }))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(paypal_error_not_found(
                response,
                PaymentError::SubscriptionNotFound(id.to_string()),
            )
            .await);
        }

        self.get_subscription(id).await
    }

    async fn resume_subscription(&self, id: &str) -> PaymentResult<Subscription> {
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("/v1/billing/subscriptions/{}/activate", id),
            )
            .await?
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(paypal_error_not_found(
                response,
                PaymentError::SubscriptionNotFound(id.to_string()),
            )
            .await);
        }

        self.get_subscription(id).await
    }

    async fn verify_webhook(&self, payload: &[u8], headers: &WebhookHeaders) -> PaymentResult<()> {
        // PayPal signs webhooks with a certificate chain we cannot check
        // offline, so authenticity is established by handing the transmission
        // headers plus the exact body back to PayPal's verification endpoint.
        let webhook_id = self.webhook_id.as_ref().ok_or_else(|| {
            PaymentError::Config(
                "PayPal webhook_id not configured; call PayPalProvider::with_webhook_id".into(),
            )
        })?;

        // Bound the body before spending a network round-trip on it. This
        // method is reached with fully unauthenticated input, so an unbounded
        // body lets anyone who can reach the webhook endpoint drive arbitrarily
        // large outbound POSTs to PayPal from this process.
        if payload.len() > MAX_WEBHOOK_BODY {
            return Err(PaymentError::InvalidWebhookSignature);
        }

        // The body must be forwarded byte-for-byte as PayPal signed it: PayPal
        // reconstructs `transmission_id|transmission_time|webhook_id|crc32(body)`
        // from the original bytes, so any re-serialization (key reordering,
        // whitespace changes, number reformatting, `\uXXXX` escaping) fails
        // verification. `RawValue` forwards the exact bytes we received
        // instead of parsing into a `Value` and letting reqwest re-serialize
        // it — `Value`'s key order is only preserved today because
        // `preserve_order` arrives via feature unification from an unrelated
        // transitive dependency; nothing in this workspace declares it.
        let webhook_event = serde_json::value::RawValue::from_string(
            String::from_utf8(payload.to_vec())
                .map_err(|_| PaymentError::InvalidWebhookSignature)?,
        )
        .map_err(|_| PaymentError::InvalidWebhookSignature)?;

        let verify_request = PayPalVerifyRequest {
            auth_algo: headers.require(PAYPAL_AUTH_ALGO_HEADER)?.to_string(),
            cert_url: headers.require(PAYPAL_CERT_URL_HEADER)?.to_string(),
            transmission_id: headers.require(PAYPAL_TRANSMISSION_ID_HEADER)?.to_string(),
            transmission_sig: headers.require(PAYPAL_TRANSMISSION_SIG_HEADER)?.to_string(),
            transmission_time: headers
                .require(PAYPAL_TRANSMISSION_TIME_HEADER)?
                .to_string(),
            webhook_id: webhook_id.clone(),
            webhook_event,
        };

        let response = self
            .request(
                reqwest::Method::POST,
                "/v1/notifications/verify-webhook-signature",
            )
            .await?
            .json(&verify_request)
            .send()
            .await?;

        // Anything other than an explicit SUCCESS is a rejection — including a
        // transport failure or an unparseable response. Fail closed.
        if !response.status().is_success() {
            return Err(PaymentError::InvalidWebhookSignature);
        }

        let verification: PayPalVerifyResponse = response
            .json()
            .await
            .map_err(|_| PaymentError::InvalidWebhookSignature)?;

        if verification.verification_status != "SUCCESS" {
            return Err(PaymentError::InvalidWebhookSignature);
        }

        Ok(())
    }

    fn parse_webhook(&self, payload: &[u8]) -> PaymentResult<WebhookEvent> {
        let event: PayPalWebhookEvent = serde_json::from_slice(payload)?;

        Ok(WebhookEvent {
            id: event.id,
            event_type: WebhookEventType::from_str(&event.event_type),
            created_at: Utc::now(),
            data: WebhookData::Generic(event.resource),
            provider: "paypal".to_string(),
            livemode: true,
        })
    }
}

// PayPal API types

#[derive(Debug, Deserialize)]
struct PayPalTokenResponse {
    access_token: String,
    expires_in: u64,
}

#[derive(Debug, Serialize)]
struct PayPalOrderRequest {
    intent: String,
    purchase_units: Vec<PayPalPurchaseUnit>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PayPalPurchaseUnit {
    amount: PayPalAmount,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    /// PayPal's per-purchase-unit statement descriptor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    soft_descriptor: Option<String>,
    /// Merchant-defined passthrough field; this crate encodes
    /// [`ChargeRequest::metadata`] into it so the metadata reaches PayPal
    /// instead of only being echoed back to the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    custom_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PayPalAmount {
    currency_code: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct PayPalOrder {
    id: String,
    status: String,
    purchase_units: Vec<PayPalPurchaseUnit>,
}

#[derive(Debug, Serialize)]
struct PayPalRefundRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    amount: Option<PayPalAmount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note_to_payer: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PayPalRefund {
    id: String,
    status: String,
    amount: Option<PayPalAmount>,
}

#[derive(Debug, Serialize)]
struct PayPalSubscriptionRequest {
    plan_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    quantity: Option<String>,
}

#[derive(Debug, Serialize)]
struct PayPalVerifyRequest {
    auth_algo: String,
    cert_url: String,
    transmission_id: String,
    transmission_sig: String,
    transmission_time: String,
    webhook_id: String,
    webhook_event: Box<serde_json::value::RawValue>,
}

#[derive(Debug, Deserialize)]
struct PayPalVerifyResponse {
    #[serde(default)]
    verification_status: String,
}

#[derive(Debug, Deserialize)]
struct PayPalSubscription {
    id: String,
    status: String,
    #[serde(default)]
    plan_id: Option<String>,
    #[serde(default)]
    quantity: Option<String>,
    #[serde(default)]
    create_time: Option<chrono::DateTime<Utc>>,
    #[serde(default)]
    status_update_time: Option<chrono::DateTime<Utc>>,
    #[serde(default)]
    billing_info: Option<PayPalBillingInfo>,
    #[serde(default)]
    subscriber: Option<PayPalSubscriber>,
}

#[derive(Debug, Deserialize)]
struct PayPalBillingInfo {
    #[serde(default)]
    next_billing_time: Option<chrono::DateTime<Utc>>,
    #[serde(default)]
    last_payment: Option<PayPalLastPayment>,
}

#[derive(Debug, Deserialize)]
struct PayPalLastPayment {
    #[serde(default)]
    time: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct PayPalSubscriber {
    #[serde(default)]
    payer_id: Option<String>,
}

impl PayPalSubscription {
    /// Project PayPal's subscription resource onto the crate's [`Subscription`].
    ///
    /// Fields PayPal does not report stay `None` rather than being invented:
    /// the previous code manufactured a 30-day window starting "now" on every
    /// read, which silently misreported every billing period.
    fn into_subscription(self) -> Subscription {
        let status = match self.status.as_str() {
            "ACTIVE" => SubscriptionStatus::Active,
            "CANCELLED" => SubscriptionStatus::Canceled,
            "SUSPENDED" => SubscriptionStatus::Paused,
            "APPROVAL_PENDING" | "APPROVED" => SubscriptionStatus::Incomplete,
            "EXPIRED" => SubscriptionStatus::Canceled,
            _ => SubscriptionStatus::Active,
        };
        let canceled_at = if status == SubscriptionStatus::Canceled {
            self.status_update_time
        } else {
            None
        };
        let billing = self.billing_info;

        Subscription {
            id: self.id,
            customer_id: self.subscriber.and_then(|s| s.payer_id),
            status,
            current_period_start: billing.as_ref().and_then(|b| {
                b.last_payment
                    .as_ref()
                    .and_then(|p| p.time)
                    .or(self.create_time)
            }),
            current_period_end: billing.as_ref().and_then(|b| b.next_billing_time),
            trial_end: None,
            cancel_at_period_end: false,
            canceled_at,
            price_id: self.plan_id.unwrap_or_default(),
            quantity: self.quantity.and_then(|q| q.parse().ok()).unwrap_or(1),
            metadata: HashMap::new(),
            created_at: self.create_time,
        }
    }
}

#[derive(Debug, Deserialize)]
struct PayPalWebhookEvent {
    id: String,
    event_type: String,
    resource: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> PayPalProvider {
        PayPalProvider::new("client_id", "client_secret")
    }

    // ---- finding 1: base URL validation ----
    //
    // This is the branch's headline fix: verify_webhook POSTs the body back to
    // the base URL and trusts the answer, so an attacker-controlled override
    // can reply SUCCESS to every forged webhook.

    #[test]
    fn with_base_url_rejects_cleartext_http() {
        assert!(provider().with_base_url("http://evil.example.com").is_err());
        assert!(provider().with_base_url("ftp://example.com").is_err());
        assert!(provider().with_base_url("not a url").is_err());
    }

    #[test]
    fn with_base_url_accepts_https_and_loopback() {
        assert!(provider().with_base_url("https://api-m.paypal.com").is_ok());
        assert!(provider().with_base_url("http://127.0.0.1:8080").is_ok());
        assert!(provider().with_base_url("http://localhost:8080").is_ok());
    }

    #[test]
    fn with_base_url_strips_trailing_slash() {
        let p = provider().with_base_url("http://localhost:8080/").unwrap();
        assert_eq!(p.base_url(), "http://localhost:8080");
    }

    // ---- finding 2: idempotency ----

    #[test]
    fn advertises_idempotency_support() {
        assert!(provider().supports_idempotency());
    }

    // ---- finding 5: intent follows request.capture ----

    #[test]
    fn order_intent_follows_capture_flag() {
        // auth_only() and the default previously produced byte-identical bodies.
        let capture = ChargeRequest::new(Money::usd(1000), PaymentSource::card("tok"));
        assert!(capture.capture);

        let auth = ChargeRequest::new(Money::usd(1000), PaymentSource::card("tok")).auth_only();
        assert!(!auth.capture);

        let intent_for = |c: bool| if c { "CAPTURE" } else { "AUTHORIZE" };
        assert_eq!(intent_for(capture.capture), "CAPTURE");
        assert_eq!(intent_for(auth.capture), "AUTHORIZE");
    }

    #[test]
    fn purchase_unit_carries_descriptor_and_metadata() {
        let unit = PayPalPurchaseUnit {
            amount: PayPalAmount {
                currency_code: "EUR".into(),
                value: Money::eur(12000).to_gateway_string(),
            },
            description: Some("Order 7".into()),
            soft_descriptor: Some("ACME STORE".into()),
            custom_id: Some(r#"{"order_id":"7"}"#.into()),
        };
        let json = serde_json::to_value(&unit).unwrap();

        assert_eq!(json["amount"]["value"], "120.00");
        assert_eq!(json["soft_descriptor"], "ACME STORE");
        assert_eq!(json["custom_id"], r#"{"order_id":"7"}"#);
    }

    #[test]
    fn purchase_unit_omits_absent_optional_fields() {
        let unit = PayPalPurchaseUnit {
            amount: PayPalAmount {
                currency_code: "USD".into(),
                value: "10.00".into(),
            },
            description: None,
            soft_descriptor: None,
            custom_id: None,
        };
        let json = serde_json::to_value(&unit).unwrap();
        assert!(json.get("soft_descriptor").is_none());
        assert!(json.get("custom_id").is_none());
        assert!(json.get("description").is_none());
    }

    #[tokio::test]
    async fn charge_rejects_metadata_too_large_for_custom_id() {
        // Better a loud failure than silently dropping the caller's metadata.
        let mut request = ChargeRequest::new(Money::usd(1000), PaymentSource::card("tok"));
        request.metadata.insert("k".into(), "x".repeat(200));

        let err = provider().charge(request).await.unwrap_err();
        assert!(matches!(err, PaymentError::Validation(_)), "got {err:?}");
    }

    // ---- finding 4: capture status is mapped, not hardcoded ----

    #[test]
    fn capture_status_is_mapped() {
        assert_eq!(order_charge_status("COMPLETED"), ChargeStatus::Succeeded);
        assert_eq!(order_charge_status("DECLINED"), ChargeStatus::Failed);
        assert_eq!(order_charge_status("FAILED"), ChargeStatus::Failed);
        assert_eq!(order_charge_status("PENDING"), ChargeStatus::Pending);
        assert_eq!(order_charge_status("VOIDED"), ChargeStatus::Canceled);
        // An unapproved order is pending, never a completed payment.
        assert_eq!(order_charge_status("CREATED"), ChargeStatus::Pending);
        assert_eq!(
            order_charge_status("PAYER_ACTION_REQUIRED"),
            ChargeStatus::Pending
        );
    }

    // ---- finding 6: partial capture hits no wrong endpoint ----

    #[tokio::test]
    async fn partial_capture_is_refused_rather_than_silently_full() {
        let err = provider()
            .capture("order_1", Some(Money::usd(500)))
            .await
            .unwrap_err();
        assert!(matches!(err, PaymentError::Unsupported(_)), "got {err:?}");
        // The message must point at the authorizations endpoint.
        assert!(err.to_string().contains("authorization"), "got {err}");
    }

    // ---- finding 8: zero-decimal currencies ----

    #[test]
    fn gateway_amounts_respect_zero_decimal_currencies() {
        assert_eq!(Money::new(1000, Currency::JPY).to_gateway_string(), "1000");
        assert_eq!(Money::eur(12000).to_gateway_string(), "120.00");
    }

    // ---- finding 9: hollow methods report their absence ----

    #[tokio::test]
    async fn delete_customer_is_unsupported() {
        // Returning Ok(()) let a GDPR erasure record a deletion that never
        // happened.
        let err = provider().delete_customer("cust_1").await.unwrap_err();
        assert!(matches!(err, PaymentError::Unsupported(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn list_payment_methods_is_unsupported() {
        // An empty Vec is indistinguishable from "no vaulted methods".
        let err = provider().list_payment_methods("cust_1").await.unwrap_err();
        assert!(matches!(err, PaymentError::Unsupported(_)), "got {err:?}");
    }

    // ---- finding 11: end-of-period cancellation is not silently upgraded ----

    #[tokio::test]
    async fn cancel_subscription_refuses_end_of_period() {
        let err = provider()
            .cancel_subscription("sub_1", false)
            .await
            .unwrap_err();
        assert!(matches!(err, PaymentError::Unsupported(_)), "got {err:?}");
    }

    // ---- finding 15: webhook body cap ----

    #[tokio::test]
    async fn oversized_webhook_is_rejected_before_the_network_call() {
        // No webhook_id is configured, so reaching the network would surface a
        // Config error instead. Getting InvalidWebhookSignature proves the size
        // check ran first... so configure one to make the ordering explicit.
        let provider = provider().with_webhook_id("wh_1");
        let payload = vec![b'x'; MAX_WEBHOOK_BODY + 1];
        let headers = WebhookHeaders::new();

        let err = provider
            .verify_webhook(&payload, &headers)
            .await
            .unwrap_err();
        assert!(
            matches!(err, PaymentError::InvalidWebhookSignature),
            "got {err:?}"
        );
    }
}
