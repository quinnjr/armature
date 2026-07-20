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
}

impl ScriptedProvider {
    fn new(failures: u32, error: fn() -> PaymentError) -> Self {
        Self {
            failures_remaining: AtomicU32::new(failures),
            error,
            attempts: AtomicU32::new(0),
            keys_seen: Mutex::new(Vec::new()),
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

#[tokio::test]
async fn retry_delay_is_honored() {
    let mut config = fast_config(true, 2);
    config.retry_delay_ms = 40;
    let processor = PaymentProcessor::with_config(
        ScriptedProvider::new(2, || PaymentError::Network("reset".into())),
        config,
    );

    let started = std::time::Instant::now();
    processor.charge(charge_request()).await.unwrap();
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(80),
        "two retries at 40ms must wait at least 80ms, waited {:?}",
        started.elapsed()
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
