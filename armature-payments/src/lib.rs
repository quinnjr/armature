//! Payment Processing Module for Armature Framework
//!
//! Provides a unified interface for payment processing with support for
//! multiple providers including Stripe, PayPal, and Braintree.
//!
//! ## Overview
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                    Payment Processing                            │
//! │                                                                  │
//! │  ┌──────────────────────────────────────────────────────────┐  │
//! │  │                 Unified Payment API                       │  │
//! │  │  charge() | refund() | subscribe() | cancel()            │  │
//! │  └──────────────────────────────────────────────────────────┘  │
//! │                            │                                    │
//! │         ┌──────────────────┼──────────────────┐                │
//! │         ▼                  ▼                  ▼                │
//! │  ┌────────────┐    ┌────────────┐    ┌────────────┐          │
//! │  │   Stripe   │    │   PayPal   │    │ Braintree  │          │
//! │  └────────────┘    └────────────┘    └────────────┘          │
//! │                                                                │
//! │  ┌──────────────────────────────────────────────────────────┐  │
//! │  │                  Webhook Handler                          │  │
//! │  │  payment.succeeded | refund.created | subscription.*     │  │
//! │  └──────────────────────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature_payments::{PaymentProcessor, StripeProvider, Money, Currency};
//!
//! // Initialize with Stripe
//! let processor = PaymentProcessor::new(
//!     StripeProvider::new("sk_test_...")
//! );
//!
//! // Create a charge
//! let charge = processor.charge(ChargeRequest {
//!     amount: Money::new(2999, Currency::USD),
//!     source: PaymentSource::Card(card_token),
//!     description: Some("Order #1234".into()),
//!     metadata: Default::default(),
//! }).await?;
//!
//! // Handle webhooks
//! processor.handle_webhook(request).await?;
//! ```

pub mod error;
pub mod money;
pub mod provider;
pub mod types;
pub mod webhook;

pub mod providers;

pub use error::*;
pub use money::*;
pub use provider::*;
pub use types::*;
pub use webhook::*;

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Main payment processor
pub struct PaymentProcessor<P: PaymentProvider> {
    provider: Arc<P>,
    config: ProcessorConfig,
}

impl<P: PaymentProvider> PaymentProcessor<P> {
    /// Create a new payment processor
    pub fn new(provider: P) -> Self {
        Self {
            provider: Arc::new(provider),
            config: ProcessorConfig::default(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(provider: P, config: ProcessorConfig) -> Self {
        Self {
            provider: Arc::new(provider),
            config,
        }
    }

    /// Get the provider
    pub fn provider(&self) -> &P {
        &self.provider
    }

    /// Create a charge
    pub async fn charge(&self, request: ChargeRequest) -> PaymentResult<Charge> {
        self.provider.charge(request).await
    }

    /// Refund a charge
    pub async fn refund(&self, request: RefundRequest) -> PaymentResult<Refund> {
        self.provider.refund(request).await
    }

    /// Create a customer
    pub async fn create_customer(&self, request: CreateCustomerRequest) -> PaymentResult<Customer> {
        self.provider.create_customer(request).await
    }

    /// Update a customer
    pub async fn update_customer(
        &self,
        id: &str,
        request: UpdateCustomerRequest,
    ) -> PaymentResult<Customer> {
        self.provider.update_customer(id, request).await
    }

    /// Delete a customer
    pub async fn delete_customer(&self, id: &str) -> PaymentResult<()> {
        self.provider.delete_customer(id).await
    }

    /// Create a payment method
    pub async fn create_payment_method(
        &self,
        request: CreatePaymentMethodRequest,
    ) -> PaymentResult<PaymentMethod> {
        self.provider.create_payment_method(request).await
    }

    /// Attach a payment method to a customer
    pub async fn attach_payment_method(
        &self,
        method_id: &str,
        customer_id: &str,
    ) -> PaymentResult<PaymentMethod> {
        self.provider
            .attach_payment_method(method_id, customer_id)
            .await
    }

    /// Create a subscription
    pub async fn create_subscription(
        &self,
        request: CreateSubscriptionRequest,
    ) -> PaymentResult<Subscription> {
        self.provider.create_subscription(request).await
    }

    /// Cancel a subscription
    pub async fn cancel_subscription(
        &self,
        id: &str,
        immediate: bool,
    ) -> PaymentResult<Subscription> {
        self.provider.cancel_subscription(id, immediate).await
    }

    /// Handle a webhook event
    pub async fn handle_webhook(
        &self,
        payload: &[u8],
        signature: &str,
    ) -> PaymentResult<WebhookEvent> {
        self.provider.verify_webhook(payload, signature)?;
        self.provider.parse_webhook(payload)
    }
}

impl<P: PaymentProvider> Clone for PaymentProcessor<P> {
    fn clone(&self) -> Self {
        Self {
            provider: Arc::clone(&self.provider),
            config: self.config.clone(),
        }
    }
}

/// Processor configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessorConfig {
    /// Retry failed charges
    pub retry_failed: bool,
    /// Maximum retry attempts
    pub max_retries: u32,
    /// Retry delay in milliseconds
    pub retry_delay_ms: u64,
    /// Enable idempotency keys
    pub use_idempotency: bool,
    /// Log all transactions
    pub log_transactions: bool,
}

impl Default for ProcessorConfig {
    fn default() -> Self {
        Self {
            retry_failed: true,
            max_retries: 3,
            retry_delay_ms: 1000,
            use_idempotency: true,
            log_transactions: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_processor_config_default() {
        let config = ProcessorConfig::default();
        assert!(config.retry_failed);
        assert_eq!(config.max_retries, 3);
    }
}
