//! `ProcessorConfig` behavior.
//!
//! Regression tests for knobs that were stored and never read: `retry_failed`,
//! `max_retries`, `retry_delay_ms`, `use_idempotency` and `log_transactions`
//! had no effect on any call.

use armature_payments::{
    Charge, ChargeRequest, ChargeStatus, CreateCustomerRequest, CreatePaymentMethodRequest,
    CreateSubscriptionRequest, Customer, Money, PaymentError, PaymentMethod, PaymentProcessor,
    PaymentProvider, PaymentResult, PaymentSource, ProcessorConfig, Refund, RefundRequest,
    RefundStatus, Subscription, UpdateCustomerRequest, WebhookEvent, WebhookHeaders,
};
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

/// A provider that fails a scripted number of times before succeeding, and
/// records the idempotency key it saw on each attempt.
struct ScriptedProvider {
    failures_remaining: AtomicU32,
    error: fn() -> PaymentError,
    attempts: AtomicU32,
    keys_seen: Mutex<Vec<Option<String>>>,
    supports_idempotency: bool,
}

impl ScriptedProvider {
    /// A provider that *can* deduplicate a replayed request, so the retry gate
    /// is open and these tests exercise the retry policy itself.
    fn new(failures: u32, error: fn() -> PaymentError) -> Self {
        Self {
            failures_remaining: AtomicU32::new(failures),
            error,
            attempts: AtomicU32::new(0),
            keys_seen: Mutex::new(Vec::new()),
            supports_idempotency: true,
        }
    }

    /// A provider with no server-side deduplication — replaying a request
    /// against it moves money twice.
    fn without_idempotency(failures: u32, error: fn() -> PaymentError) -> Self {
        Self {
            supports_idempotency: false,
            ..Self::new(failures, error)
        }
    }

    fn attempts(&self) -> u32 {
        self.attempts.load(Ordering::SeqCst)
    }

    fn keys_seen(&self) -> Vec<Option<String>> {
        self.keys_seen.lock().unwrap().clone()
    }

    fn next_result(&self) -> PaymentResult<()> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if self
            .failures_remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                n.checked_sub(1).or(Some(0))
            })
            .is_ok_and(|n| n > 0)
        {
            return Err((self.error)());
        }
        Ok(())
    }
}

fn stub_charge() -> Charge {
    Charge {
        id: "ch_stub".into(),
        amount: Money::usd(1000),
        amount_refunded: Money::usd(0),
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
    }
}

#[async_trait]
impl PaymentProvider for ScriptedProvider {
    fn name(&self) -> &'static str {
        "scripted"
    }

    fn supports_idempotency(&self) -> bool {
        self.supports_idempotency
    }

    async fn charge(&self, request: ChargeRequest) -> PaymentResult<Charge> {
        self.keys_seen
            .lock()
            .unwrap()
            .push(request.idempotency_key.clone());
        self.next_result()?;
        Ok(stub_charge())
    }

    async fn refund(&self, request: RefundRequest) -> PaymentResult<Refund> {
        self.next_result()?;
        Ok(Refund {
            id: "re_stub".into(),
            charge_id: request.charge_id,
            amount: Money::usd(1000),
            status: RefundStatus::Succeeded,
            reason: None,
            created_at: Utc::now(),
        })
    }

    async fn capture(&self, _id: &str, _amount: Option<Money>) -> PaymentResult<Charge> {
        unimplemented!("not exercised")
    }
    async fn create_customer(&self, _r: CreateCustomerRequest) -> PaymentResult<Customer> {
        unimplemented!("not exercised")
    }
    async fn get_customer(&self, _id: &str) -> PaymentResult<Customer> {
        unimplemented!("not exercised")
    }
    async fn update_customer(
        &self,
        _id: &str,
        _r: UpdateCustomerRequest,
    ) -> PaymentResult<Customer> {
        unimplemented!("not exercised")
    }
    async fn delete_customer(&self, _id: &str) -> PaymentResult<()> {
        unimplemented!("not exercised")
    }
    async fn create_payment_method(
        &self,
        _r: CreatePaymentMethodRequest,
    ) -> PaymentResult<PaymentMethod> {
        unimplemented!("not exercised")
    }
    async fn attach_payment_method(&self, _m: &str, _c: &str) -> PaymentResult<PaymentMethod> {
        unimplemented!("not exercised")
    }
    async fn detach_payment_method(&self, _m: &str) -> PaymentResult<PaymentMethod> {
        unimplemented!("not exercised")
    }
    async fn list_payment_methods(&self, _c: &str) -> PaymentResult<Vec<PaymentMethod>> {
        unimplemented!("not exercised")
    }
    async fn create_subscription(
        &self,
        _r: CreateSubscriptionRequest,
    ) -> PaymentResult<Subscription> {
        unimplemented!("not exercised")
    }
    async fn get_subscription(&self, _id: &str) -> PaymentResult<Subscription> {
        unimplemented!("not exercised")
    }
    async fn update_subscription(&self, _id: &str, _p: &str) -> PaymentResult<Subscription> {
        unimplemented!("not exercised")
    }
    async fn cancel_subscription(&self, _id: &str, _i: bool) -> PaymentResult<Subscription> {
        unimplemented!("not exercised")
    }
    async fn resume_subscription(&self, _id: &str) -> PaymentResult<Subscription> {
        unimplemented!("not exercised")
    }
    async fn verify_webhook(&self, _p: &[u8], _h: &WebhookHeaders) -> PaymentResult<()> {
        Ok(())
    }
    fn parse_webhook(&self, _p: &[u8]) -> PaymentResult<WebhookEvent> {
        unimplemented!("not exercised")
    }
}

fn fast_config(retry_failed: bool, max_retries: u32) -> ProcessorConfig {
    ProcessorConfig {
        retry_failed,
        max_retries,
        retry_delay_ms: 0,
        use_idempotency: true,
        log_transactions: true,
        ..ProcessorConfig::default()
    }
}

fn charge_request() -> ChargeRequest {
    ChargeRequest::new(Money::usd(1000), PaymentSource::card("tok_visa"))
}

#[tokio::test]
async fn transient_failures_are_retried_up_to_max_retries() {
    let processor = PaymentProcessor::with_config(
        ScriptedProvider::new(2, || PaymentError::Network("connection reset".into())),
        fast_config(true, 3),
    );

    processor
        .charge(charge_request())
        .await
        .expect("the third attempt succeeds");

    assert_eq!(
        processor.provider().attempts(),
        3,
        "two transient failures must be retried"
    );
}

#[tokio::test]
async fn retries_stop_at_max_retries_and_surface_the_error() {
    let processor = PaymentProcessor::with_config(
        ScriptedProvider::new(99, || PaymentError::Network("down".into())),
        fast_config(true, 2),
    );

    let err = processor.charge(charge_request()).await.unwrap_err();
    assert!(matches!(err, PaymentError::Network(_)));
    assert_eq!(
        processor.provider().attempts(),
        3,
        "max_retries=2 means one initial attempt plus two retries"
    );
}

#[tokio::test]
async fn retry_failed_false_disables_retrying() {
    let processor = PaymentProcessor::with_config(
        ScriptedProvider::new(1, || PaymentError::Network("down".into())),
        fast_config(false, 5),
    );

    processor.charge(charge_request()).await.unwrap_err();
    assert_eq!(
        processor.provider().attempts(),
        1,
        "retry_failed=false must mean a single attempt"
    );
}

/// A decline is deterministic: retrying it just charges the customer's patience.
#[tokio::test]
async fn non_retryable_errors_are_not_retried() {
    for error in [
        (|| PaymentError::CardDeclined("declined".into())) as fn() -> PaymentError,
        || PaymentError::Validation("bad amount".into()),
        || PaymentError::Authentication("bad key".into()),
        || PaymentError::InsufficientFunds,
    ] {
        let processor =
            PaymentProcessor::with_config(ScriptedProvider::new(99, error), fast_config(true, 5));
        processor.charge(charge_request()).await.unwrap_err();
        assert_eq!(
            processor.provider().attempts(),
            1,
            "a deterministic failure must not be retried"
        );
    }
}

/// The retry gate: `retry_failed` and `max_retries` describe *how much* to
/// retry, but a provider that cannot deduplicate a replayed request must not be
/// retried at all, however permissive the config. Without this gate an ambiguous
/// timeout against a non-idempotent gateway produced up to `max_retries + 1`
/// real charges.
#[tokio::test]
async fn a_provider_without_idempotency_support_is_never_retried() {
    let processor = PaymentProcessor::with_config(
        ScriptedProvider::without_idempotency(99, || PaymentError::Network("timeout".into())),
        fast_config(true, 5),
    );

    let err = processor.charge(charge_request()).await.unwrap_err();
    assert!(
        err.is_retryable(),
        "the error must be retryable, or this proves nothing about the gate"
    );
    assert_eq!(
        processor.provider().attempts(),
        1,
        "a retryable error must still not be replayed against a gateway that \
         cannot deduplicate it"
    );
}

/// The same gate on the refund path.
#[tokio::test]
async fn a_refund_is_not_retried_without_idempotency_support() {
    let processor = PaymentProcessor::with_config(
        ScriptedProvider::without_idempotency(99, || PaymentError::Network("timeout".into())),
        fast_config(true, 5),
    );

    processor
        .refund(RefundRequest::new("ch_1"))
        .await
        .unwrap_err();
    assert_eq!(processor.provider().attempts(), 1);
}

#[tokio::test]
async fn rate_limits_are_retried() {
    let processor = PaymentProcessor::with_config(
        ScriptedProvider::new(1, || PaymentError::RateLimited(1)),
        fast_config(true, 3),
    );
    processor.charge(charge_request()).await.unwrap();
    assert_eq!(processor.provider().attempts(), 2);
}

/// Without a stable key, a retry after an ambiguous network failure can charge
/// the customer twice.
#[tokio::test]
async fn an_idempotency_key_is_generated_and_reused_across_retries() {
    let processor = PaymentProcessor::with_config(
        ScriptedProvider::new(2, || PaymentError::Network("timeout".into())),
        fast_config(true, 3),
    );

    processor.charge(charge_request()).await.unwrap();

    let keys = processor.provider().keys_seen();
    assert_eq!(keys.len(), 3);
    let first = keys[0].as_ref().expect("a key must be generated");
    assert!(
        keys.iter().all(|k| k.as_ref() == Some(first)),
        "every retry must reuse the same idempotency key, got {keys:?}"
    );
}

#[tokio::test]
async fn a_caller_supplied_idempotency_key_is_preserved() {
    let processor = PaymentProcessor::with_config(
        ScriptedProvider::new(0, || PaymentError::Network("unused".into())),
        fast_config(true, 3),
    );

    let mut request = charge_request();
    request.idempotency_key = Some("caller-key".into());
    processor.charge(request).await.unwrap();

    assert_eq!(
        processor.provider().keys_seen(),
        vec![Some("caller-key".to_string())]
    );
}

#[tokio::test]
async fn idempotency_can_be_disabled() {
    let mut config = fast_config(true, 3);
    config.use_idempotency = false;
    let processor = PaymentProcessor::with_config(
        ScriptedProvider::new(0, || PaymentError::Network("unused".into())),
        config,
    );

    processor.charge(charge_request()).await.unwrap();
    assert_eq!(processor.provider().keys_seen(), vec![None]);
}

#[tokio::test]
async fn refunds_are_retried_too() {
    let processor = PaymentProcessor::with_config(
        ScriptedProvider::new(1, || PaymentError::Network("reset".into())),
        fast_config(true, 3),
    );

    processor.refund(RefundRequest::new("ch_1")).await.unwrap();
    assert_eq!(processor.provider().attempts(), 2);
}

/// `retry_delay_ms` must actually delay between attempts, or a burst of retries
/// hammers a gateway that is already throttling us.
///
/// This ran on `std::time::Instant` and a real 80 ms sleep, which made it both
/// slow and load-dependent: a busy CI box could satisfy the bound without the
/// code sleeping at all, and a lost 40 ms sleep would still pass. Paused time
/// removes the scheduling noise — the clock advances only when the code under
/// test actually awaits a timer — so the bounds below constrain the backoff
/// schedule itself rather than the machine's load.
///
/// The schedule is exponential from `retry_delay_ms` with ±20% jitter, so two
/// retries cost `40ms + 80ms = 120ms` before jitter and land in
/// `[96ms, 144ms]` after it.
#[tokio::test(start_paused = true)]
async fn retry_delay_is_honored() {
    let mut config = fast_config(true, 2);
    config.retry_delay_ms = 40;
    let processor = PaymentProcessor::with_config(
        ScriptedProvider::new(2, || PaymentError::Network("reset".into())),
        config,
    );

    let started = tokio::time::Instant::now();
    processor.charge(charge_request()).await.unwrap();
    let waited = started.elapsed();

    assert!(
        waited >= std::time::Duration::from_millis(96),
        "two exponential retries from 40ms must wait at least 96ms even at the \
         low end of jitter, waited {waited:?}"
    );
    assert!(
        waited <= std::time::Duration::from_millis(144),
        "the backoff must not exceed the jittered exponential schedule, \
         waited {waited:?}"
    );
}

/// A gateway that sends `Retry-After` knows better than our own schedule, and
/// ignoring it is what gets an API key throttled harder.
#[tokio::test(start_paused = true)]
async fn a_server_supplied_retry_after_overrides_the_local_schedule() {
    let mut config = fast_config(true, 1);
    config.retry_delay_ms = 1; // Deliberately far below what the server asks.
    let processor = PaymentProcessor::with_config(
        ScriptedProvider::new(1, || PaymentError::RateLimited(2)),
        config,
    );

    let started = tokio::time::Instant::now();
    processor.charge(charge_request()).await.unwrap();

    assert_eq!(
        started.elapsed(),
        std::time::Duration::from_secs(2),
        "RateLimited(2) means the gateway asked for two seconds"
    );
}

/// The exponential schedule must stay bounded, or a long `max_retries` turns a
/// transient outage into a multi-minute hang.
#[tokio::test(start_paused = true)]
async fn backoff_is_capped_by_max_retry_delay_ms() {
    let mut config = fast_config(true, 6);
    config.retry_delay_ms = 1_000;
    config.max_retry_delay_ms = 2_000;
    let processor = PaymentProcessor::with_config(
        ScriptedProvider::new(6, || PaymentError::Network("reset".into())),
        config,
    );

    let started = tokio::time::Instant::now();
    processor.charge(charge_request()).await.unwrap();

    // Six retries, each capped at 2s, can never exceed 12s.
    assert!(
        started.elapsed() <= std::time::Duration::from_millis(12_000),
        "each retry must be capped at max_retry_delay_ms, waited {:?}",
        started.elapsed()
    );
}

/// With no delay configured, the retry loop must not sleep at all.
#[tokio::test(start_paused = true)]
async fn a_zero_retry_delay_does_not_sleep() {
    let processor = PaymentProcessor::with_config(
        ScriptedProvider::new(2, || PaymentError::Network("reset".into())),
        fast_config(true, 3),
    );

    let started = tokio::time::Instant::now();
    processor.charge(charge_request()).await.unwrap();

    assert_eq!(
        started.elapsed(),
        std::time::Duration::ZERO,
        "retry_delay_ms = 0 must not introduce a timer"
    );
}

// ------------------------------------------------------- log_transactions ---
//
// `log_transactions` was set to `true` by every test in this file and its effect
// was never observed, so the flag could have been ignored entirely — or worse,
// inverted — without a single failure.
//
// Observing it needs care: `armature_log` writes formatted records straight to
// `std::io::stderr()` with no injectable sink, so no in-process capture layer
// (a `tracing_subscriber` layer included — armature-log emits no tracing events)
// can see them. The only honest observation point is the process's real stderr,
// so these tests re-exec this test binary and read the child's fd 2.

/// Set on the child process to select the `log_transactions` value under test.
const LOG_CHILD_ENV: &str = "ARMATURE_PAYMENTS_LOG_CHILD";

/// The target every `PaymentProcessor` record is logged under.
const PAYMENTS_TARGET: &str = "armature::payments";

/// Run one successful charge in a child process with `log_transactions` set as
/// requested, and return everything it wrote to stderr.
fn charge_in_child(log_transactions: bool) -> String {
    let output = std::process::Command::new(
        std::env::current_exe().expect("the running test binary is re-executable"),
    )
    .args([
        "--exact",
        "logging_child_process",
        "--nocapture",
        "--test-threads=1",
    ])
    .env(LOG_CHILD_ENV, if log_transactions { "1" } else { "0" })
    .env("ARMATURE_LOG_LEVEL", "info")
    .env("ARMATURE_LOG_FORMAT", "json")
    .env("ARMATURE_LOG_COLOR", "0")
    .output()
    .expect("spawn the child test process");

    assert!(
        output.status.success(),
        "child process failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The child half of the two tests below. Inert unless `LOG_CHILD_ENV` is set,
/// so a normal `cargo test` run treats it as a trivially passing test.
#[tokio::test]
async fn logging_child_process() {
    let Ok(mode) = std::env::var(LOG_CHILD_ENV) else {
        return;
    };
    armature_log::init();

    let mut config = fast_config(false, 0);
    config.log_transactions = mode == "1";

    let processor = PaymentProcessor::with_config(
        ScriptedProvider::new(0, || PaymentError::CardExpired),
        config,
    );
    processor
        .charge(charge_request())
        .await
        .expect("the scripted provider succeeds on the first attempt");
}

/// `log_transactions: false` must silence the processor completely. A caller
/// disables it because charge records carry amounts and customer identifiers
/// they do not want on stderr; emitting them anyway is a privacy defect, and one
/// no test in this file could previously have caught.
#[test]
fn log_transactions_false_emits_nothing_on_the_payments_target() {
    let stderr = charge_in_child(false);

    assert!(
        !stderr.contains(PAYMENTS_TARGET),
        "log_transactions = false must emit no {PAYMENTS_TARGET} record, got:\n{stderr}"
    );
    assert!(
        !stderr.contains("ch_stub"),
        "no charge identifier may be logged when logging is disabled, got:\n{stderr}"
    );
}

/// The negative test above is only meaningful if the positive case genuinely
/// logs — otherwise it would pass against a processor that never logs at all.
#[test]
fn log_transactions_true_emits_a_payments_record() {
    let stderr = charge_in_child(true);

    assert!(
        stderr.contains(PAYMENTS_TARGET),
        "log_transactions = true must emit a {PAYMENTS_TARGET} record, got:\n{stderr}"
    );
    assert!(
        stderr.contains("ch_stub"),
        "the record must identify the charge, got:\n{stderr}"
    );
}

#[test]
fn config_is_readable() {
    let processor = PaymentProcessor::with_config(
        ScriptedProvider::new(0, || PaymentError::InsufficientFunds),
        fast_config(true, 7),
    );
    assert_eq!(processor.config().max_retries, 7);
}
