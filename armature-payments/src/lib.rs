#![doc = include_str!("../README.md")]

pub mod error;
pub mod money;
pub mod provider;
pub mod types;
pub mod webhook;

pub mod providers;

pub use error::*;
pub use money::*;
pub use types::*;
pub use webhook::*;

// Deliberately *not* `pub use provider::*`. The glob dragged `sanitize_body`,
// `classify_status` and `retry_after_secs` into the published API, where semver
// would freeze them — `sanitize_body` worst of all, since it is a best-effort
// redaction heuristic whose thresholds and prefix list need to stay tunable, and
// exporting it advertises it as a security control. Those three are now
// `pub(crate)`; what a third-party `PaymentProvider` implementor genuinely needs
// is re-exported by name.
pub use provider::{
    PaymentProvider, ProviderClient, ProviderConfig, build_http_client, validate_base_url,
};

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
    ///
    /// A gateway-supplied `Retry-After` wins outright and is bounded only by
    /// [`MAX_SERVER_RETRY_AFTER_MS`], **not** by `max_retry_delay_ms` — see that
    /// constant for why. `max_retry_delay_ms` bounds the local exponential
    /// schedule alone.
    fn retry_delay(&self, error: &PaymentError, attempt: u32) -> Duration {
        // `max_retry_delay_ms` is `#[serde(default)]`, so a stored config can
        // carry 0 — and a cap of 0 makes the exponential branch below return
        // Duration::ZERO, firing max_retries + 1 requests back-to-back with no
        // pause at a gateway that just answered 429. A cap below the base delay
        // is a misconfiguration, not an instruction to remove the backoff, so
        // the floor is the base delay itself.
        let cap = self
            .config
            .max_retry_delay_ms
            .max(self.config.retry_delay_ms);

        // A gateway that told us how long to wait knows better than our
        // schedule; honor it, bounded only by MAX_SERVER_RETRY_AFTER_MS.
        // Clipping it to `cap` re-throttles: retrying a `Retry-After: 300`
        // after the local 30s just earns another 429 and burns quota.
        if let Some(server) = error.retry_after() {
            let ms = u64::try_from(server.as_millis()).unwrap_or(u64::MAX);
            if ms > MAX_SERVER_RETRY_AFTER_MS {
                armature_log::warn!(
                    target: "armature::payments",
                    "gateway Retry-After of {}ms exceeds the {}ms ceiling; capping",
                    ms,
                    MAX_SERVER_RETRY_AFTER_MS
                );
            }
            return Duration::from_millis(ms.min(MAX_SERVER_RETRY_AFTER_MS));
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
    /// Upper bound on the *locally scheduled* retry delay, in milliseconds.
    ///
    /// Bounds the exponential schedule only. A gateway-supplied `Retry-After`
    /// deliberately overrides it and is bounded instead by
    /// [`MAX_SERVER_RETRY_AFTER_MS`] — capping the gateway's own figure at the
    /// local maximum just re-throttles the next attempt.
    #[serde(default = "default_max_retry_delay_ms")]
    pub max_retry_delay_ms: u64,
    /// Enable idempotency keys
    pub use_idempotency: bool,
    /// Log all transactions
    pub log_transactions: bool,
}

/// Default ceiling on a single *locally scheduled* retry delay (30s).
fn default_max_retry_delay_ms() -> u64 {
    30_000
}

/// Ceiling on a gateway-supplied `Retry-After`, in milliseconds (one hour).
///
/// A gateway's `Retry-After` wins over the local backoff — retrying a
/// `Retry-After: 300` after `max_retry_delay_ms` (30s by default) just earns
/// another 429 — so it is deliberately *not* bounded by
/// [`ProcessorConfig::max_retry_delay_ms`]. But it arrives from outside the
/// process, and uncapped a `Retry-After: 999999999` parks a charge for roughly
/// 31 years. One hour is far above any legitimate throttle window, so this
/// ceiling only bites on a hostile or broken header.
///
/// The direct counterpart of `armature_mail::MAX_SERVER_RETRY_AFTER`, which
/// holds the same value for the same reason; keep the two in step.
pub const MAX_SERVER_RETRY_AFTER_MS: u64 = 3_600_000;

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
    ///
    /// Records the idempotency key of every `charge`/`refund` it is handed, so
    /// tests can assert on what the processor *actually sent* rather than
    /// re-deriving it from the request they built.
    struct FakeProvider {
        supports_idempotency: bool,
        calls: Arc<std::sync::atomic::AtomicU32>,
        keys: Arc<std::sync::Mutex<Vec<Option<String>>>>,
    }

    impl FakeProvider {
        fn new(supports_idempotency: bool) -> Self {
            Self {
                supports_idempotency,
                calls: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                keys: Arc::new(std::sync::Mutex::new(Vec::new())),
            }
        }

        fn record(&self, key: Option<String>) {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.keys.lock().unwrap().push(key);
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
        async fn charge(&self, r: ChargeRequest) -> PaymentResult<Charge> {
            self.record(r.idempotency_key);
            Err(PaymentError::Network("timeout".into()))
        }
        async fn refund(&self, r: RefundRequest) -> PaymentResult<Refund> {
            self.record(r.idempotency_key);
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
    async fn refund_generates_an_idempotency_key_and_reuses_it_across_retries() {
        // Previously RefundRequest had no key field at all, so every refund
        // retry was an unguarded second payout. This asserts through the
        // provider — the old version of this test never called `refund` at all
        // and only re-checked the builder it had just called.
        let provider = FakeProvider::new(true);
        let keys = Arc::clone(&provider.keys);
        let processor = PaymentProcessor::with_config(provider, fast_config());
        assert!(processor.config().use_idempotency);

        let request = RefundRequest::new("ch_1");
        assert!(request.idempotency_key.is_none(), "nothing supplied a key");
        let _ = processor.refund(request).await;

        let seen = keys.lock().unwrap().clone();
        assert_eq!(seen.len(), 4, "1 attempt + 3 retries; got {seen:?}");
        let first = seen[0]
            .as_deref()
            .expect("the processor must generate a refund key when none is supplied");
        assert!(!first.is_empty(), "an empty key deduplicates nothing");
        assert!(
            seen.iter().all(|k| k.as_deref() == Some(first)),
            "every retried refund must reuse one key or the gateway pays out \
             twice; got {seen:?}"
        );
    }

    #[tokio::test]
    async fn a_caller_supplied_refund_key_is_not_replaced() {
        let provider = FakeProvider::new(true);
        let keys = Arc::clone(&provider.keys);
        let processor = PaymentProcessor::with_config(provider, fast_config());

        let _ = processor
            .refund(RefundRequest::new("ch_1").idempotency_key("refund-42"))
            .await;

        let seen = keys.lock().unwrap().clone();
        assert!(!seen.is_empty());
        assert!(
            seen.iter().all(|k| k.as_deref() == Some("refund-42")),
            "the caller's key ties the refund to their ledger entry; got {seen:?}"
        );
    }

    #[test]
    fn a_zero_retry_delay_cap_does_not_erase_the_backoff() {
        // max_retry_delay_ms is #[serde(default)], so a stored config can carry
        // 0. Capping at 0 made both branches of retry_delay return ZERO, firing
        // four requests back-to-back at a gateway that just said 429.
        let processor = PaymentProcessor::with_config(
            FakeProvider::new(true),
            ProcessorConfig {
                retry_delay_ms: 1000,
                max_retry_delay_ms: 0,
                ..Default::default()
            },
        );

        let scheduled = processor.retry_delay(&PaymentError::Network("x".into()), 1);
        assert!(!scheduled.is_zero(), "backoff erased by a zero cap");

        // The same holds when the gateway supplied its own Retry-After.
        let server = processor.retry_delay(&PaymentError::RateLimited(5), 1);
        assert!(!server.is_zero(), "a 429 was retried with no pause at all");
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
    fn server_retry_after_is_not_clipped_to_the_local_cap() {
        // Capping the gateway's own figure at max_retry_delay_ms re-throttles:
        // coming back after 5s when the gateway said 300s just earns another
        // 429 and burns quota. armature-mail reached the same conclusion and
        // named the same ceiling; the two crates must not disagree.
        let processor = PaymentProcessor::with_config(
            FakeProvider::new(true),
            ProcessorConfig {
                max_retry_delay_ms: 5_000,
                ..Default::default()
            },
        );
        assert_eq!(
            processor.retry_delay(&PaymentError::RateLimited(300), 1),
            Duration::from_secs(300),
            "the gateway's Retry-After must survive the local cap"
        );
    }

    #[test]
    fn an_absurd_server_retry_after_is_bounded_by_one_hour() {
        let processor = PaymentProcessor::with_config(FakeProvider::new(true), fast_config());
        // Roughly 31 years, uncapped.
        let d = processor.retry_delay(&PaymentError::RateLimited(999_999_999), 1);
        assert_eq!(d, Duration::from_millis(MAX_SERVER_RETRY_AFTER_MS));
        // Exactly at the ceiling is honored verbatim.
        assert_eq!(
            processor.retry_delay(&PaymentError::RateLimited(3600), 1),
            Duration::from_secs(3600)
        );
    }

    #[test]
    fn the_local_schedule_is_still_bounded_by_max_retry_delay() {
        // Loosening the server branch must not loosen the exponential one.
        let processor = PaymentProcessor::with_config(
            FakeProvider::new(true),
            ProcessorConfig {
                retry_delay_ms: 1_000,
                max_retry_delay_ms: 5_000,
                ..Default::default()
            },
        );
        let err = PaymentError::Network("x".into());
        for attempt in [5u32, 10, 64] {
            assert!(processor.retry_delay(&err, attempt).as_millis() as u64 <= 5_000);
        }
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
