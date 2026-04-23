//! Stripe payment provider implementation

use crate::{
    error::{PaymentError, PaymentResult},
    money::{Currency, Money},
    provider::{PaymentProvider, ProviderClient},
    types::*,
    webhook::{WebhookData, WebhookEvent, WebhookEventType},
};
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use hmac::{Hmac, Mac};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use sha2::Sha256;
use std::collections::HashMap;

/// Stripe provider
pub struct StripeProvider {
    #[allow(dead_code)]
    api_key: SecretString,
    webhook_secret: Option<SecretString>,
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
        }
    }

    /// Set webhook secret
    pub fn with_webhook_secret(mut self, secret: impl Into<String>) -> Self {
        self.webhook_secret = Some(SecretString::new(secret.into().into()));
        self
    }

    /// Create a payment intent
    pub async fn create_payment_intent(
        &self,
        request: ChargeRequest,
    ) -> PaymentResult<StripePaymentIntent> {
        let mut params = HashMap::new();
        params.insert("amount", request.amount.amount.to_string());
        params.insert("currency", request.amount.currency.code().to_lowercase());

        if let Some(desc) = &request.description {
            params.insert("description", desc.clone());
        }

        if let Some(customer_id) = &request.customer_id {
            params.insert("customer", customer_id.clone());
        }

        match &request.source {
            PaymentSource::PaymentMethod { id } => {
                params.insert("payment_method", id.clone());
                if request.capture {
                    params.insert("confirm", "true".to_string());
                }
            }
            PaymentSource::Customer { customer_id } => {
                params.insert("customer", customer_id.clone());
            }
            _ => {}
        }

        let response = self.client.post_form("/payment_intents", &params).await?;
        let intent: StripePaymentIntent = response.json().await?;
        Ok(intent)
    }
}

#[async_trait]
impl PaymentProvider for StripeProvider {
    fn name(&self) -> &'static str {
        "stripe"
    }

    async fn charge(&self, request: ChargeRequest) -> PaymentResult<Charge> {
        let mut params = HashMap::new();
        params.insert("amount", request.amount.amount.to_string());
        params.insert("currency", request.amount.currency.code().to_lowercase());

        if let Some(desc) = &request.description {
            params.insert("description", desc.clone());
        }

        if let Some(customer_id) = &request.customer_id {
            params.insert("customer", customer_id.clone());
        }

        match &request.source {
            PaymentSource::Card { token } => {
                params.insert("source", token.clone());
            }
            PaymentSource::Customer { customer_id } => {
                params.insert("customer", customer_id.clone());
            }
            _ => {}
        }

        params.insert("capture", request.capture.to_string());

        let response = self.client.post_form("/charges", &params).await?;

        if !response.status().is_success() {
            let error: StripeError = response.json().await?;
            return Err(PaymentError::Provider(error.error.message));
        }

        let stripe_charge: StripeCharge = response.json().await?;
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

        let stripe_charge: StripeCharge = response.json().await?;
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
        let stripe_refund: StripeRefund = response.json().await?;
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
        let stripe_customer: StripeCustomer = response.json().await?;
        Ok(stripe_customer.into())
    }

    async fn get_customer(&self, id: &str) -> PaymentResult<Customer> {
        let response = self.client.get(&format!("/customers/{}", id)).await?;
        let stripe_customer: StripeCustomer = response.json().await?;
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
        let stripe_customer: StripeCustomer = response.json().await?;
        Ok(stripe_customer.into())
    }

    async fn delete_customer(&self, id: &str) -> PaymentResult<()> {
        self.client.delete(&format!("/customers/{}", id)).await?;
        Ok(())
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
        let stripe_pm: StripePaymentMethod = response.json().await?;
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
        let stripe_pm: StripePaymentMethod = response.json().await?;
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
        let stripe_pm: StripePaymentMethod = response.json().await?;
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
        let list: StripeList<StripePaymentMethod> = response.json().await?;
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
        let stripe_sub: StripeSubscription = response.json().await?;
        Ok(stripe_sub.into())
    }

    async fn get_subscription(&self, id: &str) -> PaymentResult<Subscription> {
        let response = self.client.get(&format!("/subscriptions/{}", id)).await?;
        let stripe_sub: StripeSubscription = response.json().await?;
        Ok(stripe_sub.into())
    }

    async fn update_subscription(&self, id: &str, price_id: &str) -> PaymentResult<Subscription> {
        let mut params = HashMap::new();
        params.insert("items[0][price]", price_id.to_string());

        let response = self
            .client
            .post_form(&format!("/subscriptions/{}", id), &params)
            .await?;
        let stripe_sub: StripeSubscription = response.json().await?;
        Ok(stripe_sub.into())
    }

    async fn cancel_subscription(&self, id: &str, immediate: bool) -> PaymentResult<Subscription> {
        if immediate {
            let response = self
                .client
                .delete(&format!("/subscriptions/{}", id))
                .await?;
            let stripe_sub: StripeSubscription = response.json().await?;
            Ok(stripe_sub.into())
        } else {
            let mut params = HashMap::new();
            params.insert("cancel_at_period_end", "true".to_string());

            let response = self
                .client
                .post_form(&format!("/subscriptions/{}", id), &params)
                .await?;
            let stripe_sub: StripeSubscription = response.json().await?;
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
        let stripe_sub: StripeSubscription = response.json().await?;
        Ok(stripe_sub.into())
    }

    fn verify_webhook(&self, payload: &[u8], signature: &str) -> PaymentResult<()> {
        let secret = self
            .webhook_secret
            .as_ref()
            .ok_or(PaymentError::Config("Webhook secret not configured".into()))?;

        // Parse Stripe signature header
        let parts: HashMap<&str, &str> = signature
            .split(',')
            .filter_map(|part| {
                let mut kv = part.split('=');
                Some((kv.next()?, kv.next()?))
            })
            .collect();

        let timestamp = parts
            .get("t")
            .ok_or(PaymentError::InvalidWebhookSignature)?;
        let expected_sig = parts
            .get("v1")
            .ok_or(PaymentError::InvalidWebhookSignature)?;

        // Compute signature
        let signed_payload = format!("{}.{}", timestamp, String::from_utf8_lossy(payload));
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.expose_secret().as_bytes())
            .map_err(|_| PaymentError::InvalidWebhookSignature)?;
        mac.update(signed_payload.as_bytes());
        let computed_sig = hex::encode(mac.finalize().into_bytes());

        if computed_sig != *expected_sig {
            return Err(PaymentError::InvalidWebhookSignature);
        }

        Ok(())
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
    message: String,
    #[allow(dead_code)]
    #[serde(rename = "type")]
    error_type: Option<String>,
    #[allow(dead_code)]
    code: Option<String>,
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
}

impl From<StripeSubscription> for Subscription {
    fn from(ss: StripeSubscription) -> Self {
        Self {
            id: ss.id,
            customer_id: ss.customer,
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
            current_period_start: Utc.timestamp_opt(ss.current_period_start, 0).unwrap(),
            current_period_end: Utc.timestamp_opt(ss.current_period_end, 0).unwrap(),
            trial_end: ss.trial_end.map(|t| Utc.timestamp_opt(t, 0).unwrap()),
            cancel_at_period_end: ss.cancel_at_period_end,
            canceled_at: ss.canceled_at.map(|t| Utc.timestamp_opt(t, 0).unwrap()),
            price_id: String::new(), // Would need to extract from items
            quantity: 1,
            metadata: ss.metadata,
            created_at: Utc.timestamp_opt(ss.created, 0).unwrap(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct StripePaymentIntent {
    pub id: String,
    pub amount: i64,
    pub currency: String,
    pub status: String,
    pub client_secret: Option<String>,
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
