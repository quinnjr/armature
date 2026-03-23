//! Payment provider trait and common functionality

use crate::{error::PaymentResult, types::*, webhook::WebhookEvent};
use async_trait::async_trait;

/// Payment provider trait
///
/// Implement this trait for each payment gateway (Stripe, PayPal, etc.)
#[async_trait]
pub trait PaymentProvider: Send + Sync {
    /// Get provider name
    fn name(&self) -> &'static str;

    /// Create a charge
    async fn charge(&self, request: ChargeRequest) -> PaymentResult<Charge>;

    /// Capture an authorized charge
    async fn capture(&self, charge_id: &str, amount: Option<crate::Money>)
    -> PaymentResult<Charge>;

    /// Refund a charge
    async fn refund(&self, request: RefundRequest) -> PaymentResult<Refund>;

    /// Create a customer
    async fn create_customer(&self, request: CreateCustomerRequest) -> PaymentResult<Customer>;

    /// Get a customer
    async fn get_customer(&self, id: &str) -> PaymentResult<Customer>;

    /// Update a customer
    async fn update_customer(
        &self,
        id: &str,
        request: UpdateCustomerRequest,
    ) -> PaymentResult<Customer>;

    /// Delete a customer
    async fn delete_customer(&self, id: &str) -> PaymentResult<()>;

    /// Create a payment method
    async fn create_payment_method(
        &self,
        request: CreatePaymentMethodRequest,
    ) -> PaymentResult<PaymentMethod>;

    /// Attach a payment method to a customer
    async fn attach_payment_method(
        &self,
        method_id: &str,
        customer_id: &str,
    ) -> PaymentResult<PaymentMethod>;

    /// Detach a payment method from a customer
    async fn detach_payment_method(&self, method_id: &str) -> PaymentResult<PaymentMethod>;

    /// List customer's payment methods
    async fn list_payment_methods(&self, customer_id: &str) -> PaymentResult<Vec<PaymentMethod>>;

    /// Create a subscription
    async fn create_subscription(
        &self,
        request: CreateSubscriptionRequest,
    ) -> PaymentResult<Subscription>;

    /// Get a subscription
    async fn get_subscription(&self, id: &str) -> PaymentResult<Subscription>;

    /// Update a subscription
    async fn update_subscription(&self, id: &str, price_id: &str) -> PaymentResult<Subscription>;

    /// Cancel a subscription
    async fn cancel_subscription(&self, id: &str, immediate: bool) -> PaymentResult<Subscription>;

    /// Resume a canceled subscription
    async fn resume_subscription(&self, id: &str) -> PaymentResult<Subscription>;

    /// Verify webhook signature
    fn verify_webhook(&self, payload: &[u8], signature: &str) -> PaymentResult<()>;

    /// Parse webhook payload
    fn parse_webhook(&self, payload: &[u8]) -> PaymentResult<WebhookEvent>;
}

/// Provider configuration
pub trait ProviderConfig {
    /// Get API key
    fn api_key(&self) -> &str;

    /// Get webhook secret
    fn webhook_secret(&self) -> Option<&str>;

    /// Is test/sandbox mode
    fn is_test_mode(&self) -> bool;

    /// Get API base URL
    fn base_url(&self) -> &str;
}

/// Common HTTP client for providers
pub struct ProviderClient {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl ProviderClient {
    /// Create a new provider client
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            api_key: api_key.into(),
        }
    }

    /// GET request
    pub async fn get(&self, path: &str) -> PaymentResult<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        Ok(self
            .client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?)
    }

    /// POST request with JSON body
    pub async fn post<T: serde::Serialize>(
        &self,
        path: &str,
        body: &T,
    ) -> PaymentResult<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        Ok(self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(body)
            .send()
            .await?)
    }

    /// POST request with form body
    pub async fn post_form<T: serde::Serialize>(
        &self,
        path: &str,
        body: &T,
    ) -> PaymentResult<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        Ok(self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .form(body)
            .send()
            .await?)
    }

    /// DELETE request
    pub async fn delete(&self, path: &str) -> PaymentResult<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        Ok(self
            .client
            .delete(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?)
    }
}
