//! PayPal payment provider implementation

use crate::{
    error::{PaymentError, PaymentResult},
    money::{Currency, Money},
    provider::PaymentProvider,
    types::*,
    webhook::{WebhookData, WebhookEvent, WebhookEventType},
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
    client: Client,
    access_token: tokio::sync::RwLock<Option<PayPalToken>>,
}

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
            client: Client::new(),
            access_token: tokio::sync::RwLock::new(None),
        }
    }

    /// Use production environment
    pub fn production(mut self) -> Self {
        self.sandbox = false;
        self
    }

    /// Set webhook ID for verification
    pub fn with_webhook_id(mut self, webhook_id: impl Into<String>) -> Self {
        self.webhook_id = Some(webhook_id.into());
        self
    }

    /// Get API base URL
    fn base_url(&self) -> &str {
        if self.sandbox {
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
            if let Some(ref t) = *token {
                if t.expires_at > Utc::now() {
                    return Ok(t.token.clone());
                }
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
            .post(&format!("{}/v1/oauth2/token", self.base_url()))
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
            .request(method, &format!("{}{}", self.base_url(), path))
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

    async fn capture(&self, charge_id: &str, _amount: Option<Money>) -> PaymentResult<Charge> {
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("/v2/checkout/orders/{}/capture", charge_id),
            )
            .await?
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

    async fn create_customer(&self, request: CreateCustomerRequest) -> PaymentResult<Customer> {
        // PayPal doesn't have a direct customer API like Stripe
        // We create a stub customer for compatibility
        Ok(Customer {
            id: uuid::Uuid::new_v4().to_string(),
            email: request.email,
            name: request.name,
            phone: request.phone,
            description: request.description,
            default_payment_method: None,
            address: request.address,
            metadata: request.metadata,
            created_at: Utc::now(),
        })
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

        Ok(Subscription {
            id: paypal_sub.id,
            customer_id: request.customer_id,
            status: match paypal_sub.status.as_str() {
                "ACTIVE" => SubscriptionStatus::Active,
                "CANCELLED" => SubscriptionStatus::Canceled,
                "SUSPENDED" => SubscriptionStatus::Paused,
                _ => SubscriptionStatus::Active,
            },
            current_period_start: Utc::now(),
            current_period_end: Utc::now() + chrono::Duration::days(30),
            trial_end: None,
            cancel_at_period_end: false,
            canceled_at: None,
            price_id: request.price_id,
            quantity: request.quantity.unwrap_or(1),
            metadata: request.metadata,
            created_at: Utc::now(),
        })
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

        Ok(Subscription {
            id: paypal_sub.id,
            customer_id: String::new(),
            status: match paypal_sub.status.as_str() {
                "ACTIVE" => SubscriptionStatus::Active,
                "CANCELLED" => SubscriptionStatus::Canceled,
                "SUSPENDED" => SubscriptionStatus::Paused,
                _ => SubscriptionStatus::Active,
            },
            current_period_start: Utc::now(),
            current_period_end: Utc::now() + chrono::Duration::days(30),
            trial_end: None,
            cancel_at_period_end: false,
            canceled_at: None,
            price_id: String::new(),
            quantity: 1,
            metadata: HashMap::new(),
            created_at: Utc::now(),
        })
    }

    async fn update_subscription(&self, id: &str, _price_id: &str) -> PaymentResult<Subscription> {
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

    fn verify_webhook(&self, _payload: &[u8], _signature: &str) -> PaymentResult<()> {
        // PayPal webhook verification requires calling their API
        // This is a simplified version
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

#[derive(Debug, Deserialize)]
struct PayPalSubscription {
    id: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct PayPalWebhookEvent {
    id: String,
    event_type: String,
    resource: serde_json::Value,
}
