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
            client: Client::new(),
            access_token: tokio::sync::RwLock::new(None),
        }
    }

    /// Use production environment
    pub fn production(mut self) -> Self {
        self.sandbox = false;
        self
    }

    /// Point the client at an alternate API base URL (a mock or proxy).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url_override = Some(base_url.into());
        self
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
        // Check if we have a valid token
        {
            let token = self.access_token.read().await;
            if let Some(ref t) = *token
                && t.expires_at > Utc::now()
            {
                return Ok(t.token.clone());
            }
        }

        // Get new token
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
            return Err(PaymentError::Authentication(
                "Failed to get PayPal token".into(),
            ));
        }

        let token_response: PayPalTokenResponse = response.json().await?;
        let new_token = PayPalToken {
            token: token_response.access_token.clone(),
            expires_at: Utc::now()
                + chrono::Duration::seconds(token_response.expires_in as i64 - 60),
        };

        let mut token = self.access_token.write().await;
        *token = Some(new_token);

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

#[async_trait]
impl PaymentProvider for PayPalProvider {
    fn name(&self) -> &'static str {
        "paypal"
    }

    async fn charge(&self, request: ChargeRequest) -> PaymentResult<Charge> {
        // PayPal uses Orders API for charges
        let order_request = PayPalOrderRequest {
            intent: "CAPTURE".to_string(),
            purchase_units: vec![PayPalPurchaseUnit {
                amount: PayPalAmount {
                    currency_code: request.amount.currency.code().to_string(),
                    value: format!("{:.2}", request.amount.to_float()),
                },
                description: request.description.clone(),
            }],
        };

        let response = self
            .request(reqwest::Method::POST, "/v2/checkout/orders")
            .await?
            .json(&order_request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error: PayPalError = response.json().await?;
            return Err(PaymentError::Provider(error.message.unwrap_or_default()));
        }

        let order: PayPalOrder = response.json().await?;

        // For captured orders, return as charge
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
            amount_refunded: Money::new(0, request.amount.currency),
            status: match order.status.as_str() {
                "COMPLETED" | "CAPTURED" => ChargeStatus::Succeeded,
                "VOIDED" => ChargeStatus::Canceled,
                _ => ChargeStatus::Pending,
            },
            customer_id: None,
            payment_method: None,
            description: request.description,
            receipt_url: None,
            failure_reason: None,
            metadata: request.metadata,
            created_at: Utc::now(),
            captured: order.status == "COMPLETED",
            refunded: false,
            disputed: false,
        })
    }

    async fn capture(&self, charge_id: &str, amount: Option<Money>) -> PaymentResult<Charge> {
        // A partial capture must actually send the amount; omitting it captures
        // the full authorization.
        let body = PayPalCaptureRequest {
            amount: amount.map(|a| PayPalAmount {
                currency_code: a.currency.code().to_string(),
                value: format!("{:.2}", a.to_float()),
            }),
        };

        let response = self
            .request(
                reqwest::Method::POST,
                &format!("/v2/checkout/orders/{}/capture", charge_id),
            )
            .await?
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error: PayPalError = response.json().await?;
            return Err(PaymentError::Provider(error.message.unwrap_or_default()));
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
            .unwrap_or(Money::usd(0));

        Ok(Charge {
            id: order.id,
            amount,
            amount_refunded: Money::new(0, Currency::USD),
            status: ChargeStatus::Succeeded,
            customer_id: None,
            payment_method: None,
            description: None,
            receipt_url: None,
            failure_reason: None,
            metadata: HashMap::new(),
            created_at: Utc::now(),
            captured: true,
            refunded: false,
            disputed: false,
        })
    }

    async fn refund(&self, request: RefundRequest) -> PaymentResult<Refund> {
        // PayPal requires the capture ID for refunds
        let refund_request = PayPalRefundRequest {
            amount: request.amount.map(|a| PayPalAmount {
                currency_code: a.currency.code().to_string(),
                value: format!("{:.2}", a.to_float()),
            }),
            note_to_payer: None,
        };

        let response = self
            .request(
                reqwest::Method::POST,
                &format!("/v2/payments/captures/{}/refund", request.charge_id),
            )
            .await?
            .json(&refund_request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error: PayPalError = response.json().await?;
            return Err(PaymentError::Provider(error.message.unwrap_or_default()));
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
        Ok(()) // No-op for PayPal
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
        Ok(Vec::new())
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
            let error: PayPalError = response.json().await?;
            return Err(PaymentError::Provider(error.message.unwrap_or_default()));
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
            return Err(PaymentError::SubscriptionNotFound(id.to_string()));
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
            let error: PayPalError = response.json().await?;
            return Err(PaymentError::Provider(error.message.unwrap_or_default()));
        }

        self.get_subscription(id).await
    }

    async fn cancel_subscription(&self, id: &str, _immediate: bool) -> PaymentResult<Subscription> {
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
            let error: PayPalError = response.json().await?;
            return Err(PaymentError::Provider(error.message.unwrap_or_default()));
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
            let error: PayPalError = response.json().await?;
            return Err(PaymentError::Provider(error.message.unwrap_or_default()));
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

#[derive(Debug, Deserialize)]
struct PayPalError {
    #[allow(dead_code)]
    name: Option<String>,
    message: Option<String>,
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
struct PayPalCaptureRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    amount: Option<PayPalAmount>,
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
