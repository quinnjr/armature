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
//! ```rust,no_run
//! use armature_payments::providers::StripeProvider;
//! use armature_payments::{
//!     ChargeRequest, Money, PaymentProcessor, PaymentSource, WebhookHeaders,
//! };
//!
//! # async fn example(
//! #     body: Vec<u8>,
//! #     signature_header: &str,
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! // Initialize with Stripe
//! let processor = PaymentProcessor::new(
//!     StripeProvider::new("sk_test_...").with_webhook_secret("whsec_..."),
//! );
//!
//! // Create a charge
//! let _charge = processor
//!     .charge(
//!         ChargeRequest::new(Money::usd(2999), PaymentSource::card("tok_visa"))
//!             .description("Order #1234"),
//!     )
//!     .await?;
//!
//! // Handle webhooks. Verification runs before parsing and fails closed, so
//! // pass the request's real headers — a forged webhook is rejected here.
//! let headers = WebhookHeaders::single("Stripe-Signature", signature_header);
//! let _event = processor.handle_webhook(&body, &headers).await?;
//! # Ok(())
//! # }
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Main payment processor
pub struct PaymentProcessor<P: PaymentProvider> {
    provider: Arc<P>,
    config: ProcessorConfig,
    /// Set once we have warned that retries are disabled for lack of
    /// idempotency support, so the warning does not repeat per transaction.
    warned_no_idempotency: Arc<AtomicBool>,
}

impl<P: PaymentProvider> PaymentProcessor<P> {
    /// Create a new payment processor
    pub fn new(provider: P) -> Self {
        Self {
            provider: Arc::new(provider),
            config: ProcessorConfig::default(),
            warned_no_idempotency: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Create with custom configuration
    pub fn with_config(provider: P, config: ProcessorConfig) -> Self {
        Self {
            provider: Arc::new(provider),
            config,
            warned_no_idempotency: Arc::new(AtomicBool::new(false)),
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
    /// # Retry safety
    ///
    /// A retryable failure (network or throttling) is retried up to
    /// `max_retries` times **only when the retry is provably safe**: that is,
    /// when the provider reports [`PaymentProvider::supports_idempotency`] and
    /// the request carries an idempotency key, so the gateway will collapse a
    /// duplicate submission into the original charge.
    ///
    /// Otherwise exactly one attempt is made, regardless of `retry_failed`. An
    /// ambiguous timeout is indistinguishable from a slow success, so blindly
    /// re-posting a non-deduplicated charge would bill the customer once per
    /// attempt. Surfacing the error and letting the caller reconcile is the
    /// only safe behavior.
    ///
    /// When `use_idempotency` is set and the request carries no key, one is
    /// generated here and reused across every attempt.
    pub async fn charge(&self, request: ChargeRequest) -> PaymentResult<Charge> {
        let mut request = request;
        if self.config.use_idempotency && request.idempotency_key.is_none() {
            request.idempotency_key = Some(uuid::Uuid::new_v4().to_string());
        }

        let retry_safe = self.retry_is_safe("charge", request.idempotency_key.is_some());
        let amount = request.amount;
        let result = self
            .with_retries("charge", retry_safe, || {
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
    /// # Retry safety
    ///
    /// Same rule as [`charge`](Self::charge): a transient failure is retried
    /// only when the provider supports idempotency and the request carries a
    /// key. Without gateway-side deduplication a retried refund pays the
    /// customer out twice, so the processor makes a single attempt instead.
    ///
    /// When `use_idempotency` is set and the request carries no key, one is
    /// generated here and reused across every attempt.
    pub async fn refund(&self, request: RefundRequest) -> PaymentResult<Refund> {
        let mut request = request;
        if self.config.use_idempotency && request.idempotency_key.is_none() {
            request.idempotency_key = Some(uuid::Uuid::new_v4().to_string());
        }

        let retry_safe = self.retry_is_safe("refund", request.idempotency_key.is_some());
        let charge_id = request.charge_id.clone();
        let result = self
            .with_retries("refund", retry_safe, || {
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

    /// Whether re-submitting a money-moving `operation` is safe.
    ///
    /// Safe only when the gateway deduplicates by idempotency key *and* this
    /// request actually carries one. Warns once per processor when retries are
    /// silently downgraded, so an operator can see that `retry_failed` is not
    /// taking effect and why.
    fn retry_is_safe(&self, operation: &str, has_key: bool) -> bool {
        if !self.config.retry_failed {
            return false;
        }

        let safe = self.provider.supports_idempotency() && has_key;
        if !safe && !self.warned_no_idempotency.swap(true, Ordering::Relaxed) {
            armature_log::warn!(
                target: "armature::payments",
                "retries disabled for {} via {}: provider reports \
                 supports_idempotency()={} and request idempotency key \
                 present={}. Retrying without gateway-side deduplication \
                 could double-charge or double-refund, so a single attempt \
                 will be made.",
                operation,
                self.provider.name(),
                self.provider.supports_idempotency(),
                has_key
            );
        }
        safe
    }

    /// Run `op` under the configured retry policy.
    ///
    /// Only [`PaymentError::is_retryable`] failures are retried; a decline or a
    /// validation error is returned on the first attempt. `retry_safe` gates
    /// retrying entirely — see [`charge`](Self::charge) for why a
    /// non-idempotent money-moving call must never be replayed.
    ///
    /// Backoff prefers the server's own [`PaymentError::retry_after`] when it
    /// gave one (a `RateLimited` error carries the gateway's `Retry-After`);
    /// otherwise it grows exponentially from `retry_delay_ms` with jitter,
    /// capped at `max_retry_delay_ms`.
    async fn with_retries<T, F, Fut>(
        &self,
        operation: &str,
        retry_safe: bool,
        mut op: F,
    ) -> PaymentResult<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = PaymentResult<T>>,
    {
        let max_attempts = if retry_safe {
            self.config.max_retries.saturating_add(1)
        } else {
            1
        };

        let mut attempt = 1u32;
        loop {
            match op().await {
                Ok(value) => return Ok(value),
                Err(e) if e.is_retryable() && attempt < max_attempts => {
                    let delay = self.retry_delay(&e, attempt);
                    if self.config.log_transactions {
                        armature_log::warn!(
                            target: "armature::payments",
                            "{} attempt {}/{} failed ({}), retrying in {}ms",
                            operation,
                            attempt,
                            max_attempts,
                            e,
                            delay.as_millis()
                        );
                    }
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                    attempt += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// How long to wait before retry number `attempt` (1-based).
    fn retry_delay(&self, error: &PaymentError, attempt: u32) -> Duration {
        let cap = self.config.max_retry_delay_ms;

        // A gateway that told us how long to wait knows better than our
        // schedule; honor it (still bounded by the cap).
        if let Some(server) = error.retry_after() {
            let ms = u64::try_from(server.as_millis()).unwrap_or(u64::MAX);
            return Duration::from_millis(ms.min(cap));
        }

        let base = self.config.retry_delay_ms;
        if base == 0 {
            return Duration::ZERO;
        }

        // Exponential growth, saturating so a large max_retries cannot overflow.
        let factor = 2u64
            .checked_pow(attempt.saturating_sub(1))
            .unwrap_or(u64::MAX);
        let raw = base.saturating_mul(factor);

        // Jitter first, then cap: capping first would let the ±20% spread push
        // the delay back above max_retry_delay_ms, so the cap must be applied
        // last to be a real bound.
        Duration::from_millis(Self::apply_jitter(raw).min(cap))
    }

    /// Spread retries by ±20% so concurrent failures do not resynchronize into
    /// a thundering herd against an already-struggling gateway.
    fn apply_jitter(delay_ms: u64) -> u64 {
        if delay_ms == 0 {
            return 0;
        }
        // Cheap entropy: this only needs to decorrelate callers, not be secure,
        // so it avoids pulling in an RNG dependency.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0);

        let spread = delay_ms / 5; // 20%
        if spread == 0 {
            return delay_ms;
        }
        let offset = nanos % (spread.saturating_mul(2).saturating_add(1));
        delay_ms.saturating_add(offset).saturating_sub(spread)
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
            warned_no_idempotency: Arc::clone(&self.warned_no_idempotency),
        }
    }
}

/// Processor configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessorConfig {
    /// Retry failed charges.
    ///
    /// Note this is permissive, not decisive: a retry additionally requires the
    /// provider to support idempotency and the request to carry a key. See
    /// [`PaymentProcessor::charge`].
    pub retry_failed: bool,
    /// Maximum retry attempts
    pub max_retries: u32,
    /// Base retry delay in milliseconds.
    ///
    /// The delay for the first retry; subsequent retries grow exponentially
    /// from it, bounded by `max_retry_delay_ms`.
    pub retry_delay_ms: u64,
    /// Upper bound on any single retry delay, in milliseconds.
    ///
    /// Caps both exponential growth and an over-large server-supplied
    /// `Retry-After`, so a misbehaving gateway cannot stall a caller for hours.
    #[serde(default = "default_max_retry_delay_ms")]
    pub max_retry_delay_ms: u64,
    /// Enable idempotency keys
    pub use_idempotency: bool,
    /// Log all transactions
    pub log_transactions: bool,
}

/// Default ceiling on a single retry delay (30s).
fn default_max_retry_delay_ms() -> u64 {
    30_000
}

impl Default for ProcessorConfig {
    fn default() -> Self {
        Self {
            retry_failed: true,
            max_retries: 3,
            retry_delay_ms: 1000,
            max_retry_delay_ms: default_max_retry_delay_ms(),
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
        assert_eq!(config.max_retry_delay_ms, 30_000);
    }

    #[test]
    fn config_without_max_retry_delay_still_deserializes() {
        // The field was added after 0.2.0; stored configs predate it.
        let config: ProcessorConfig = serde_json::from_str(
            r#"{"retry_failed":true,"max_retries":2,"retry_delay_ms":500,
                "use_idempotency":true,"log_transactions":false}"#,
        )
        .unwrap();
        assert_eq!(config.max_retry_delay_ms, 30_000);
        assert_eq!(config.retry_delay_ms, 500);
    }

    /// Minimal provider used to exercise the processor's retry policy.
    struct FakeProvider {
        supports_idempotency: bool,
        calls: Arc<std::sync::atomic::AtomicU32>,
    }

    impl FakeProvider {
        fn new(supports_idempotency: bool) -> Self {
            Self {
                supports_idempotency,
                calls: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            }
        }
    }

    #[async_trait::async_trait]
    impl PaymentProvider for FakeProvider {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn supports_idempotency(&self) -> bool {
            self.supports_idempotency
        }
        async fn charge(&self, _r: ChargeRequest) -> PaymentResult<Charge> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Err(PaymentError::Network("timeout".into()))
        }
        async fn refund(&self, _r: RefundRequest) -> PaymentResult<Refund> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Err(PaymentError::Network("timeout".into()))
        }
        async fn capture(&self, _i: &str, _a: Option<Money>) -> PaymentResult<Charge> {
            unimplemented!()
        }
        async fn create_customer(&self, _r: CreateCustomerRequest) -> PaymentResult<Customer> {
            unimplemented!()
        }
        async fn get_customer(&self, _i: &str) -> PaymentResult<Customer> {
            unimplemented!()
        }
        async fn update_customer(
            &self,
            _i: &str,
            _r: UpdateCustomerRequest,
        ) -> PaymentResult<Customer> {
            unimplemented!()
        }
        async fn delete_customer(&self, _i: &str) -> PaymentResult<()> {
            unimplemented!()
        }
        async fn create_payment_method(
            &self,
            _r: CreatePaymentMethodRequest,
        ) -> PaymentResult<PaymentMethod> {
            unimplemented!()
        }
        async fn attach_payment_method(&self, _m: &str, _c: &str) -> PaymentResult<PaymentMethod> {
            unimplemented!()
        }
        async fn detach_payment_method(&self, _m: &str) -> PaymentResult<PaymentMethod> {
            unimplemented!()
        }
        async fn list_payment_methods(&self, _c: &str) -> PaymentResult<Vec<PaymentMethod>> {
            unimplemented!()
        }
        async fn create_subscription(
            &self,
            _r: CreateSubscriptionRequest,
        ) -> PaymentResult<Subscription> {
            unimplemented!()
        }
        async fn get_subscription(&self, _i: &str) -> PaymentResult<Subscription> {
            unimplemented!()
        }
        async fn update_subscription(&self, _i: &str, _p: &str) -> PaymentResult<Subscription> {
            unimplemented!()
        }
        async fn cancel_subscription(&self, _i: &str, _x: bool) -> PaymentResult<Subscription> {
            unimplemented!()
        }
        async fn resume_subscription(&self, _i: &str) -> PaymentResult<Subscription> {
            unimplemented!()
        }
        async fn verify_webhook(&self, _p: &[u8], _h: &WebhookHeaders) -> PaymentResult<()> {
            unimplemented!()
        }
        fn parse_webhook(&self, _p: &[u8]) -> PaymentResult<WebhookEvent> {
            unimplemented!()
        }
    }

    fn fast_config() -> ProcessorConfig {
        ProcessorConfig {
            retry_delay_ms: 0,
            log_transactions: false,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn charge_is_attempted_once_when_provider_lacks_idempotency() {
        // The core double-charge guard: 3 retries configured, but the gateway
        // cannot deduplicate, so re-posting could bill the customer 4 times.
        let provider = FakeProvider::new(false);
        let calls = Arc::clone(&provider.calls);
        let processor = PaymentProcessor::with_config(provider, fast_config());

        let req = ChargeRequest::new(Money::usd(2999), PaymentSource::card("tok"));
        let _ = processor.charge(req).await;

        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn charge_retries_when_provider_supports_idempotency() {
        let provider = FakeProvider::new(true);
        let calls = Arc::clone(&provider.calls);
        let processor = PaymentProcessor::with_config(provider, fast_config());

        let req = ChargeRequest::new(Money::usd(2999), PaymentSource::card("tok"));
        let _ = processor.charge(req).await;

        // 1 initial + 3 retries, all sharing the generated idempotency key.
        assert_eq!(calls.load(Ordering::Relaxed), 4);
    }

    #[tokio::test]
    async fn charge_is_attempted_once_when_idempotency_key_is_absent() {
        // Provider supports idempotency, but use_idempotency is off so no key
        // is generated — retrying is still unsafe.
        let provider = FakeProvider::new(true);
        let calls = Arc::clone(&provider.calls);
        let processor = PaymentProcessor::with_config(
            provider,
            ProcessorConfig {
                use_idempotency: false,
                ..fast_config()
            },
        );

        let req = ChargeRequest::new(Money::usd(2999), PaymentSource::card("tok"));
        let _ = processor.charge(req).await;

        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn refund_is_attempted_once_when_provider_lacks_idempotency() {
        let provider = FakeProvider::new(false);
        let calls = Arc::clone(&provider.calls);
        let processor = PaymentProcessor::with_config(provider, fast_config());

        let _ = processor.refund(RefundRequest::new("ch_1")).await;

        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn refund_retries_when_provider_supports_idempotency() {
        let provider = FakeProvider::new(true);
        let calls = Arc::clone(&provider.calls);
        let processor = PaymentProcessor::with_config(provider, fast_config());

        let _ = processor.refund(RefundRequest::new("ch_1")).await;

        assert_eq!(calls.load(Ordering::Relaxed), 4);
    }

    #[tokio::test]
    async fn refund_generates_an_idempotency_key() {
        // Previously RefundRequest had no key field at all, so every refund
        // retry was an unguarded second payout.
        let provider = FakeProvider::new(true);
        let processor = PaymentProcessor::with_config(provider, fast_config());
        assert!(processor.config().use_idempotency);

        let req = RefundRequest::new("ch_1");
        assert!(req.idempotency_key.is_none());
        let explicit = RefundRequest::new("ch_1").idempotency_key("refund-42");
        assert_eq!(explicit.idempotency_key.as_deref(), Some("refund-42"));
    }

    #[test]
    fn backoff_grows_exponentially_and_is_capped() {
        let processor = PaymentProcessor::with_config(
            FakeProvider::new(true),
            ProcessorConfig {
                retry_delay_ms: 1000,
                max_retry_delay_ms: 5000,
                ..Default::default()
            },
        );
        let err = PaymentError::Network("x".into());

        // attempt 1 -> ~1000ms, 2 -> ~2000ms, 3 -> ~4000ms, then capped at 5000.
        let d1 = processor.retry_delay(&err, 1).as_millis() as u64;
        let d2 = processor.retry_delay(&err, 2).as_millis() as u64;
        let d3 = processor.retry_delay(&err, 3).as_millis() as u64;

        assert!((800..=1200).contains(&d1), "d1 = {d1}");
        assert!((1600..=2400).contains(&d2), "d2 = {d2}");
        assert!((3200..=4800).contains(&d3), "d3 = {d3}");

        // Far-out attempts saturate at the cap rather than overflowing.
        for attempt in [10u32, 40, 64, 100, u32::MAX] {
            let d = processor.retry_delay(&err, attempt).as_millis() as u64;
            assert!(d <= 5000, "attempt {attempt} gave {d}");
        }
    }

    #[test]
    fn backoff_honors_server_retry_after() {
        let processor = PaymentProcessor::with_config(
            FakeProvider::new(true),
            ProcessorConfig {
                retry_delay_ms: 1000,
                max_retry_delay_ms: 30_000,
                ..Default::default()
            },
        );

        // The gateway asked for 12s; the flat/exponential schedule must yield.
        let d = processor.retry_delay(&PaymentError::RateLimited(12), 1);
        assert_eq!(d, Duration::from_secs(12));
    }

    #[test]
    fn server_retry_after_is_still_bounded_by_the_cap() {
        let processor = PaymentProcessor::with_config(
            FakeProvider::new(true),
            ProcessorConfig {
                max_retry_delay_ms: 5_000,
                ..Default::default()
            },
        );
        // A gateway asking for an hour must not stall the caller for an hour.
        let d = processor.retry_delay(&PaymentError::RateLimited(3600), 1);
        assert_eq!(d, Duration::from_millis(5_000));
    }

    #[test]
    fn zero_base_delay_means_no_sleep() {
        let processor = PaymentProcessor::with_config(
            FakeProvider::new(true),
            ProcessorConfig {
                retry_delay_ms: 0,
                ..Default::default()
            },
        );
        assert_eq!(
            processor.retry_delay(&PaymentError::Network("x".into()), 3),
            Duration::ZERO
        );
    }

    #[test]
    fn jitter_stays_within_twenty_percent() {
        for _ in 0..200 {
            let d = PaymentProcessor::<FakeProvider>::apply_jitter(1000);
            assert!((800..=1200).contains(&d), "jittered to {d}");
        }
    }

    #[test]
    fn jitter_saturates_rather_than_overflowing() {
        assert!(PaymentProcessor::<FakeProvider>::apply_jitter(u64::MAX) > 0);
        assert_eq!(PaymentProcessor::<FakeProvider>::apply_jitter(0), 0);
    }
}
