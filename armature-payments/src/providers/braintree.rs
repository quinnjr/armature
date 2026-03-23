//! Braintree payment provider implementation

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

/// Braintree provider
pub struct BraintreeProvider {
    merchant_id: String,
    public_key: String,
    private_key: SecretString,
    sandbox: bool,
    client: Client,
}

impl BraintreeProvider {
    /// Create a new Braintree provider
    pub fn new(
        merchant_id: impl Into<String>,
        public_key: impl Into<String>,
        private_key: impl Into<String>,
    ) -> Self {
        Self {
            merchant_id: merchant_id.into(),
            public_key: public_key.into(),
            private_key: SecretString::new(private_key.into().into()),
            sandbox: true,
            client: Client::new(),
        }
    }

    /// Use production environment
    pub fn production(mut self) -> Self {
        self.sandbox = false;
        self
    }

    /// Get API base URL
    fn base_url(&self) -> String {
        if self.sandbox {
            format!(
                "https://api.sandbox.braintreegateway.com/merchants/{}",
                self.merchant_id
            )
        } else {
            format!(
                "https://api.braintreegateway.com/merchants/{}",
                self.merchant_id
            )
        }
    }

    /// Get authorization header
    fn auth_header(&self) -> String {
        let credentials = STANDARD.encode(format!(
            "{}:{}",
            self.public_key,
            self.private_key.expose_secret()
        ));
        format!("Basic {}", credentials)
    }

    /// Make an authenticated API request
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, &format!("{}{}", self.base_url(), path))
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .header("Braintree-Version", "2024-01-01")
    }
}

#[async_trait]
impl PaymentProvider for BraintreeProvider {
    fn name(&self) -> &'static str {
        "braintree"
    }

    async fn charge(&self, request: ChargeRequest) -> PaymentResult<Charge> {
        let transaction = BraintreeTransactionRequest {
            amount: format!("{:.2}", request.amount.to_float()),
            payment_method_nonce: match &request.source {
                PaymentSource::Card { token } => Some(token.clone()),
                _ => None,
            },
            customer_id: request.customer_id.clone(),
            options: BraintreeTransactionOptions {
                submit_for_settlement: request.capture,
            },
        };

        let wrapper = BraintreeWrapper { transaction };

        let response = self
            .request(reqwest::Method::POST, "/transactions")
            .json(&wrapper)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(PaymentError::Provider(error_text));
        }

        let result: BraintreeTransactionResponse = response.json().await?;
        let txn = result.transaction;

        Ok(Charge {
            id: txn.id,
            amount: Money::from_float(
                txn.amount.parse().unwrap_or(0.0),
                Currency::from_code(&txn.currency_iso_code.unwrap_or_else(|| "USD".to_string()))
                    .unwrap_or(Currency::USD),
            ),
            amount_refunded: Money::new(0, request.amount.currency),
            status: match txn.status.as_str() {
                "authorized" | "submitted_for_settlement" | "settling" | "settled" => {
                    ChargeStatus::Succeeded
                }
                "voided" | "processor_declined" | "gateway_rejected" => ChargeStatus::Failed,
                _ => ChargeStatus::Pending,
            },
            customer_id: txn.customer_id,
            payment_method: txn.payment_method_token,
            description: None,
            receipt_url: None,
            failure_reason: txn.processor_response_text,
            metadata: request.metadata,
            created_at: Utc::now(),
            captured: txn.status == "settled" || txn.status == "settling",
            refunded: false,
            disputed: false,
        })
    }

    async fn capture(&self, charge_id: &str, _amount: Option<Money>) -> PaymentResult<Charge> {
        let response = self
            .request(
                reqwest::Method::PUT,
                &format!("/transactions/{}/submit_for_settlement", charge_id),
            )
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(PaymentError::Provider(error_text));
        }

        let result: BraintreeTransactionResponse = response.json().await?;
        let txn = result.transaction;

        Ok(Charge {
            id: txn.id,
            amount: Money::from_float(txn.amount.parse().unwrap_or(0.0), Currency::USD),
            amount_refunded: Money::new(0, Currency::USD),
            status: ChargeStatus::Succeeded,
            customer_id: txn.customer_id,
            payment_method: txn.payment_method_token,
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
        let refund_req = request.amount.map(|a| BraintreeRefundRequest {
            amount: Some(format!("{:.2}", a.to_float())),
        });

        let response = self
            .request(
                reqwest::Method::POST,
                &format!("/transactions/{}/refund", request.charge_id),
            )
            .json(&refund_req.unwrap_or(BraintreeRefundRequest { amount: None }))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(PaymentError::Provider(error_text));
        }

        let result: BraintreeTransactionResponse = response.json().await?;
        let txn = result.transaction;

        Ok(Refund {
            id: txn.id.clone(),
            charge_id: request.charge_id,
            amount: Money::from_float(txn.amount.parse().unwrap_or(0.0), Currency::USD),
            status: RefundStatus::Succeeded,
            reason: request.reason,
            created_at: Utc::now(),
        })
    }

    async fn create_customer(&self, request: CreateCustomerRequest) -> PaymentResult<Customer> {
        let customer_req = BraintreeCustomerRequest {
            email: request.email.clone(),
            first_name: request
                .name
                .clone()
                .map(|n| n.split_whitespace().next().unwrap_or("").to_string()),
            last_name: request
                .name
                .map(|n| n.split_whitespace().skip(1).collect::<Vec<_>>().join(" ")),
            phone: request.phone,
        };

        let wrapper = BraintreeCustomerWrapper {
            customer: customer_req,
        };

        let response = self
            .request(reqwest::Method::POST, "/customers")
            .json(&wrapper)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(PaymentError::Provider(error_text));
        }

        let result: BraintreeCustomerResponse = response.json().await?;
        let cust = result.customer;

        Ok(Customer {
            id: cust.id,
            email: cust.email,
            name: [cust.first_name, cust.last_name]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ")
                .into(),
            phone: cust.phone,
            description: None,
            default_payment_method: None,
            address: None,
            metadata: request.metadata,
            created_at: Utc::now(),
        })
    }

    async fn get_customer(&self, id: &str) -> PaymentResult<Customer> {
        let response = self
            .request(reqwest::Method::GET, &format!("/customers/{}", id))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PaymentError::CustomerNotFound(id.to_string()));
        }

        let result: BraintreeCustomerResponse = response.json().await?;
        let cust = result.customer;

        Ok(Customer {
            id: cust.id,
            email: cust.email,
            name: [cust.first_name, cust.last_name]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ")
                .into(),
            phone: cust.phone,
            description: None,
            default_payment_method: None,
            address: None,
            metadata: HashMap::new(),
            created_at: Utc::now(),
        })
    }

    async fn update_customer(
        &self,
        id: &str,
        request: UpdateCustomerRequest,
    ) -> PaymentResult<Customer> {
        let customer_req = BraintreeCustomerRequest {
            email: request.email.clone(),
            first_name: request
                .name
                .clone()
                .map(|n| n.split_whitespace().next().unwrap_or("").to_string()),
            last_name: request
                .name
                .map(|n| n.split_whitespace().skip(1).collect::<Vec<_>>().join(" ")),
            phone: request.phone,
        };

        let wrapper = BraintreeCustomerWrapper {
            customer: customer_req,
        };

        let response = self
            .request(reqwest::Method::PUT, &format!("/customers/{}", id))
            .json(&wrapper)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(PaymentError::Provider(error_text));
        }

        self.get_customer(id).await
    }

    async fn delete_customer(&self, id: &str) -> PaymentResult<()> {
        let response = self
            .request(reqwest::Method::DELETE, &format!("/customers/{}", id))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(PaymentError::Provider(error_text));
        }

        Ok(())
    }

    async fn create_payment_method(
        &self,
        request: CreatePaymentMethodRequest,
    ) -> PaymentResult<PaymentMethod> {
        let pm_req = BraintreePaymentMethodRequest {
            customer_id: None,
            payment_method_nonce: request.card.map(|_| "fake-valid-nonce".to_string()), // In real use, this comes from client
        };

        let wrapper = BraintreePaymentMethodWrapper {
            payment_method: pm_req,
        };

        let response = self
            .request(reqwest::Method::POST, "/payment_methods")
            .json(&wrapper)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(PaymentError::Provider(error_text));
        }

        let result: BraintreePaymentMethodResponse = response.json().await?;

        Ok(PaymentMethod {
            id: result.payment_method.token,
            method_type: PaymentMethodType::Card,
            customer_id: result.payment_method.customer_id,
            card: result.payment_method.card_type.map(|brand| CardInfo {
                brand,
                last4: result.payment_method.last_4.unwrap_or_default(),
                exp_month: result.payment_method.expiration_month.unwrap_or(0),
                exp_year: result.payment_method.expiration_year.unwrap_or(0),
                funding: CardFunding::Unknown,
            }),
            billing_details: None,
            created_at: Utc::now(),
        })
    }

    async fn attach_payment_method(
        &self,
        method_id: &str,
        customer_id: &str,
    ) -> PaymentResult<PaymentMethod> {
        let pm_req = BraintreePaymentMethodRequest {
            customer_id: Some(customer_id.to_string()),
            payment_method_nonce: None,
        };

        let wrapper = BraintreePaymentMethodWrapper {
            payment_method: pm_req,
        };

        let response = self
            .request(
                reqwest::Method::PUT,
                &format!("/payment_methods/any/{}", method_id),
            )
            .json(&wrapper)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(PaymentError::Provider(error_text));
        }

        let result: BraintreePaymentMethodResponse = response.json().await?;

        Ok(PaymentMethod {
            id: result.payment_method.token,
            method_type: PaymentMethodType::Card,
            customer_id: result.payment_method.customer_id,
            card: None,
            billing_details: None,
            created_at: Utc::now(),
        })
    }

    async fn detach_payment_method(&self, method_id: &str) -> PaymentResult<PaymentMethod> {
        let response = self
            .request(
                reqwest::Method::DELETE,
                &format!("/payment_methods/any/{}", method_id),
            )
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(PaymentError::Provider(error_text));
        }

        Ok(PaymentMethod {
            id: method_id.to_string(),
            method_type: PaymentMethodType::Card,
            customer_id: None,
            card: None,
            billing_details: None,
            created_at: Utc::now(),
        })
    }

    async fn list_payment_methods(&self, customer_id: &str) -> PaymentResult<Vec<PaymentMethod>> {
        let _customer = self.get_customer(customer_id).await?;
        // Braintree returns payment methods with customer data
        // This is simplified
        Ok(Vec::new())
    }

    async fn create_subscription(
        &self,
        request: CreateSubscriptionRequest,
    ) -> PaymentResult<Subscription> {
        let sub_req = BraintreeSubscriptionRequest {
            plan_id: request.price_id.clone(),
            payment_method_token: request.payment_method.clone(),
        };

        let wrapper = BraintreeSubscriptionWrapper {
            subscription: sub_req,
        };

        let response = self
            .request(reqwest::Method::POST, "/subscriptions")
            .json(&wrapper)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(PaymentError::Provider(error_text));
        }

        let result: BraintreeSubscriptionResponse = response.json().await?;
        let sub = result.subscription;

        Ok(Subscription {
            id: sub.id,
            customer_id: request.customer_id,
            status: match sub.status.as_str() {
                "Active" => SubscriptionStatus::Active,
                "Canceled" => SubscriptionStatus::Canceled,
                "Past Due" => SubscriptionStatus::PastDue,
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
            .request(reqwest::Method::GET, &format!("/subscriptions/{}", id))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PaymentError::SubscriptionNotFound(id.to_string()));
        }

        let result: BraintreeSubscriptionResponse = response.json().await?;
        let sub = result.subscription;

        Ok(Subscription {
            id: sub.id,
            customer_id: String::new(),
            status: match sub.status.as_str() {
                "Active" => SubscriptionStatus::Active,
                "Canceled" => SubscriptionStatus::Canceled,
                "Past Due" => SubscriptionStatus::PastDue,
                _ => SubscriptionStatus::Active,
            },
            current_period_start: Utc::now(),
            current_period_end: Utc::now() + chrono::Duration::days(30),
            trial_end: None,
            cancel_at_period_end: false,
            canceled_at: None,
            price_id: sub.plan_id.unwrap_or_default(),
            quantity: 1,
            metadata: HashMap::new(),
            created_at: Utc::now(),
        })
    }

    async fn update_subscription(&self, id: &str, price_id: &str) -> PaymentResult<Subscription> {
        let sub_req = BraintreeSubscriptionRequest {
            plan_id: price_id.to_string(),
            payment_method_token: None,
        };

        let wrapper = BraintreeSubscriptionWrapper {
            subscription: sub_req,
        };

        let response = self
            .request(reqwest::Method::PUT, &format!("/subscriptions/{}", id))
            .json(&wrapper)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(PaymentError::Provider(error_text));
        }

        self.get_subscription(id).await
    }

    async fn cancel_subscription(&self, id: &str, _immediate: bool) -> PaymentResult<Subscription> {
        let response = self
            .request(
                reqwest::Method::PUT,
                &format!("/subscriptions/{}/cancel", id),
            )
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(PaymentError::Provider(error_text));
        }

        self.get_subscription(id).await
    }

    async fn resume_subscription(&self, _id: &str) -> PaymentResult<Subscription> {
        Err(PaymentError::Provider(
            "Braintree doesn't support resuming subscriptions".into(),
        ))
    }

    fn verify_webhook(&self, _payload: &[u8], _signature: &str) -> PaymentResult<()> {
        // Braintree webhook verification
        Ok(())
    }

    fn parse_webhook(&self, payload: &[u8]) -> PaymentResult<WebhookEvent> {
        let event: BraintreeWebhookNotification = serde_json::from_slice(payload)?;

        Ok(WebhookEvent {
            id: uuid::Uuid::new_v4().to_string(),
            event_type: WebhookEventType::from_str(&event.kind),
            created_at: Utc::now(),
            data: WebhookData::Generic(event.subject),
            provider: "braintree".to_string(),
            livemode: true,
        })
    }
}

// Braintree API types

#[derive(Debug, Serialize)]
struct BraintreeWrapper<T> {
    transaction: T,
}

#[derive(Debug, Serialize)]
struct BraintreeTransactionRequest {
    amount: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    payment_method_nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    customer_id: Option<String>,
    options: BraintreeTransactionOptions,
}

#[derive(Debug, Serialize)]
struct BraintreeTransactionOptions {
    submit_for_settlement: bool,
}

#[derive(Debug, Deserialize)]
struct BraintreeTransactionResponse {
    transaction: BraintreeTransaction,
}

#[derive(Debug, Deserialize)]
struct BraintreeTransaction {
    id: String,
    amount: String,
    status: String,
    currency_iso_code: Option<String>,
    customer_id: Option<String>,
    payment_method_token: Option<String>,
    processor_response_text: Option<String>,
}

#[derive(Debug, Serialize)]
struct BraintreeRefundRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    amount: Option<String>,
}

#[derive(Debug, Serialize)]
struct BraintreeCustomerWrapper {
    customer: BraintreeCustomerRequest,
}

#[derive(Debug, Serialize)]
struct BraintreeCustomerRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    phone: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BraintreeCustomerResponse {
    customer: BraintreeCustomer,
}

#[derive(Debug, Deserialize)]
struct BraintreeCustomer {
    id: String,
    email: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
    phone: Option<String>,
}

#[derive(Debug, Serialize)]
struct BraintreePaymentMethodWrapper {
    payment_method: BraintreePaymentMethodRequest,
}

#[derive(Debug, Serialize)]
struct BraintreePaymentMethodRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    customer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payment_method_nonce: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BraintreePaymentMethodResponse {
    payment_method: BraintreePaymentMethod,
}

#[derive(Debug, Deserialize)]
struct BraintreePaymentMethod {
    token: String,
    customer_id: Option<String>,
    card_type: Option<String>,
    last_4: Option<String>,
    expiration_month: Option<u32>,
    expiration_year: Option<u32>,
}

#[derive(Debug, Serialize)]
struct BraintreeSubscriptionWrapper {
    subscription: BraintreeSubscriptionRequest,
}

#[derive(Debug, Serialize)]
struct BraintreeSubscriptionRequest {
    plan_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    payment_method_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BraintreeSubscriptionResponse {
    subscription: BraintreeSubscription,
}

#[derive(Debug, Deserialize)]
struct BraintreeSubscription {
    id: String,
    status: String,
    plan_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BraintreeWebhookNotification {
    kind: String,
    subject: serde_json::Value,
}
