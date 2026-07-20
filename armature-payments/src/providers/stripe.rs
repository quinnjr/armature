//! Stripe payment provider implementation

use crate::{
    error::{DeclineCode, PaymentError, PaymentResult},
    money::{Currency, Money},
    provider::{PaymentProvider, ProviderClient, retry_after_secs},
    types::*,
    webhook::{WebhookData, WebhookEvent, WebhookEventType, WebhookHeaders},
};
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use hmac::{Hmac, KeyInit, Mac};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use sha2::Sha256;
use std::collections::HashMap;

/// The header Stripe signs each webhook with.
pub const STRIPE_SIGNATURE_HEADER: &str = "stripe-signature";

/// Maximum number of `v1=` signature candidates accepted from a single
/// `Stripe-Signature` header. Stripe sends at most two (old and new) during a
/// secret rotation; each candidate costs a fresh HMAC-SHA256 over the entire
/// (attacker-controlled, pre-verification) payload, so an unbounded count is
/// a CPU-amplification vector.
const MAX_V1_CANDIDATES: usize = 8;

/// Maximum accepted length of a `Stripe-Signature` header, in bytes. Stripe's
/// own header is well under 200 bytes even during a rotation; anything larger
/// is a malformed or hostile header and is rejected before it is parsed.
const MAX_SIGNATURE_HEADER_LEN: usize = 1024;

/// Objects requested per page when walking a Stripe list endpoint. 100 is
/// Stripe's documented maximum; its default is 10.
const LIST_PAGE_SIZE: u32 = 100;

/// Pages a list walk will follow before giving up.
///
/// A gateway that always answers `has_more: true` would otherwise loop forever
/// inside a request handler. At [`LIST_PAGE_SIZE`] per page this still covers
/// far more payment methods than any real customer has.
const MAX_LIST_PAGES: usize = 20;

/// Stripe provider
pub struct StripeProvider {
    api_key: SecretString,
    webhook_secret: Option<SecretString>,
    webhook_tolerance: Option<chrono::Duration>,
    client: ProviderClient,
}

/// How a malformed gateway timestamp is handled.
///
/// `chrono`'s `timestamp_opt` returns `LocalResult::None` for a seconds value
/// outside the representable range, and `.unwrap()` on that **panics** — inside
/// a payment path, on a value the gateway (or a webhook sender) controls. Every
/// projection in this file therefore uses `.single()` and falls back to "now".
///
/// Falling back rather than propagating is deliberate and matches Braintree,
/// which treats an absent `created_at` the same way: the timestamp is metadata
/// on an operation that already succeeded, so failing the whole charge because
/// its `created` field was unrepresentable would turn a cosmetic problem into a
/// lost transaction. The amount, currency and status — the fields that decide
/// what happened to the money — are parsed strictly.
fn timestamp_or_now(secs: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(secs, 0).single().unwrap_or_else(Utc::now)
}

impl StripeProvider {
    /// Create a new Stripe provider.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::Config`] if the HTTP client cannot be built. It
    /// is fallible so the timeouts are guaranteed: the previous fallback to an
    /// untimed `reqwest::Client::new()` could hang a charge indefinitely inside
    /// the processor's retry loop. See [`ProviderClient::new`].
    pub fn new(api_key: impl Into<String>) -> PaymentResult<Self> {
        let api_key = api_key.into();
        Ok(Self {
            client: ProviderClient::new("https://api.stripe.com/v1", &api_key)?,
            api_key: SecretString::new(api_key.into()),
            webhook_secret: None,
            webhook_tolerance: Some(chrono::Duration::minutes(5)),
        })
    }

    /// Point the client at an alternate API base URL (a mock or proxy).
    ///
    /// The URL must be `https`, or `http` addressed to a loopback host
    /// (`localhost`, `127.0.0.1`, `[::1]`) for local testing. A cleartext
    /// `http` URL to any other host is rejected: every request carries the
    /// secret API key in an `Authorization` header, so an unencrypted base URL
    /// leaks `sk_live_…` to anyone on the path.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::Config`] if `base_url` is unparseable or uses a
    /// scheme/host combination that would transmit credentials in cleartext.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> PaymentResult<Self> {
        let base_url = crate::provider::validate_base_url(&base_url.into())?;
        self.client = ProviderClient::new(base_url, self.api_key.expose_secret())?;
        Ok(self)
    }

    /// Set webhook secret
    pub fn with_webhook_secret(mut self, secret: impl Into<String>) -> Self {
        self.webhook_secret = Some(SecretString::new(secret.into().into()));
        self
    }

    /// Maximum age a signed webhook timestamp may have before it is rejected as
    /// a replay. Defaults to five minutes.
    ///
    /// To turn the check off entirely you must say so explicitly with
    /// [`without_webhook_tolerance`](Self::without_webhook_tolerance); this
    /// method can no longer disable replay protection by accident.
    pub fn with_webhook_tolerance(mut self, tolerance: chrono::Duration) -> Self {
        self.webhook_tolerance = Some(tolerance);
        self
    }

    /// Disable webhook replay protection.
    ///
    /// # Security
    ///
    /// With no tolerance window, any webhook this endpoint ever received can be
    /// replayed verbatim forever — its signature stays valid indefinitely, so
    /// an attacker who captures one `charge.succeeded` can resubmit it without
    /// limit. Only use this where an external layer already enforces
    /// freshness/deduplication. Turning it off emits a warning.
    pub fn without_webhook_tolerance(mut self) -> Self {
        armature_log::warn!(
            "Stripe webhook replay protection disabled: signed payloads will be accepted \
             regardless of age. Captured webhooks can be replayed indefinitely."
        );
        self.webhook_tolerance = None;
        self
    }

    /// Create a payment intent
    pub async fn create_payment_intent(
        &self,
        request: ChargeRequest,
    ) -> PaymentResult<StripePaymentIntent> {
        let mut params: HashMap<String, String> = HashMap::new();
        params.insert("amount".into(), request.amount.amount.to_string());
        params.insert(
            "currency".into(),
            request.amount.currency.code().to_lowercase(),
        );

        if let Some(desc) = &request.description {
            params.insert("description".into(), desc.clone());
        }

        if let Some(customer_id) = &request.customer_id {
            params.insert("customer".into(), customer_id.clone());
        }
        if let Some(descriptor) = &request.statement_descriptor {
            params.insert("statement_descriptor".into(), descriptor.clone());
        }
        for (key, value) in &request.metadata {
            params.insert(format!("metadata[{key}]"), value.clone());
        }

        match &request.source {
            PaymentSource::PaymentMethod { id } => {
                params.insert("payment_method".into(), id.clone());
                params.insert("confirm".into(), "true".to_string());
                if !request.capture {
                    params.insert("capture_method".into(), "manual".to_string());
                }
            }
            PaymentSource::Customer { customer_id } => {
                params.insert("customer".into(), customer_id.clone());
            }
            PaymentSource::Card { .. } => {
                return Err(PaymentError::Validation(
                    "Stripe PaymentIntents do not accept raw card tokens; tokenize the card into a \
                     PaymentMethod first and use PaymentSource::PaymentMethod"
                        .into(),
                ));
            }
            PaymentSource::Bank { .. } => {
                return Err(PaymentError::Validation(
                    "Stripe PaymentIntents do not accept bank-account tokens directly; create a \
                     us_bank_account PaymentMethod and use PaymentSource::PaymentMethod"
                        .into(),
                ));
            }
        }

        let response = self
            .client
            .post_form_idempotent(
                "/payment_intents",
                &params,
                request.idempotency_key.as_deref(),
            )
            .await?;
        stripe_json(response).await
    }
}

/// Decode a Stripe response body, mapping any non-2xx status onto the error
/// Stripe actually reported rather than a downstream deserialization failure.
async fn stripe_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> PaymentResult<T> {
    let status = response.status();
    let retry_after = retry_after_secs(&response);
    let body = response.text().await?;
    if !status.is_success() {
        return Err(stripe_error(status, &body, retry_after));
    }
    serde_json::from_str(&body).map_err(|e| {
        // This arm fires on a *2xx* whose shape we failed to decode, so `body`
        // is a full Stripe resource — a Customer carries email, name, address;
        // a PaymentMethod carries card brand and last4. The error string ends
        // up in caller logs, so it gets only the redacted form; the full body
        // stays at debug level for operators who have opted into it.
        armature_log::debug!(
            "Stripe response failed to deserialize: {} (body: {})",
            e,
            body
        );
        PaymentError::Serialization(format!(
            "{e} (body: {})",
            crate::provider::sanitize_body(&body)
        ))
    })
}

/// As [`stripe_json`], but reporting a 404 as the resource-specific not-found
/// error.
///
/// Stripe's error envelope types a missing resource only as
/// `invalid_request_error`, which is indistinguishable from a malformed
/// request; the call site is the only place that knows *what* was missing.
async fn stripe_json_as<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    not_found: PaymentError,
) -> PaymentResult<T> {
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(not_found);
    }
    stripe_json(response).await
}

/// Check a Stripe response that carries no useful body, refining a 404 into a
/// resource-specific error.
///
/// Same rationale as [`stripe_json_as`]: Stripe reports a missing resource as a
/// generic `invalid_request_error`, so only the call site knows which resource
/// was absent. Without this, a delete of a missing customer surfaces as an
/// untyped `Provider` error while a *read* of the same customer surfaces as
/// `CustomerNotFound` — a divergence callers cannot reasonably code against.
async fn stripe_unit_as(response: reqwest::Response, not_found: PaymentError) -> PaymentResult<()> {
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(not_found);
    }
    stripe_unit(response).await
}

/// Check a Stripe response that carries no useful body.
async fn stripe_unit(response: reqwest::Response) -> PaymentResult<()> {
    let status = response.status();
    let retry_after = retry_after_secs(&response);
    if !status.is_success() {
        // A body that could not be read is not the same thing as an empty body:
        // `unwrap_or_default()` made a truncated response indistinguishable from
        // a gateway that legitimately sent nothing, which is exactly the
        // distinction someone debugging a failed charge needs.
        let body = response
            .text()
            .await
            .unwrap_or_else(|e| format!("<body unreadable: {e}>"));
        return Err(stripe_error(status, &body, retry_after));
    }
    Ok(())
}

/// Map a Stripe error body + HTTP status onto a typed [`PaymentError`].
fn stripe_error(status: reqwest::StatusCode, body: &str, retry_after: Option<u32>) -> PaymentError {
    // Fall back to one second only when Stripe sent no `Retry-After`.
    let backoff = retry_after.unwrap_or(1);
    let detail = serde_json::from_str::<StripeError>(body)
        .ok()
        .map(|e| e.error);

    if let Some(detail) = detail {
        if let Some(code) = detail.decline_code.as_deref().or(detail.code.as_deref()) {
            match DeclineCode::from_str(code) {
                DeclineCode::ExpiredCard => return PaymentError::CardExpired,
                DeclineCode::InsufficientFunds => return PaymentError::InsufficientFunds,
                _ => {}
            }
        }

        // Redact once, up front, and use it in *every* arm. The parsed-envelope
        // path is the common case, and it previously embedded `detail.message`
        // raw — but Stripe's `message` routinely echoes the parameter that was
        // submitted ("Invalid value for source: tok_…"), and the processor logs
        // these verbatim. Redacting only the no-envelope fallthrough scrubbed
        // the path that almost never fires.
        let message = crate::provider::sanitize_body(&detail.message);

        return match detail.error_type.as_deref() {
            Some("card_error") => PaymentError::CardDeclined(message),
            Some("authentication_error") => PaymentError::Authentication(message),
            Some("invalid_request_error") if status == reqwest::StatusCode::NOT_FOUND => {
                PaymentError::Provider(message)
            }
            Some("rate_limit_error") => PaymentError::RateLimited(backoff),
            _ if status == reqwest::StatusCode::TOO_MANY_REQUESTS => {
                PaymentError::RateLimited(backoff)
            }
            // 403 belongs here too. `classify_status` maps `401 | 403 =>
            // Authentication`, but this branch handled only 401, so a Stripe 403
            // landed in `Provider` while the identical status from PayPal or
            // Braintree came back as `Authentication` — a divergence callers
            // cannot code against.
            _ if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN =>
            {
                PaymentError::Authentication(message)
            }
            _ => PaymentError::Provider(message),
        };
    }

    // No parseable Stripe error envelope, so the raw body is all we have. It
    // may still be an unexpected resource dump or a proxy error page, so it goes
    // through the shared classifier, which redacts it. Hand-rolling the same
    // match here let the message format drift away from every other provider's.
    armature_log::debug!("Stripe returned {}: {}", status, body);
    crate::provider::classify_status("Stripe", status, body, retry_after)
}

#[async_trait]
impl PaymentProvider for StripeProvider {
    fn name(&self) -> &'static str {
        "stripe"
    }

    /// Stripe honours `Idempotency-Key` on every mutating endpoint this
    /// provider calls, so a retried charge or refund collapses onto the
    /// original result instead of moving money twice.
    fn supports_idempotency(&self) -> bool {
        true
    }

    async fn charge(&self, request: ChargeRequest) -> PaymentResult<Charge> {
        // A PaymentMethod source cannot be charged through the legacy Charges
        // API; route it through PaymentIntents rather than dropping it.
        if matches!(request.source, PaymentSource::PaymentMethod { .. }) {
            let amount = request.amount;
            let description = request.description.clone();
            let metadata = request.metadata.clone();
            let intent = self.create_payment_intent(request).await?;
            return Ok(charge_from_intent(intent, amount, description, metadata));
        }

        let mut params: HashMap<String, String> = HashMap::new();
        params.insert("amount".into(), request.amount.amount.to_string());
        params.insert(
            "currency".into(),
            request.amount.currency.code().to_lowercase(),
        );

        if let Some(desc) = &request.description {
            params.insert("description".into(), desc.clone());
        }

        if let Some(customer_id) = &request.customer_id {
            params.insert("customer".into(), customer_id.clone());
        }

        if let Some(descriptor) = &request.statement_descriptor {
            params.insert("statement_descriptor".into(), descriptor.clone());
        }

        for (key, value) in &request.metadata {
            params.insert(format!("metadata[{key}]"), value.clone());
        }

        match &request.source {
            // Both card and bank-account tokens are `source` on the Charges API.
            PaymentSource::Card { token } | PaymentSource::Bank { token } => {
                params.insert("source".into(), token.clone());
            }
            PaymentSource::Customer { customer_id } => {
                params.insert("customer".into(), customer_id.clone());
            }
            // Routed to PaymentIntents at the top of this function. Reaching
            // here would mean that guard was edited away; return an error
            // rather than panicking inside a payment path.
            PaymentSource::PaymentMethod { .. } => {
                return Err(PaymentError::Validation(
                    "PaymentMethod sources must be routed through create_payment_intent".into(),
                ));
            }
        }

        params.insert("capture".into(), request.capture.to_string());

        let response = self
            .client
            .post_form_idempotent("/charges", &params, request.idempotency_key.as_deref())
            .await?;

        let stripe_charge: StripeCharge = stripe_json(response).await?;
        Ok(stripe_charge.into())
    }

    async fn capture(&self, charge_id: &str, amount: Option<Money>) -> PaymentResult<Charge> {
        let mut params = HashMap::new();
        if let Some(amt) = amount {
            params.insert("amount", amt.amount.to_string());
        }

        let response = self
            .client
            .post_form(&format!("/charges/{}/capture", charge_id), &params)
            .await?;

        let stripe_charge: StripeCharge = stripe_json_as(
            response,
            PaymentError::ChargeNotFound(charge_id.to_string()),
        )
        .await?;
        Ok(stripe_charge.into())
    }

    async fn refund(&self, request: RefundRequest) -> PaymentResult<Refund> {
        let mut params = HashMap::new();
        params.insert("charge", request.charge_id.clone());

        if let Some(amount) = &request.amount {
            params.insert("amount", amount.amount.to_string());
        }

        if let Some(reason) = &request.reason {
            params.insert(
                "reason",
                match reason {
                    RefundReason::Duplicate => "duplicate",
                    RefundReason::Fraudulent => "fraudulent",
                    RefundReason::RequestedByCustomer => "requested_by_customer",
                }
                .to_string(),
            );
        }

        // A refund moves money, so a retry that reaches Stripe twice must
        // collapse onto one refund rather than returning the customer's money
        // twice.
        let response = self
            .client
            .post_form_idempotent("/refunds", &params, request.idempotency_key.as_deref())
            .await?;
        let stripe_refund: StripeRefund =
            stripe_json_as(response, PaymentError::ChargeNotFound(request.charge_id)).await?;
        Ok(stripe_refund.into())
    }

    async fn create_customer(&self, request: CreateCustomerRequest) -> PaymentResult<Customer> {
        let mut params = HashMap::new();

        if let Some(email) = &request.email {
            params.insert("email", email.clone());
        }
        if let Some(name) = &request.name {
            params.insert("name", name.clone());
        }
        if let Some(phone) = &request.phone {
            params.insert("phone", phone.clone());
        }
        if let Some(desc) = &request.description {
            params.insert("description", desc.clone());
        }

        let response = self.client.post_form("/customers", &params).await?;
        let stripe_customer: StripeCustomer = stripe_json(response).await?;
        Ok(stripe_customer.into())
    }

    async fn get_customer(&self, id: &str) -> PaymentResult<Customer> {
        let response = self.client.get(&format!("/customers/{}", id)).await?;
        let stripe_customer: StripeCustomer =
            stripe_json_as(response, PaymentError::CustomerNotFound(id.to_string())).await?;
        Ok(stripe_customer.into())
    }

    async fn update_customer(
        &self,
        id: &str,
        request: UpdateCustomerRequest,
    ) -> PaymentResult<Customer> {
        let mut params = HashMap::new();

        if let Some(email) = &request.email {
            params.insert("email", email.clone());
        }
        if let Some(name) = &request.name {
            params.insert("name", name.clone());
        }
        if let Some(phone) = &request.phone {
            params.insert("phone", phone.clone());
        }

        let response = self
            .client
            .post_form(&format!("/customers/{}", id), &params)
            .await?;
        let stripe_customer: StripeCustomer =
            stripe_json_as(response, PaymentError::CustomerNotFound(id.to_string())).await?;
        Ok(stripe_customer.into())
    }

    async fn delete_customer(&self, id: &str) -> PaymentResult<()> {
        let response = self.client.delete(&format!("/customers/{}", id)).await?;
        stripe_unit_as(response, PaymentError::CustomerNotFound(id.to_string())).await
    }

    async fn create_payment_method(
        &self,
        request: CreatePaymentMethodRequest,
    ) -> PaymentResult<PaymentMethod> {
        let mut params = HashMap::new();
        params.insert("type", "card".to_string());

        if let Some(card) = &request.card {
            params.insert("card[number]", card.number.clone());
            params.insert("card[exp_month]", card.exp_month.to_string());
            params.insert("card[exp_year]", card.exp_year.to_string());
            params.insert("card[cvc]", card.cvc.clone());
        }

        let response = self.client.post_form("/payment_methods", &params).await?;
        let stripe_pm: StripePaymentMethod = stripe_json(response).await?;
        Ok(stripe_pm.into())
    }

    async fn attach_payment_method(
        &self,
        method_id: &str,
        customer_id: &str,
    ) -> PaymentResult<PaymentMethod> {
        let mut params = HashMap::new();
        params.insert("customer", customer_id.to_string());

        let response = self
            .client
            .post_form(&format!("/payment_methods/{}/attach", method_id), &params)
            .await?;
        let stripe_pm: StripePaymentMethod = stripe_json_as(
            response,
            PaymentError::PaymentMethodNotFound(method_id.to_string()),
        )
        .await?;
        Ok(stripe_pm.into())
    }

    async fn detach_payment_method(&self, method_id: &str) -> PaymentResult<PaymentMethod> {
        let response = self
            .client
            .post_form(
                &format!("/payment_methods/{}/detach", method_id),
                &HashMap::<String, String>::new(),
            )
            .await?;
        let stripe_pm: StripePaymentMethod = stripe_json_as(
            response,
            PaymentError::PaymentMethodNotFound(method_id.to_string()),
        )
        .await?;
        Ok(stripe_pm.into())
    }

    /// List a customer's card payment methods, following Stripe's pagination.
    ///
    /// Stripe's list endpoints default to `limit: 10` and report `has_more`
    /// alongside a cursor. The previous implementation issued one unpaginated
    /// GET and read neither field, so a customer with 11 or more saved cards
    /// silently lost every card past the tenth — a wallet UI would render an
    /// incomplete list with no error to say so.
    ///
    /// Pages are requested at Stripe's maximum `limit` of 100 and walked via
    /// `starting_after`, bounded by [`MAX_LIST_PAGES`] so a gateway that keeps
    /// answering `has_more: true` (or replays the same page, leaving the cursor
    /// unchanged) cannot spin forever inside a request handler.
    async fn list_payment_methods(&self, customer_id: &str) -> PaymentResult<Vec<PaymentMethod>> {
        let mut methods: Vec<PaymentMethod> = Vec::new();
        let mut starting_after: Option<String> = None;

        for _ in 0..MAX_LIST_PAGES {
            let mut path = format!(
                "/payment_methods?customer={}&type=card&limit={}",
                customer_id, LIST_PAGE_SIZE
            );
            if let Some(cursor) = &starting_after {
                path.push_str("&starting_after=");
                path.push_str(cursor);
            }

            let response = self.client.get(&path).await?;
            let list: StripeList<StripePaymentMethod> = stripe_json(response).await?;

            // The cursor is the id of the last object on the page, so it has to
            // be taken before the page is consumed.
            let last_id = list.data.last().map(|pm| pm.id.clone());
            methods.extend(list.data.into_iter().map(Into::into));

            match (list.has_more, last_id) {
                (true, Some(id)) => starting_after = Some(id),
                // `has_more` with an empty page would loop with an unchanged
                // cursor; stop rather than spin.
                _ => break,
            }
        }

        Ok(methods)
    }

    async fn create_subscription(
        &self,
        request: CreateSubscriptionRequest,
    ) -> PaymentResult<Subscription> {
        let mut params = HashMap::new();
        params.insert("customer", request.customer_id.clone());
        params.insert("items[0][price]", request.price_id.clone());

        if let Some(qty) = request.quantity {
            params.insert("items[0][quantity]", qty.to_string());
        }

        if let Some(days) = request.trial_days {
            params.insert("trial_period_days", days.to_string());
        }

        if let Some(pm) = &request.payment_method {
            params.insert("default_payment_method", pm.clone());
        }

        let response = self.client.post_form("/subscriptions", &params).await?;
        let stripe_sub: StripeSubscription = stripe_json(response).await?;
        Ok(stripe_sub.into())
    }

    async fn get_subscription(&self, id: &str) -> PaymentResult<Subscription> {
        let response = self.client.get(&format!("/subscriptions/{}", id)).await?;
        let stripe_sub: StripeSubscription =
            stripe_json_as(response, PaymentError::SubscriptionNotFound(id.to_string())).await?;
        Ok(stripe_sub.into())
    }

    async fn update_subscription(&self, id: &str, price_id: &str) -> PaymentResult<Subscription> {
        let mut params = HashMap::new();
        params.insert("items[0][price]", price_id.to_string());

        let response = self
            .client
            .post_form(&format!("/subscriptions/{}", id), &params)
            .await?;
        let stripe_sub: StripeSubscription = stripe_json(response).await?;
        Ok(stripe_sub.into())
    }

    async fn cancel_subscription(&self, id: &str, immediate: bool) -> PaymentResult<Subscription> {
        if immediate {
            let response = self
                .client
                .delete(&format!("/subscriptions/{}", id))
                .await?;
            let stripe_sub: StripeSubscription = stripe_json(response).await?;
            Ok(stripe_sub.into())
        } else {
            let mut params = HashMap::new();
            params.insert("cancel_at_period_end", "true".to_string());

            let response = self
                .client
                .post_form(&format!("/subscriptions/{}", id), &params)
                .await?;
            let stripe_sub: StripeSubscription = stripe_json(response).await?;
            Ok(stripe_sub.into())
        }
    }

    async fn resume_subscription(&self, id: &str) -> PaymentResult<Subscription> {
        let mut params = HashMap::new();
        params.insert("cancel_at_period_end", "false".to_string());

        let response = self
            .client
            .post_form(&format!("/subscriptions/{}", id), &params)
            .await?;
        let stripe_sub: StripeSubscription = stripe_json(response).await?;
        Ok(stripe_sub.into())
    }

    async fn verify_webhook(&self, payload: &[u8], headers: &WebhookHeaders) -> PaymentResult<()> {
        let secret = self
            .webhook_secret
            .as_ref()
            .ok_or(PaymentError::Config("Webhook secret not configured".into()))?;
        // An empty secret would HMAC every payload with a zero-length key,
        // which is not a real secret at all — reject it the same way an
        // absent secret is rejected, rather than letting any signature over
        // an empty-string-keyed HMAC verify.
        if secret.expose_secret().is_empty() {
            return Err(PaymentError::Config(
                "Webhook secret must not be empty".into(),
            ));
        }

        let signature = headers.require(STRIPE_SIGNATURE_HEADER)?;

        // Bound the header before parsing it. A 10 MB header is not a Stripe
        // signature under any rotation, and rejecting it up front keeps the
        // allocation below proportional to a legitimate header.
        if signature.len() > MAX_SIGNATURE_HEADER_LEN {
            return Err(PaymentError::InvalidWebhookSignature);
        }

        // Parse the Stripe-Signature header: `t=<unix>,v1=<hex>[,v1=<hex>...]`.
        // Stripe may send several v1 signatures during a secret rotation, so
        // every candidate is checked.
        let mut timestamp: Option<&str> = None;
        let mut candidates: Vec<&str> = Vec::new();
        for part in signature.split(',') {
            match part.trim().split_once('=') {
                Some(("t", v)) => timestamp = Some(v),
                Some(("v1", v)) => {
                    // Enforce the cap *inside* the loop. Checking it only after
                    // collecting let a header full of `v1=` pairs build the
                    // entire Vec before it was rejected.
                    if candidates.len() == MAX_V1_CANDIDATES {
                        return Err(PaymentError::InvalidWebhookSignature);
                    }
                    candidates.push(v);
                }
                _ => {}
            }
        }

        let timestamp = timestamp.ok_or(PaymentError::InvalidWebhookSignature)?;
        if candidates.is_empty() {
            return Err(PaymentError::InvalidWebhookSignature);
        }

        // Reject stale timestamps: without this, a captured webhook can be
        // replayed forever with its original (still valid) signature.
        if let Some(tolerance) = self.webhook_tolerance {
            let signed_at: i64 = timestamp
                .parse()
                .map_err(|_| PaymentError::InvalidWebhookSignature)?;
            let age = Utc::now().timestamp() - signed_at;
            if age.abs() > tolerance.num_seconds() {
                return Err(PaymentError::InvalidWebhookSignature);
            }
        }

        let mut signed_payload = Vec::with_capacity(timestamp.len() + 1 + payload.len());
        signed_payload.extend_from_slice(timestamp.as_bytes());
        signed_payload.push(b'.');
        signed_payload.extend_from_slice(payload);

        for candidate in candidates {
            let Ok(expected) = hex::decode(candidate) else {
                continue;
            };
            let mut mac = Hmac::<Sha256>::new_from_slice(secret.expose_secret().as_bytes())
                .map_err(|_| PaymentError::InvalidWebhookSignature)?;
            mac.update(&signed_payload);
            // `verify_slice` compares in constant time; a byte-wise `!=` on the
            // hex string leaks the shared secret one byte at a time under a
            // timing attack.
            if mac.verify_slice(&expected).is_ok() {
                return Ok(());
            }
        }

        Err(PaymentError::InvalidWebhookSignature)
    }

    fn parse_webhook(&self, payload: &[u8]) -> PaymentResult<WebhookEvent> {
        let event: StripeWebhookEvent = serde_json::from_slice(payload)?;
        let event_type = WebhookEventType::from_str(&event.event_type);

        Ok(WebhookEvent {
            id: event.id,
            // Interpret the object according to the event type instead of
            // always emitting `Generic`. `from_event_type` is documented as the
            // thing providers should call, but no provider called it, so the
            // five typed `WebhookData` variants were unreachable through the
            // crate's own webhook path. It falls back to `Generic` losslessly
            // when the payload does not match the typed shape.
            data: WebhookData::from_event_type(&event_type, event.data.object),
            event_type,
            created_at: timestamp_or_now(event.created),
            provider: "stripe".to_string(),
            livemode: event.livemode,
        })
    }
}

// Stripe API types

#[derive(Debug, Deserialize)]
struct StripeError {
    error: StripeErrorDetail,
}

#[derive(Debug, Deserialize)]
struct StripeErrorDetail {
    #[serde(default)]
    message: String,
    #[serde(rename = "type")]
    error_type: Option<String>,
    code: Option<String>,
    decline_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StripeList<T> {
    data: Vec<T>,
    /// Stripe's pagination flag. Absent on a non-list response, so it defaults
    /// to "this is everything" rather than failing the parse.
    #[serde(default)]
    has_more: bool,
}

#[derive(Debug, Deserialize)]
struct StripeCharge {
    id: String,
    amount: i64,
    currency: String,
    status: String,
    customer: Option<String>,
    payment_method: Option<String>,
    description: Option<String>,
    receipt_url: Option<String>,
    failure_message: Option<String>,
    captured: bool,
    refunded: bool,
    disputed: bool,
    created: i64,
    #[serde(default)]
    metadata: HashMap<String, String>,
    amount_refunded: Option<i64>,
}

impl From<StripeCharge> for Charge {
    fn from(sc: StripeCharge) -> Self {
        let currency = Currency::from_code(&sc.currency).unwrap_or(Currency::USD);
        Self {
            id: sc.id,
            amount: Money::new(sc.amount, currency),
            amount_refunded: Money::new(sc.amount_refunded.unwrap_or(0), currency),
            status: match sc.status.as_str() {
                "succeeded" => ChargeStatus::Succeeded,
                "failed" => ChargeStatus::Failed,
                "pending" => ChargeStatus::Pending,
                _ => ChargeStatus::Pending,
            },
            customer_id: sc.customer,
            payment_method: sc.payment_method,
            description: sc.description,
            receipt_url: sc.receipt_url,
            failure_reason: sc.failure_message,
            metadata: sc.metadata,
            created_at: timestamp_or_now(sc.created),
            captured: sc.captured,
            refunded: sc.refunded,
            disputed: sc.disputed,
        }
    }
}

#[derive(Debug, Deserialize)]
struct StripeRefund {
    id: String,
    charge: String,
    amount: i64,
    currency: String,
    status: String,
    reason: Option<String>,
    created: i64,
}

impl From<StripeRefund> for Refund {
    fn from(sr: StripeRefund) -> Self {
        let currency = Currency::from_code(&sr.currency).unwrap_or(Currency::USD);
        Self {
            id: sr.id,
            charge_id: sr.charge,
            amount: Money::new(sr.amount, currency),
            status: match sr.status.as_str() {
                "succeeded" => RefundStatus::Succeeded,
                "failed" => RefundStatus::Failed,
                "pending" => RefundStatus::Pending,
                "canceled" => RefundStatus::Canceled,
                _ => RefundStatus::Pending,
            },
            reason: sr.reason.and_then(|r| match r.as_str() {
                "duplicate" => Some(RefundReason::Duplicate),
                "fraudulent" => Some(RefundReason::Fraudulent),
                "requested_by_customer" => Some(RefundReason::RequestedByCustomer),
                _ => None,
            }),
            created_at: timestamp_or_now(sr.created),
        }
    }
}

#[derive(Debug, Deserialize)]
struct StripeCustomer {
    id: String,
    email: Option<String>,
    name: Option<String>,
    phone: Option<String>,
    description: Option<String>,
    default_source: Option<String>,
    created: i64,
    #[serde(default)]
    metadata: HashMap<String, String>,
}

impl From<StripeCustomer> for Customer {
    fn from(sc: StripeCustomer) -> Self {
        Self {
            id: sc.id,
            email: sc.email,
            name: sc.name,
            phone: sc.phone,
            description: sc.description,
            default_payment_method: sc.default_source,
            address: None,
            metadata: sc.metadata,
            created_at: timestamp_or_now(sc.created),
        }
    }
}

#[derive(Debug, Deserialize)]
struct StripePaymentMethod {
    id: String,
    #[allow(dead_code)]
    #[serde(rename = "type")]
    method_type: String,
    customer: Option<String>,
    card: Option<StripeCard>,
    created: i64,
}

#[derive(Debug, Deserialize)]
struct StripeCard {
    brand: String,
    last4: String,
    exp_month: u32,
    exp_year: u32,
    funding: String,
}

impl From<StripePaymentMethod> for PaymentMethod {
    fn from(spm: StripePaymentMethod) -> Self {
        Self {
            id: spm.id,
            method_type: PaymentMethodType::Card,
            customer_id: spm.customer,
            card: spm.card.map(|c| CardInfo {
                brand: c.brand,
                last4: c.last4,
                exp_month: c.exp_month,
                exp_year: c.exp_year,
                funding: match c.funding.as_str() {
                    "credit" => CardFunding::Credit,
                    "debit" => CardFunding::Debit,
                    "prepaid" => CardFunding::Prepaid,
                    _ => CardFunding::Unknown,
                },
            }),
            billing_details: None,
            created_at: timestamp_or_now(spm.created),
        }
    }
}

#[derive(Debug, Deserialize)]
struct StripeSubscription {
    id: String,
    customer: String,
    status: String,
    current_period_start: i64,
    current_period_end: i64,
    trial_end: Option<i64>,
    cancel_at_period_end: bool,
    canceled_at: Option<i64>,
    created: i64,
    #[serde(default)]
    metadata: HashMap<String, String>,
    #[serde(default)]
    items: StripeSubscriptionItems,
}

#[derive(Debug, Default, Deserialize)]
struct StripeSubscriptionItems {
    #[serde(default)]
    data: Vec<StripeSubscriptionItem>,
}

#[derive(Debug, Deserialize)]
struct StripeSubscriptionItem {
    price: Option<StripePrice>,
    quantity: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct StripePrice {
    id: String,
}

impl From<StripeSubscription> for Subscription {
    fn from(ss: StripeSubscription) -> Self {
        // The plan and quantity live on the first subscription item; reading
        // them is the only way to report what the customer is actually
        // subscribed to.
        let first_item = ss.items.data.into_iter().next();
        let price_id = first_item
            .as_ref()
            .and_then(|i| i.price.as_ref())
            .map(|p| p.id.clone())
            .unwrap_or_default();
        let quantity = first_item.as_ref().and_then(|i| i.quantity).unwrap_or(1);

        Self {
            id: ss.id,
            customer_id: Some(ss.customer),
            status: match ss.status.as_str() {
                "active" => SubscriptionStatus::Active,
                "trialing" => SubscriptionStatus::Trialing,
                "past_due" => SubscriptionStatus::PastDue,
                "canceled" => SubscriptionStatus::Canceled,
                "unpaid" => SubscriptionStatus::Unpaid,
                "incomplete" => SubscriptionStatus::Incomplete,
                "incomplete_expired" => SubscriptionStatus::IncompleteExpired,
                "paused" => SubscriptionStatus::Paused,
                _ => SubscriptionStatus::Active,
            },
            current_period_start: Utc.timestamp_opt(ss.current_period_start, 0).single(),
            current_period_end: Utc.timestamp_opt(ss.current_period_end, 0).single(),
            trial_end: ss.trial_end.and_then(|t| Utc.timestamp_opt(t, 0).single()),
            cancel_at_period_end: ss.cancel_at_period_end,
            canceled_at: ss
                .canceled_at
                .and_then(|t| Utc.timestamp_opt(t, 0).single()),
            price_id,
            quantity,
            metadata: ss.metadata,
            created_at: Utc.timestamp_opt(ss.created, 0).single(),
        }
    }
}

/// A Stripe PaymentIntent.
#[derive(Debug, Deserialize)]
pub struct StripePaymentIntent {
    /// PaymentIntent ID.
    pub id: String,
    /// Amount in the currency's minor unit.
    pub amount: i64,
    /// ISO currency code.
    pub currency: String,
    /// Intent status (`succeeded`, `requires_capture`, ...).
    pub status: String,
    /// Client secret for completing the intent in the browser.
    pub client_secret: Option<String>,
    /// Attached customer, if any.
    #[serde(default)]
    pub customer: Option<String>,
    /// Attached payment method, if any.
    #[serde(default)]
    pub payment_method: Option<String>,
    /// Creation time, as a Unix timestamp.
    #[serde(default)]
    pub created: Option<i64>,
    /// The amount already captured.
    #[serde(default)]
    pub amount_received: Option<i64>,
    /// Failure detail when the intent could not be confirmed.
    #[serde(default)]
    pub last_payment_error: Option<StripeErrorDetailPublic>,
}

/// Failure detail attached to a PaymentIntent.
#[derive(Debug, Deserialize)]
pub struct StripeErrorDetailPublic {
    /// Human-readable failure message.
    #[serde(default)]
    pub message: Option<String>,
}

/// Project a confirmed PaymentIntent onto the crate's [`Charge`] shape.
fn charge_from_intent(
    intent: StripePaymentIntent,
    requested: Money,
    description: Option<String>,
    metadata: HashMap<String, String>,
) -> Charge {
    let currency = Currency::from_code(&intent.currency).unwrap_or(requested.currency);
    let status = match intent.status.as_str() {
        "succeeded" => ChargeStatus::Succeeded,
        "canceled" => ChargeStatus::Canceled,
        _ => ChargeStatus::Pending,
    };
    Charge {
        id: intent.id,
        amount: Money::new(intent.amount, currency),
        amount_refunded: Money::new(0, currency),
        status,
        customer_id: intent.customer,
        payment_method: intent.payment_method,
        description,
        receipt_url: None,
        failure_reason: intent.last_payment_error.and_then(|e| e.message),
        metadata,
        created_at: intent
            .created
            .and_then(|t| Utc.timestamp_opt(t, 0).single())
            .unwrap_or_else(Utc::now),
        captured: intent.amount_received.unwrap_or(0) > 0,
        refunded: false,
        disputed: false,
    }
}

#[derive(Debug, Deserialize)]
struct StripeWebhookEvent {
    id: String,
    #[serde(rename = "type")]
    event_type: String,
    created: i64,
    livemode: bool,
    data: StripeWebhookData,
}

#[derive(Debug, Deserialize)]
struct StripeWebhookData {
    object: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> StripeProvider {
        StripeProvider::new("sk_test_123").expect("HTTP client builds")
    }

    // ---- finding 1: base URL validation ----

    #[test]
    fn with_base_url_rejects_cleartext_http() {
        // Every request carries sk_live_… as a bearer token.
        assert!(provider().with_base_url("http://evil.example.com").is_err());
        assert!(provider().with_base_url("ftp://example.com").is_err());
        assert!(provider().with_base_url("not a url").is_err());
    }

    #[test]
    fn with_base_url_accepts_https_and_loopback() {
        assert!(
            provider()
                .with_base_url("https://api.stripe.com/v1")
                .is_ok()
        );
        assert!(provider().with_base_url("http://127.0.0.1:8080").is_ok());
        assert!(provider().with_base_url("http://localhost:8080").is_ok());
    }

    #[test]
    fn with_base_url_accepts_a_trailing_slash() {
        // `validate_base_url` normalizes it away so request paths join cleanly.
        assert!(provider().with_base_url("http://localhost:8080/").is_ok());
    }

    // ---- finding 2: idempotency ----

    #[test]
    fn advertises_idempotency_support() {
        assert!(provider().supports_idempotency());
    }

    // ---- finding 7: Retry-After drives the backoff ----

    #[test]
    fn rate_limit_uses_the_retry_after_header() {
        let body = r#"{"error":{"type":"rate_limit_error","message":"slow down"}}"#;
        let err = stripe_error(reqwest::StatusCode::TOO_MANY_REQUESTS, body, Some(30));
        assert!(matches!(err, PaymentError::RateLimited(30)), "got {err:?}");
    }

    #[test]
    fn rate_limit_falls_back_to_one_second_without_the_header() {
        let body = r#"{"error":{"type":"rate_limit_error","message":"slow down"}}"#;
        let err = stripe_error(reqwest::StatusCode::TOO_MANY_REQUESTS, body, None);
        assert!(matches!(err, PaymentError::RateLimited(1)), "got {err:?}");
    }

    #[test]
    fn bare_429_without_an_error_envelope_still_uses_retry_after() {
        let err = stripe_error(reqwest::StatusCode::TOO_MANY_REQUESTS, "<html/>", Some(12));
        assert!(matches!(err, PaymentError::RateLimited(12)), "got {err:?}");
    }

    // ---- finding 13: bodies are redacted before entering errors/logs ----

    #[test]
    fn unparseable_error_bodies_are_sanitized() {
        // A card-number-shaped digit run must not survive into the error text.
        let body = r#"{"customer":"alice@example.com","card":"4242424242424242"}"#;
        let err = stripe_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR, body, None);
        let text = err.to_string();
        assert!(!text.contains("4242424242424242"), "leaked PAN: {text}");
        assert!(text.contains("[redacted]"), "got {text}");
    }

    #[test]
    fn auth_failures_are_sanitized_too() {
        let body = r#"{"token":"4111111111111111"}"#;
        let err = stripe_error(reqwest::StatusCode::UNAUTHORIZED, body, None);
        assert!(
            matches!(err, PaymentError::Authentication(_)),
            "got {err:?}"
        );
        assert!(!err.to_string().contains("4111111111111111"));
    }

    #[test]
    fn parsed_envelope_messages_are_sanitized_too() {
        // The common case. Stripe's `message` echoes the submitted parameter
        // back — "Invalid value for source: tok_…" — and the processor logs
        // these verbatim, so redacting only the no-envelope fallthrough scrubbed
        // the path that almost never fires.
        let body = r#"{"error":{"type":"invalid_request_error",
            "message":"Invalid API key provided: sk_live_abcdefghijklmnop"}}"#;
        let err = stripe_error(reqwest::StatusCode::BAD_REQUEST, body, None);
        let text = err.to_string();
        assert!(
            !text.contains("sk_live_abcdefghijklmnop"),
            "key leaked: {text}"
        );
        assert!(text.contains("[redacted]"), "got {text}");

        // Every arm, not just the fallthrough one.
        let declined = r#"{"error":{"type":"card_error",
            "message":"Your card 4242424242424242 was declined"}}"#;
        let err = stripe_error(reqwest::StatusCode::PAYMENT_REQUIRED, declined, None);
        assert!(matches!(err, PaymentError::CardDeclined(_)), "got {err:?}");
        assert!(
            !err.to_string().contains("4242424242424242"),
            "PAN leaked: {err}"
        );

        let auth = r#"{"error":{"type":"authentication_error",
            "message":"key sk_live_abcdefghijklmnop is invalid"}}"#;
        let err = stripe_error(reqwest::StatusCode::UNAUTHORIZED, auth, None);
        assert!(!err.to_string().contains("sk_live_abcdefghijklmnop"));
    }

    #[test]
    fn a_forbidden_response_is_an_authentication_error() {
        // `classify_status` maps 401 | 403 => Authentication, but the typed
        // branch handled only 401, so a Stripe 403 landed in `Provider` while
        // the same status from PayPal or Braintree came back as Authentication.
        let body = r#"{"error":{"type":"invalid_request_error","message":"no access"}}"#;
        let err = stripe_error(reqwest::StatusCode::FORBIDDEN, body, None);
        assert!(
            matches!(err, PaymentError::Authentication(_)),
            "a 403 must classify the same way on every provider; got {err:?}"
        );
        assert!(!err.is_retryable());

        // And without an envelope, via the shared classifier.
        let err = stripe_error(reqwest::StatusCode::FORBIDDEN, "<html/>", None);
        assert!(
            matches!(err, PaymentError::Authentication(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn typed_stripe_errors_are_still_classified() {
        let expired =
            r#"{"error":{"type":"card_error","decline_code":"expired_card","message":"expired"}}"#;
        assert!(matches!(
            stripe_error(reqwest::StatusCode::PAYMENT_REQUIRED, expired, None),
            PaymentError::CardExpired
        ));

        let funds =
            r#"{"error":{"type":"card_error","decline_code":"insufficient_funds","message":"no"}}"#;
        assert!(matches!(
            stripe_error(reqwest::StatusCode::PAYMENT_REQUIRED, funds, None),
            PaymentError::InsufficientFunds
        ));

        let auth = r#"{"error":{"type":"authentication_error","message":"bad key"}}"#;
        assert!(matches!(
            stripe_error(reqwest::StatusCode::UNAUTHORIZED, auth, None),
            PaymentError::Authentication(_)
        ));
    }

    // ---- finding 17: signature parsing is bounded ----

    #[tokio::test]
    async fn oversized_signature_header_is_rejected() {
        let provider = provider().with_webhook_secret("whsec_test");
        // A 10 MB header must not build a million-entry Vec before being
        // rejected; it must be refused on length alone.
        let huge = format!("t=1,{}", "v1=aa,".repeat(200_000));
        assert!(huge.len() > MAX_SIGNATURE_HEADER_LEN);

        let headers = WebhookHeaders::single(STRIPE_SIGNATURE_HEADER, huge);
        let err = provider.verify_webhook(b"{}", &headers).await.unwrap_err();
        assert!(
            matches!(err, PaymentError::InvalidWebhookSignature),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn too_many_v1_candidates_are_rejected() {
        let provider = provider().with_webhook_secret("whsec_test");
        // Within the length cap, but more v1 candidates than allowed.
        let sig = format!("t=1,{}", "v1=aa,".repeat(MAX_V1_CANDIDATES + 5));
        assert!(sig.len() <= MAX_SIGNATURE_HEADER_LEN);

        let headers = WebhookHeaders::single(STRIPE_SIGNATURE_HEADER, sig);
        let err = provider.verify_webhook(b"{}", &headers).await.unwrap_err();
        assert!(
            matches!(err, PaymentError::InvalidWebhookSignature),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn a_valid_signature_still_verifies() {
        use hmac::Mac;

        let secret = "whsec_test";
        let provider = provider().with_webhook_secret(secret);
        let payload = br#"{"id":"evt_1"}"#;
        let timestamp = Utc::now().timestamp();

        let mut signed = Vec::new();
        signed.extend_from_slice(timestamp.to_string().as_bytes());
        signed.push(b'.');
        signed.extend_from_slice(payload);

        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(&signed);
        let sig = hex::encode(mac.finalize().into_bytes());

        let headers =
            WebhookHeaders::single(STRIPE_SIGNATURE_HEADER, format!("t={timestamp},v1={sig}"));
        provider.verify_webhook(payload, &headers).await.unwrap();
    }

    #[tokio::test]
    async fn stale_signatures_are_rejected_by_default() {
        use hmac::Mac;

        let secret = "whsec_test";
        let provider = provider().with_webhook_secret(secret);
        // Well outside the default five-minute tolerance.
        let timestamp = Utc::now().timestamp() - 3600;

        let payload = br#"{"id":"evt_1"}"#;
        let mut signed = Vec::new();
        signed.extend_from_slice(timestamp.to_string().as_bytes());
        signed.push(b'.');
        signed.extend_from_slice(payload);

        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(&signed);
        let sig = hex::encode(mac.finalize().into_bytes());

        let headers =
            WebhookHeaders::single(STRIPE_SIGNATURE_HEADER, format!("t={timestamp},v1={sig}"));
        let err = provider
            .verify_webhook(payload, &headers)
            .await
            .unwrap_err();
        assert!(
            matches!(err, PaymentError::InvalidWebhookSignature),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn replay_protection_can_be_disabled_explicitly() {
        use hmac::Mac;

        let secret = "whsec_test";
        // Only the explicit opt-out disables the check; with_webhook_tolerance
        // can no longer do it by accident.
        let provider = provider()
            .with_webhook_secret(secret)
            .without_webhook_tolerance();
        let timestamp = Utc::now().timestamp() - 86_400;

        let payload = br#"{"id":"evt_1"}"#;
        let mut signed = Vec::new();
        signed.extend_from_slice(timestamp.to_string().as_bytes());
        signed.push(b'.');
        signed.extend_from_slice(payload);

        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(&signed);
        let sig = hex::encode(mac.finalize().into_bytes());

        let headers =
            WebhookHeaders::single(STRIPE_SIGNATURE_HEADER, format!("t={timestamp},v1={sig}"));
        provider.verify_webhook(payload, &headers).await.unwrap();
    }

    #[test]
    fn with_webhook_tolerance_takes_a_duration() {
        let p = provider().with_webhook_tolerance(chrono::Duration::minutes(1));
        assert_eq!(p.webhook_tolerance, Some(chrono::Duration::minutes(1)));
    }

    // ---- out-of-range gateway timestamps must not panic ----

    #[test]
    fn an_out_of_range_timestamp_does_not_panic() {
        // `Utc.timestamp_opt(i64::MAX, 0)` is LocalResult::None, and `.unwrap()`
        // on that panics — which used to take out the caller's task from inside
        // a payment path, on a value the gateway controls.
        for secs in [i64::MAX, i64::MIN, 253_402_300_800_000] {
            let stamped = timestamp_or_now(secs);
            assert!(stamped.timestamp().abs() < 253_402_300_800);
        }
        // A representable timestamp is still reported faithfully.
        assert_eq!(timestamp_or_now(1_700_000_000).timestamp(), 1_700_000_000);
    }

    #[test]
    fn a_charge_with_an_unrepresentable_created_still_projects() {
        let body = format!(
            r#"{{"id":"ch_1","amount":2999,"currency":"usd","status":"succeeded",
                "customer":null,"payment_method":null,"description":null,
                "receipt_url":null,"failure_message":null,"captured":true,
                "refunded":false,"disputed":false,"created":{}}}"#,
            i64::MAX
        );
        let stripe_charge: StripeCharge = serde_json::from_str(&body).unwrap();
        // Previously this panicked rather than returning a Charge.
        let charge: Charge = stripe_charge.into();
        assert_eq!(charge.amount, Money::usd(2999));
        assert_eq!(charge.status, ChargeStatus::Succeeded);
    }

    #[tokio::test]
    async fn a_webhook_with_an_unrepresentable_created_does_not_panic() {
        let payload = format!(
            r#"{{"id":"evt_1","type":"charge.succeeded","created":{},
                "livemode":false,"data":{{"object":{{"id":"ch_1"}}}}}}"#,
            i64::MIN
        );
        let event = provider().parse_webhook(payload.as_bytes()).unwrap();
        assert_eq!(event.id, "evt_1");
    }

    // ---- webhook data is typed, not always Generic ----

    #[test]
    fn parse_webhook_emits_typed_data_for_a_modeled_event() {
        // `WebhookData::from_event_type` was documented as the thing providers
        // should call, but no provider called it, so the typed variants were
        // unreachable through the crate's own webhook path.
        let payload = br#"{"id":"evt_1","type":"charge.succeeded","created":1700000000,
            "livemode":true,"data":{"object":{"id":"ch_1","amount":2999,
            "currency":"usd","status":"succeeded"}}}"#;
        let event = provider().parse_webhook(payload).unwrap();

        assert_eq!(event.event_type, WebhookEventType::ChargeSucceeded);
        match event.data {
            WebhookData::Charge(c) => {
                assert_eq!(c.id, "ch_1");
                assert_eq!(c.amount, 2999);
            }
            other => panic!("expected typed Charge data, got {other:?}"),
        }
    }

    #[test]
    fn parse_webhook_falls_back_to_generic_on_an_unmodeled_shape() {
        let payload = br#"{"id":"evt_2","type":"charge.succeeded","created":1700000000,
            "livemode":false,"data":{"object":{"totally":"different"}}}"#;
        let event = provider().parse_webhook(payload).unwrap();
        match event.data {
            WebhookData::Generic(v) => assert_eq!(v["totally"], "different"),
            other => panic!("expected Generic, got {other:?}"),
        }
    }

    // ---- finding 17: no panic in the payment path ----

    #[tokio::test]
    async fn payment_method_source_never_panics_in_charge() {
        // Routed to PaymentIntents; the old code had an unreachable!() here.
        // Point at a loopback port nothing is listening on so the call fails as
        // a network error rather than panicking.
        let provider = provider().with_base_url("http://127.0.0.1:1").unwrap();
        let request = ChargeRequest::new(Money::usd(1000), PaymentSource::payment_method("pm_123"));
        let err = provider.charge(request).await.unwrap_err();
        assert!(matches!(err, PaymentError::Network(_)), "got {err:?}");
    }
}
