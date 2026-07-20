//! Stripe payment provider implementation

use crate::{
    error::{DeclineCode, PaymentError, PaymentResult},
    money::{Currency, Money},
    provider::{PaymentProvider, ProviderClient},
    types::*,
    webhook::{WebhookData, WebhookEvent, WebhookEventType, WebhookHeaders},
};
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use hmac::{Hmac, KeyInit, Mac};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use sha2::Sha256;
use std::collections::HashMap;

/// The header Stripe signs each webhook with.
pub const STRIPE_SIGNATURE_HEADER: &str = "stripe-signature";

/// Maximum number of `v1=` signature candidates accepted from a single
/// `Stripe-Signature` header. Stripe sends at most two (old and new) during a
/// secret rotation; each candidate costs a fresh HMAC-SHA256 over the entire
/// (attacker-controlled, pre-verification) payload, so an unbounded count is
/// a CPU-amplification vector.
const MAX_V1_CANDIDATES: usize = 8;

/// Stripe provider
pub struct StripeProvider {
    #[allow(dead_code)]
    api_key: SecretString,
    webhook_secret: Option<SecretString>,
    webhook_tolerance: Option<chrono::Duration>,
    client: ProviderClient,
}

impl StripeProvider {
    /// Create a new Stripe provider
    pub fn new(api_key: impl Into<String>) -> Self {
        let api_key = api_key.into();
        Self {
            client: ProviderClient::new("https://api.stripe.com/v1", &api_key),
            api_key: SecretString::new(api_key.into()),
            webhook_secret: None,
            webhook_tolerance: Some(chrono::Duration::minutes(5)),
        }
    }

    /// Point the client at an alternate API base URL (a mock or proxy).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.client = ProviderClient::new(base_url, self.api_key.expose_secret());
        self
    }

    /// Set webhook secret
    pub fn with_webhook_secret(mut self, secret: impl Into<String>) -> Self {
        self.webhook_secret = Some(SecretString::new(secret.into().into()));
        self
    }

    /// Maximum age a signed webhook timestamp may have before it is rejected as
    /// a replay. Defaults to five minutes; `None` disables the check.
    pub fn with_webhook_tolerance(mut self, tolerance: Option<chrono::Duration>) -> Self {
        self.webhook_tolerance = tolerance;
        self
    }

    /// Create a payment intent
    pub async fn create_payment_intent(
        &self,
        request: ChargeRequest,
    ) -> PaymentResult<StripePaymentIntent> {
        let mut params: HashMap<String, String> = HashMap::new();
        params.insert("amount".into(), request.amount.amount.to_string());
        params.insert(
            "currency".into(),
            request.amount.currency.code().to_lowercase(),
        );

        if let Some(desc) = &request.description {
            params.insert("description".into(), desc.clone());
        }

        if let Some(customer_id) = &request.customer_id {
            params.insert("customer".into(), customer_id.clone());
        }
        if let Some(descriptor) = &request.statement_descriptor {
            params.insert("statement_descriptor".into(), descriptor.clone());
        }
        for (key, value) in &request.metadata {
            params.insert(format!("metadata[{key}]"), value.clone());
        }

        match &request.source {
            PaymentSource::PaymentMethod { id } => {
                params.insert("payment_method".into(), id.clone());
                params.insert("confirm".into(), "true".to_string());
                if !request.capture {
                    params.insert("capture_method".into(), "manual".to_string());
                }
            }
            PaymentSource::Customer { customer_id } => {
                params.insert("customer".into(), customer_id.clone());
            }
            PaymentSource::Card { .. } => {
                return Err(PaymentError::Validation(
                    "Stripe PaymentIntents do not accept raw card tokens; tokenize the card into a \
                     PaymentMethod first and use PaymentSource::PaymentMethod"
                        .into(),
                ));
            }
            PaymentSource::Bank { .. } => {
                return Err(PaymentError::Validation(
                    "Stripe PaymentIntents do not accept bank-account tokens directly; create a \
                     us_bank_account PaymentMethod and use PaymentSource::PaymentMethod"
                        .into(),
                ));
            }
        }

        let response = self
            .client
            .post_form_idempotent(
                "/payment_intents",
                &params,
                request.idempotency_key.as_deref(),
            )
            .await?;
        stripe_json(response).await
    }
}

/// Decode a Stripe response body, mapping any non-2xx status onto the error
/// Stripe actually reported rather than a downstream deserialization failure.
async fn stripe_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> PaymentResult<T> {
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(stripe_error(status, &body));
    }
    serde_json::from_str(&body)
        .map_err(|e| PaymentError::Serialization(format!("{e} (body: {body})")))
}

/// Check a Stripe response that carries no useful body.
async fn stripe_unit(response: reqwest::Response) -> PaymentResult<()> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(stripe_error(status, &body));
    }
    Ok(())
}

/// Map a Stripe error body + HTTP status onto a typed [`PaymentError`].
fn stripe_error(status: reqwest::StatusCode, body: &str) -> PaymentError {
    let detail = serde_json::from_str::<StripeError>(body)
        .ok()
        .map(|e| e.error);

    if let Some(detail) = detail {
        if let Some(code) = detail.decline_code.as_deref().or(detail.code.as_deref()) {
            match DeclineCode::from_str(code) {
                DeclineCode::ExpiredCard => return PaymentError::CardExpired,
                DeclineCode::InsufficientFunds => return PaymentError::InsufficientFunds,
                _ => {}
            }
        }
        return match detail.error_type.as_deref() {
            Some("card_error") => PaymentError::CardDeclined(detail.message),
            Some("authentication_error") => PaymentError::Authentication(detail.message),
            Some("invalid_request_error") if status == reqwest::StatusCode::NOT_FOUND => {
                PaymentError::Provider(detail.message)
            }
            Some("rate_limit_error") => PaymentError::RateLimited(1),
            _ if status == reqwest::StatusCode::TOO_MANY_REQUESTS => PaymentError::RateLimited(1),
            _ if status == reqwest::StatusCode::UNAUTHORIZED => {
                PaymentError::Authentication(detail.message)
            }
            _ => PaymentError::Provider(detail.message),
        };
    }

    match status {
        reqwest::StatusCode::TOO_MANY_REQUESTS => PaymentError::RateLimited(1),
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
            PaymentError::Authentication(format!("Stripe returned {status}: {body}"))
        }
        _ => PaymentError::Provider(format!("Stripe returned {status}: {body}")),
    }
}

#[async_trait]
impl PaymentProvider for StripeProvider {
    fn name(&self) -> &'static str {
        "stripe"
    }

    async fn charge(&self, request: ChargeRequest) -> PaymentResult<Charge> {
        // A PaymentMethod source cannot be charged through the legacy Charges
        // API; route it through PaymentIntents rather than dropping it.
        if matches!(request.source, PaymentSource::PaymentMethod { .. }) {
            let amount = request.amount;
            let description = request.description.clone();
            let metadata = request.metadata.clone();
            let intent = self.create_payment_intent(request).await?;
            return Ok(charge_from_intent(intent, amount, description, metadata));
        }

        let mut params: HashMap<String, String> = HashMap::new();
        params.insert("amount".into(), request.amount.amount.to_string());
        params.insert(
            "currency".into(),
            request.amount.currency.code().to_lowercase(),
        );

        if let Some(desc) = &request.description {
            params.insert("description".into(), desc.clone());
        }

        if let Some(customer_id) = &request.customer_id {
            params.insert("customer".into(), customer_id.clone());
        }

        if let Some(descriptor) = &request.statement_descriptor {
            params.insert("statement_descriptor".into(), descriptor.clone());
        }

        for (key, value) in &request.metadata {
            params.insert(format!("metadata[{key}]"), value.clone());
        }

        match &request.source {
            // Both card and bank-account tokens are `source` on the Charges API.
            PaymentSource::Card { token } | PaymentSource::Bank { token } => {
                params.insert("source".into(), token.clone());
            }
            PaymentSource::Customer { customer_id } => {
                params.insert("customer".into(), customer_id.clone());
            }
            PaymentSource::PaymentMethod { .. } => unreachable!("handled above"),
        }

        params.insert("capture".into(), request.capture.to_string());

        let response = self
            .client
            .post_form_idempotent("/charges", &params, request.idempotency_key.as_deref())
            .await?;

        let stripe_charge: StripeCharge = stripe_json(response).await?;
        Ok(stripe_charge.into())
    }

    async fn capture(&self, charge_id: &str, amount: Option<Money>) -> PaymentResult<Charge> {
        let mut params = HashMap::new();
        if let Some(amt) = amount {
            params.insert("amount", amt.amount.to_string());
        }

        let response = self
            .client
            .post_form(&format!("/charges/{}/capture", charge_id), &params)
            .await?;

        let stripe_charge: StripeCharge = stripe_json(response).await?;
        Ok(stripe_charge.into())
    }

    async fn refund(&self, request: RefundRequest) -> PaymentResult<Refund> {
        let mut params = HashMap::new();
        params.insert("charge", request.charge_id.clone());

        if let Some(amount) = &request.amount {
            params.insert("amount", amount.amount.to_string());
        }

        if let Some(reason) = &request.reason {
            params.insert(
                "reason",
                match reason {
                    RefundReason::Duplicate => "duplicate",
                    RefundReason::Fraudulent => "fraudulent",
                    RefundReason::RequestedByCustomer => "requested_by_customer",
                }
                .to_string(),
            );
        }

        let response = self.client.post_form("/refunds", &params).await?;
        let stripe_refund: StripeRefund = stripe_json(response).await?;
        Ok(stripe_refund.into())
    }

    async fn create_customer(&self, request: CreateCustomerRequest) -> PaymentResult<Customer> {
        let mut params = HashMap::new();

        if let Some(email) = &request.email {
            params.insert("email", email.clone());
        }
        if let Some(name) = &request.name {
            params.insert("name", name.clone());
        }
        if let Some(phone) = &request.phone {
            params.insert("phone", phone.clone());
        }
        if let Some(desc) = &request.description {
            params.insert("description", desc.clone());
        }

        let response = self.client.post_form("/customers", &params).await?;
        let stripe_customer: StripeCustomer = stripe_json(response).await?;
        Ok(stripe_customer.into())
    }

    async fn get_customer(&self, id: &str) -> PaymentResult<Customer> {
        let response = self.client.get(&format!("/customers/{}", id)).await?;
        let stripe_customer: StripeCustomer = stripe_json(response).await?;
        Ok(stripe_customer.into())
    }

    async fn update_customer(
        &self,
        id: &str,
        request: UpdateCustomerRequest,
    ) -> PaymentResult<Customer> {
        let mut params = HashMap::new();

        if let Some(email) = &request.email {
            params.insert("email", email.clone());
        }
        if let Some(name) = &request.name {
            params.insert("name", name.clone());
        }
        if let Some(phone) = &request.phone {
            params.insert("phone", phone.clone());
        }

        let response = self
            .client
            .post_form(&format!("/customers/{}", id), &params)
            .await?;
        let stripe_customer: StripeCustomer = stripe_json(response).await?;
        Ok(stripe_customer.into())
    }

    async fn delete_customer(&self, id: &str) -> PaymentResult<()> {
        let response = self.client.delete(&format!("/customers/{}", id)).await?;
        stripe_unit(response).await
    }

    async fn create_payment_method(
        &self,
        request: CreatePaymentMethodRequest,
    ) -> PaymentResult<PaymentMethod> {
        let mut params = HashMap::new();
        params.insert("type", "card".to_string());

        if let Some(card) = &request.card {
            params.insert("card[number]", card.number.clone());
            params.insert("card[exp_month]", card.exp_month.to_string());
            params.insert("card[exp_year]", card.exp_year.to_string());
            params.insert("card[cvc]", card.cvc.clone());
        }

        let response = self.client.post_form("/payment_methods", &params).await?;
        let stripe_pm: StripePaymentMethod = stripe_json(response).await?;
        Ok(stripe_pm.into())
    }

    async fn attach_payment_method(
        &self,
        method_id: &str,
        customer_id: &str,
    ) -> PaymentResult<PaymentMethod> {
        let mut params = HashMap::new();
        params.insert("customer", customer_id.to_string());

        let response = self
            .client
            .post_form(&format!("/payment_methods/{}/attach", method_id), &params)
            .await?;
        let stripe_pm: StripePaymentMethod = stripe_json(response).await?;
        Ok(stripe_pm.into())
    }

    async fn detach_payment_method(&self, method_id: &str) -> PaymentResult<PaymentMethod> {
        let response = self
            .client
            .post_form(
                &format!("/payment_methods/{}/detach", method_id),
                &HashMap::<String, String>::new(),
            )
            .await?;
        let stripe_pm: StripePaymentMethod = stripe_json(response).await?;
        Ok(stripe_pm.into())
    }

    async fn list_payment_methods(&self, customer_id: &str) -> PaymentResult<Vec<PaymentMethod>> {
        let response = self
            .client
            .get(&format!(
                "/payment_methods?customer={}&type=card",
                customer_id
            ))
            .await?;
        let list: StripeList<StripePaymentMethod> = stripe_json(response).await?;
        Ok(list.data.into_iter().map(Into::into).collect())
    }

    async fn create_subscription(
        &self,
        request: CreateSubscriptionRequest,
    ) -> PaymentResult<Subscription> {
        let mut params = HashMap::new();
        params.insert("customer", request.customer_id.clone());
        params.insert("items[0][price]", request.price_id.clone());

        if let Some(qty) = request.quantity {
            params.insert("items[0][quantity]", qty.to_string());
        }

        if let Some(days) = request.trial_days {
            params.insert("trial_period_days", days.to_string());
        }

        if let Some(pm) = &request.payment_method {
            params.insert("default_payment_method", pm.clone());
        }

        let response = self.client.post_form("/subscriptions", &params).await?;
        let stripe_sub: StripeSubscription = stripe_json(response).await?;
        Ok(stripe_sub.into())
    }

    async fn get_subscription(&self, id: &str) -> PaymentResult<Subscription> {
        let response = self.client.get(&format!("/subscriptions/{}", id)).await?;
        let stripe_sub: StripeSubscription = stripe_json(response).await?;
        Ok(stripe_sub.into())
    }

    async fn update_subscription(&self, id: &str, price_id: &str) -> PaymentResult<Subscription> {
        let mut params = HashMap::new();
        params.insert("items[0][price]", price_id.to_string());

        let response = self
            .client
            .post_form(&format!("/subscriptions/{}", id), &params)
            .await?;
        let stripe_sub: StripeSubscription = stripe_json(response).await?;
        Ok(stripe_sub.into())
    }

    async fn cancel_subscription(&self, id: &str, immediate: bool) -> PaymentResult<Subscription> {
        if immediate {
            let response = self
                .client
                .delete(&format!("/subscriptions/{}", id))
                .await?;
            let stripe_sub: StripeSubscription = stripe_json(response).await?;
            Ok(stripe_sub.into())
        } else {
            let mut params = HashMap::new();
            params.insert("cancel_at_period_end", "true".to_string());

            let response = self
                .client
                .post_form(&format!("/subscriptions/{}", id), &params)
                .await?;
            let stripe_sub: StripeSubscription = stripe_json(response).await?;
            Ok(stripe_sub.into())
        }
    }

    async fn resume_subscription(&self, id: &str) -> PaymentResult<Subscription> {
        let mut params = HashMap::new();
        params.insert("cancel_at_period_end", "false".to_string());

        let response = self
            .client
            .post_form(&format!("/subscriptions/{}", id), &params)
            .await?;
        let stripe_sub: StripeSubscription = stripe_json(response).await?;
        Ok(stripe_sub.into())
    }

    async fn verify_webhook(&self, payload: &[u8], headers: &WebhookHeaders) -> PaymentResult<()> {
        let secret = self
            .webhook_secret
            .as_ref()
            .ok_or(PaymentError::Config("Webhook secret not configured".into()))?;
        // An empty secret would HMAC every payload with a zero-length key,
        // which is not a real secret at all — reject it the same way an
        // absent secret is rejected, rather than letting any signature over
        // an empty-string-keyed HMAC verify.
        if secret.expose_secret().is_empty() {
            return Err(PaymentError::Config(
                "Webhook secret must not be empty".into(),
            ));
        }

        let signature = headers.require(STRIPE_SIGNATURE_HEADER)?;

        // Parse the Stripe-Signature header: `t=<unix>,v1=<hex>[,v1=<hex>...]`.
        // Stripe may send several v1 signatures during a secret rotation, so
        // every candidate is checked.
        let mut timestamp: Option<&str> = None;
        let mut candidates: Vec<&str> = Vec::new();
        for part in signature.split(',') {
            match part.trim().split_once('=') {
                Some(("t", v)) => timestamp = Some(v),
                Some(("v1", v)) => candidates.push(v),
                _ => {}
            }
        }

        let timestamp = timestamp.ok_or(PaymentError::InvalidWebhookSignature)?;
        if candidates.is_empty() {
            return Err(PaymentError::InvalidWebhookSignature);
        }
        if candidates.len() > MAX_V1_CANDIDATES {
            return Err(PaymentError::InvalidWebhookSignature);
        }

        // Reject stale timestamps: without this, a captured webhook can be
        // replayed forever with its original (still valid) signature.
        if let Some(tolerance) = self.webhook_tolerance {
            let signed_at: i64 = timestamp
                .parse()
                .map_err(|_| PaymentError::InvalidWebhookSignature)?;
            let age = Utc::now().timestamp() - signed_at;
            if age.abs() > tolerance.num_seconds() {
                return Err(PaymentError::InvalidWebhookSignature);
            }
        }

        let mut signed_payload = Vec::with_capacity(timestamp.len() + 1 + payload.len());
        signed_payload.extend_from_slice(timestamp.as_bytes());
        signed_payload.push(b'.');
        signed_payload.extend_from_slice(payload);

        for candidate in candidates {
            let Ok(expected) = hex::decode(candidate) else {
                continue;
            };
            let mut mac = Hmac::<Sha256>::new_from_slice(secret.expose_secret().as_bytes())
                .map_err(|_| PaymentError::InvalidWebhookSignature)?;
            mac.update(&signed_payload);
            // `verify_slice` compares in constant time; a byte-wise `!=` on the
            // hex string leaks the shared secret one byte at a time under a
            // timing attack.
            if mac.verify_slice(&expected).is_ok() {
                return Ok(());
            }
        }

        Err(PaymentError::InvalidWebhookSignature)
    }

    fn parse_webhook(&self, payload: &[u8]) -> PaymentResult<WebhookEvent> {
        let event: StripeWebhookEvent = serde_json::from_slice(payload)?;

        Ok(WebhookEvent {
            id: event.id,
            event_type: WebhookEventType::from_str(&event.event_type),
            created_at: Utc.timestamp_opt(event.created, 0).unwrap(),
            data: WebhookData::Generic(event.data.object),
            provider: "stripe".to_string(),
            livemode: event.livemode,
        })
    }
}

// Stripe API types

#[derive(Debug, Deserialize)]
struct StripeError {
    error: StripeErrorDetail,
}

#[derive(Debug, Deserialize)]
struct StripeErrorDetail {
    #[serde(default)]
    message: String,
    #[serde(rename = "type")]
    error_type: Option<String>,
    code: Option<String>,
    decline_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StripeList<T> {
    data: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct StripeCharge {
    id: String,
    amount: i64,
    currency: String,
    status: String,
    customer: Option<String>,
    payment_method: Option<String>,
    description: Option<String>,
    receipt_url: Option<String>,
    failure_message: Option<String>,
    captured: bool,
    refunded: bool,
    disputed: bool,
    created: i64,
    #[serde(default)]
    metadata: HashMap<String, String>,
    amount_refunded: Option<i64>,
}

impl From<StripeCharge> for Charge {
    fn from(sc: StripeCharge) -> Self {
        let currency = Currency::from_code(&sc.currency).unwrap_or(Currency::USD);
        Self {
            id: sc.id,
            amount: Money::new(sc.amount, currency),
            amount_refunded: Money::new(sc.amount_refunded.unwrap_or(0), currency),
            status: match sc.status.as_str() {
                "succeeded" => ChargeStatus::Succeeded,
                "failed" => ChargeStatus::Failed,
                "pending" => ChargeStatus::Pending,
                _ => ChargeStatus::Pending,
            },
            customer_id: sc.customer,
            payment_method: sc.payment_method,
            description: sc.description,
            receipt_url: sc.receipt_url,
            failure_reason: sc.failure_message,
            metadata: sc.metadata,
            created_at: Utc.timestamp_opt(sc.created, 0).unwrap(),
            captured: sc.captured,
            refunded: sc.refunded,
            disputed: sc.disputed,
        }
    }
}

#[derive(Debug, Deserialize)]
struct StripeRefund {
    id: String,
    charge: String,
    amount: i64,
    currency: String,
    status: String,
    reason: Option<String>,
    created: i64,
}

impl From<StripeRefund> for Refund {
    fn from(sr: StripeRefund) -> Self {
        let currency = Currency::from_code(&sr.currency).unwrap_or(Currency::USD);
        Self {
            id: sr.id,
            charge_id: sr.charge,
            amount: Money::new(sr.amount, currency),
            status: match sr.status.as_str() {
                "succeeded" => RefundStatus::Succeeded,
                "failed" => RefundStatus::Failed,
                "pending" => RefundStatus::Pending,
                "canceled" => RefundStatus::Canceled,
                _ => RefundStatus::Pending,
            },
            reason: sr.reason.and_then(|r| match r.as_str() {
                "duplicate" => Some(RefundReason::Duplicate),
                "fraudulent" => Some(RefundReason::Fraudulent),
                "requested_by_customer" => Some(RefundReason::RequestedByCustomer),
                _ => None,
            }),
            created_at: Utc.timestamp_opt(sr.created, 0).unwrap(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct StripeCustomer {
    id: String,
    email: Option<String>,
    name: Option<String>,
    phone: Option<String>,
    description: Option<String>,
    default_source: Option<String>,
    created: i64,
    #[serde(default)]
    metadata: HashMap<String, String>,
}

impl From<StripeCustomer> for Customer {
    fn from(sc: StripeCustomer) -> Self {
        Self {
            id: sc.id,
            email: sc.email,
            name: sc.name,
            phone: sc.phone,
            description: sc.description,
            default_payment_method: sc.default_source,
            address: None,
            metadata: sc.metadata,
            created_at: Utc.timestamp_opt(sc.created, 0).unwrap(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct StripePaymentMethod {
    id: String,
    #[allow(dead_code)]
    #[serde(rename = "type")]
    method_type: String,
    customer: Option<String>,
    card: Option<StripeCard>,
    created: i64,
}

#[derive(Debug, Deserialize)]
struct StripeCard {
    brand: String,
    last4: String,
    exp_month: u32,
    exp_year: u32,
    funding: String,
}

impl From<StripePaymentMethod> for PaymentMethod {
    fn from(spm: StripePaymentMethod) -> Self {
        Self {
            id: spm.id,
            method_type: PaymentMethodType::Card,
            customer_id: spm.customer,
            card: spm.card.map(|c| CardInfo {
                brand: c.brand,
                last4: c.last4,
                exp_month: c.exp_month,
                exp_year: c.exp_year,
                funding: match c.funding.as_str() {
                    "credit" => CardFunding::Credit,
                    "debit" => CardFunding::Debit,
                    "prepaid" => CardFunding::Prepaid,
                    _ => CardFunding::Unknown,
                },
            }),
            billing_details: None,
            created_at: Utc.timestamp_opt(spm.created, 0).unwrap(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct StripeSubscription {
    id: String,
    customer: String,
    status: String,
    current_period_start: i64,
    current_period_end: i64,
    trial_end: Option<i64>,
    cancel_at_period_end: bool,
    canceled_at: Option<i64>,
    created: i64,
    #[serde(default)]
    metadata: HashMap<String, String>,
    #[serde(default)]
    items: StripeSubscriptionItems,
}

#[derive(Debug, Default, Deserialize)]
struct StripeSubscriptionItems {
    #[serde(default)]
    data: Vec<StripeSubscriptionItem>,
}

#[derive(Debug, Deserialize)]
struct StripeSubscriptionItem {
    price: Option<StripePrice>,
    quantity: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct StripePrice {
    id: String,
}

impl From<StripeSubscription> for Subscription {
    fn from(ss: StripeSubscription) -> Self {
        // The plan and quantity live on the first subscription item; reading
        // them is the only way to report what the customer is actually
        // subscribed to.
        let first_item = ss.items.data.into_iter().next();
        let price_id = first_item
            .as_ref()
            .and_then(|i| i.price.as_ref())
            .map(|p| p.id.clone())
            .unwrap_or_default();
        let quantity = first_item.as_ref().and_then(|i| i.quantity).unwrap_or(1);

        Self {
            id: ss.id,
            customer_id: Some(ss.customer),
            status: match ss.status.as_str() {
                "active" => SubscriptionStatus::Active,
                "trialing" => SubscriptionStatus::Trialing,
                "past_due" => SubscriptionStatus::PastDue,
                "canceled" => SubscriptionStatus::Canceled,
                "unpaid" => SubscriptionStatus::Unpaid,
                "incomplete" => SubscriptionStatus::Incomplete,
                "incomplete_expired" => SubscriptionStatus::IncompleteExpired,
                "paused" => SubscriptionStatus::Paused,
                _ => SubscriptionStatus::Active,
            },
            current_period_start: Utc.timestamp_opt(ss.current_period_start, 0).single(),
            current_period_end: Utc.timestamp_opt(ss.current_period_end, 0).single(),
            trial_end: ss.trial_end.and_then(|t| Utc.timestamp_opt(t, 0).single()),
            cancel_at_period_end: ss.cancel_at_period_end,
            canceled_at: ss
                .canceled_at
                .and_then(|t| Utc.timestamp_opt(t, 0).single()),
            price_id,
            quantity,
            metadata: ss.metadata,
            created_at: Utc.timestamp_opt(ss.created, 0).single(),
        }
    }
}

/// A Stripe PaymentIntent.
#[derive(Debug, Deserialize)]
pub struct StripePaymentIntent {
    /// PaymentIntent ID.
    pub id: String,
    /// Amount in the currency's minor unit.
    pub amount: i64,
    /// ISO currency code.
    pub currency: String,
    /// Intent status (`succeeded`, `requires_capture`, ...).
    pub status: String,
    /// Client secret for completing the intent in the browser.
    pub client_secret: Option<String>,
    /// Attached customer, if any.
    #[serde(default)]
    pub customer: Option<String>,
    /// Attached payment method, if any.
    #[serde(default)]
    pub payment_method: Option<String>,
    /// Creation time, as a Unix timestamp.
    #[serde(default)]
    pub created: Option<i64>,
    /// The amount already captured.
    #[serde(default)]
    pub amount_received: Option<i64>,
    /// Failure detail when the intent could not be confirmed.
    #[serde(default)]
    pub last_payment_error: Option<StripeErrorDetailPublic>,
}

/// Failure detail attached to a PaymentIntent.
#[derive(Debug, Deserialize)]
pub struct StripeErrorDetailPublic {
    /// Human-readable failure message.
    #[serde(default)]
    pub message: Option<String>,
}

/// Project a confirmed PaymentIntent onto the crate's [`Charge`] shape.
fn charge_from_intent(
    intent: StripePaymentIntent,
    requested: Money,
    description: Option<String>,
    metadata: HashMap<String, String>,
) -> Charge {
    let currency = Currency::from_code(&intent.currency).unwrap_or(requested.currency);
    let status = match intent.status.as_str() {
        "succeeded" => ChargeStatus::Succeeded,
        "canceled" => ChargeStatus::Canceled,
        _ => ChargeStatus::Pending,
    };
    Charge {
        id: intent.id,
        amount: Money::new(intent.amount, currency),
        amount_refunded: Money::new(0, currency),
        status,
        customer_id: intent.customer,
        payment_method: intent.payment_method,
        description,
        receipt_url: None,
        failure_reason: intent.last_payment_error.and_then(|e| e.message),
        metadata,
        created_at: intent
            .created
            .and_then(|t| Utc.timestamp_opt(t, 0).single())
            .unwrap_or_else(Utc::now),
        captured: intent.amount_received.unwrap_or(0) > 0,
        refunded: false,
        disputed: false,
    }
}

#[derive(Debug, Deserialize)]
struct StripeWebhookEvent {
    id: String,
    #[serde(rename = "type")]
    event_type: String,
    created: i64,
    livemode: bool,
    data: StripeWebhookData,
}

#[derive(Debug, Deserialize)]
struct StripeWebhookData {
    object: serde_json::Value,
}
