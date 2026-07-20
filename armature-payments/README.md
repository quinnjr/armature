# armature-payments

Payment processing for the Armature framework.

## Features

- **Stripe** — Charges, PaymentIntents, customers, payment methods, subscriptions, HMAC-SHA256 webhooks
- **PayPal** — Orders v2, captures, refunds, subscriptions, API-verified webhooks
- **Braintree** — Transactions, customers, subscriptions, HMAC-SHA1 webhook notifications
- **One API** — `PaymentProcessor` wraps any `PaymentProvider` with retry, backoff and idempotency policy
- **Money** — Minor-unit `Money` with per-currency precision, so a zero-decimal currency never gains cents

## Installation

```toml
[dependencies]
armature-payments = "0.2"
```

All three providers are on by default. Enable only what you need with
`default-features = false, features = ["stripe"]`.

## Quick Start

```rust,no_run
# #[cfg(feature = "stripe")]
# async fn example(
#     body: Vec<u8>,
#     signature_header: &str,
# ) -> Result<(), Box<dyn std::error::Error>> {
use armature_payments::providers::StripeProvider;
use armature_payments::{
    ChargeRequest, Money, PaymentProcessor, PaymentSource, WebhookHeaders,
};

// `new` is fallible because the provider's HTTP client always carries a
// request and connect timeout — it will not silently fall back to an untimed
// client that could hang a charge inside the retry loop.
let processor = PaymentProcessor::new(
    StripeProvider::new("sk_test_...")?.with_webhook_secret("whsec_..."),
);

let charge = processor
    .charge(
        ChargeRequest::new(Money::usd(2999), PaymentSource::card("tok_visa"))
            .description("Order #1234"),
    )
    .await?;
println!("charged {} -> {:?}", charge.amount, charge.status);

// Verification runs before parsing and fails closed, so pass the request's
// real headers — a forged webhook is rejected here, not after it is parsed.
let headers = WebhookHeaders::single("Stripe-Signature", signature_header);
let event = processor.handle_webhook(&body, &headers).await?;
println!("verified {:?}", event.event_type);
# Ok(())
# }
```

## Architecture

```text
┌─────────────────────────────────────────────────────────────────┐
│                    Payment Processing                            │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                 Unified Payment API                       │   │
│  │  charge() | refund() | subscribe() | cancel()             │   │
│  └──────────────────────────────────────────────────────────┘   │
│                            │                                     │
│         ┌──────────────────┼──────────────────┐                  │
│         ▼                  ▼                  ▼                  │
│  ┌────────────┐    ┌────────────┐    ┌────────────┐             │
│  │   Stripe   │    │   PayPal   │    │ Braintree  │             │
│  └────────────┘    └────────────┘    └────────────┘             │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                  Webhook Handler                          │   │
│  │  payment.succeeded | refund.created | subscription.*      │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

## Retry safety

A retryable failure is retried only when the replay is **provably safe**: the
provider must report `PaymentProvider::supports_idempotency()` and the request
must carry an idempotency key, so the gateway collapses the duplicate onto the
original transaction. Otherwise exactly one attempt is made regardless of
`ProcessorConfig::retry_failed` — an ambiguous timeout is indistinguishable from
a slow success, and re-posting a non-deduplicated charge bills the customer
twice. Stripe and PayPal deduplicate (`Idempotency-Key`, `PayPal-Request-Id`);
Braintree does not, so its charges and refunds are never replayed.

`PaymentError::is_retryable()` treats a gateway 5xx or 408 as transient, which is
exactly where an idempotency key earns its keep. A gateway-supplied `Retry-After`
overrides the local exponential backoff and is bounded only by
`MAX_SERVER_RETRY_AFTER_MS` (one hour) — capping it at `max_retry_delay_ms` would
just get the next attempt re-throttled.

## Webhooks

`PaymentProcessor::handle_webhook` verifies before it parses. Provider
`parse_webhook` implementations perform **no** authentication and must never be
called on an unverified body. Stripe additionally rejects signatures older than a
five-minute tolerance so a captured webhook cannot be replayed forever.

## Error redaction

Gateway error bodies routinely echo the request back — a PAN, an API key, an
`Authorization` header. Every error path clips the body to a bounded snippet and
redacts card-shaped digit runs (including `4242 4242 4242 4242` spacing),
`sk_`/`rk_`/`pk_`/`whsec_` tokens, JWTs, and `Bearer`/`Basic` credentials before
the text reaches an error string or a log line. It is a best-effort scrubber, not
a compliance boundary: keep card data out of your own logs too.

## License

MIT OR Apache-2.0
