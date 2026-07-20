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
//! use armature_payments::{
//!     ChargeRequest, Money, PaymentProcessor, PaymentSource, WebhookHeaders,
//! };
//! use armature_payments::providers::StripeProvider;
//!
//! // Initialize with Stripe
//! let processor = PaymentProcessor::new(
//!     StripeProvider::new("sk_test_...").with_webhook_secret("whsec_..."),
//! );
//!
//! // Create a charge
//! let charge = processor
//!     .charge(
//!         ChargeRequest::new(Money::usd(2999), PaymentSource::card("tok_visa"))
//!             .description("Order #1234"),
//!     )
//!     .await?;
//!
//! // Handle webhooks. Verification runs before parsing and fails closed, so
//! // pass the request's real headers — a forged webhook is rejected here.
//! let headers = WebhookHeaders::single("Stripe-Signature", signature_header);
//! let event = processor.handle_webhook(&body, &headers).await?;
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

    /// The processor's configuration.
    pub fn config(&self) -> &ProcessorConfig {
        &self.config
    }

    /// Create a charge.
    ///
    /// Honors [`ProcessorConfig`]: a retryable failure (network or throttling)
    /// is retried up to `max_retries` times with `retry_delay_ms` between
    /// attempts. When `use_idempotency` is set and the request carries no key,
    /// one is generated and reused across every attempt so a retry after an
    /// ambiguous timeout cannot double-charge the customer.
    pub async fn charge(&self, request: ChargeRequest) -> PaymentResult<Charge> {
        let mut request = request;
        if self.config.use_idempotency && request.idempotency_key.is_none() {
            request.idempotency_key = Some(uuid::Uuid::new_v4().to_string());
        }

        let amount = request.amount;
        let result = self
            .with_retries("charge", || {
                let provider = Arc::clone(&self.provider);
                let request = request.clone();
                async move { provider.charge(request).await }
            })
            .await;

        if self.config.log_transactions {
            match &result {
                Ok(charge) => armature_log::info!(
                    target: "armature::payments",
                    "charge {} {} {} via {} -> {:?}",
                    charge.id,
                    amount.amount,
                    amount.currency.code(),
                    self.provider.name(),
                    charge.status
                ),
                Err(e) => armature_log::warn!(
                    target: "armature::payments",
                    "charge of {} {} via {} failed: {}",
                    amount.amount,
                    amount.currency.code(),
                    self.provider.name(),
                    e
                ),
            }
        }

        result
    }

    /// Refund a charge.
    ///
    /// Retries on transient failures per [`ProcessorConfig`].
    pub async fn refund(&self, request: RefundRequest) -> PaymentResult<Refund> {
        let charge_id = request.charge_id.clone();
        let result = self
            .with_retries("refund", || {
                let provider = Arc::clone(&self.provider);
                let request = request.clone();
                async move { provider.refund(request).await }
            })
            .await;

        if self.config.log_transactions {
            match &result {
                Ok(refund) => armature_log::info!(
                    target: "armature::payments",
                    "refund {} of charge {} via {} -> {:?}",
                    refund.id,
                    charge_id,
                    self.provider.name(),
                    refund.status
                ),
                Err(e) => armature_log::warn!(
                    target: "armature::payments",
                    "refund of charge {} via {} failed: {}",
                    charge_id,
                    self.provider.name(),
                    e
                ),
            }
        }

        result
    }

    /// Run `op` under the configured retry policy.
    ///
    /// Only [`PaymentError::is_retryable`] failures are retried; a decline or a
    /// validation error is returned on the first attempt.
    async fn with_retries<T, F, Fut>(&self, operation: &str, mut op: F) -> PaymentResult<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = PaymentResult<T>>,
    {
        let max_attempts = if self.config.retry_failed {
            self.config.max_retries.saturating_add(1)
        } else {
            1
        };

        let mut attempt = 1;
        loop {
            match op().await {
                Ok(value) => return Ok(value),
                Err(e) if e.is_retryable() && attempt < max_attempts => {
                    if self.config.log_transactions {
                        armature_log::warn!(
                            target: "armature::payments",
                            "{} attempt {}/{} failed ({}), retrying in {}ms",
                            operation,
                            attempt,
                            max_attempts,
                            e,
                            self.config.retry_delay_ms
                        );
                    }
                    if self.config.retry_delay_ms > 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(
                            self.config.retry_delay_ms,
                        ))
                        .await;
                    }
                    attempt += 1;
                }
                Err(e) => return Err(e),
            }
        }
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

    /// Verify and parse an inbound webhook.
    ///
    /// Verification runs before parsing and must succeed: an unsigned or
    /// mis-signed payload is never handed back to the caller.
    pub async fn handle_webhook(
        &self,
        payload: &[u8],
        headers: &WebhookHeaders,
    ) -> PaymentResult<WebhookEvent> {
        self.provider.verify_webhook(payload, headers).await?;
        let event = self.provider.parse_webhook(payload)?;

        if self.config.log_transactions {
            armature_log::info!(
                target: "armature::payments",
                "verified {} webhook {} ({:?})",
                self.provider.name(),
                event.id,
                event.event_type
            );
        }

        Ok(event)
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
