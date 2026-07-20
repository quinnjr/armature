//! Payment provider trait and common functionality

use crate::{
    error::{PaymentError, PaymentResult},
    types::*,
    webhook::{WebhookEvent, WebhookHeaders},
};
use async_trait::async_trait;
use std::time::Duration;

/// Request timeout applied to every provider HTTP call.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Connect timeout applied to every provider HTTP call.
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Longest gateway response body echoed into an error message.
const MAX_BODY_SNIPPET: usize = 512;

/// How much of an untrusted body [`sanitize_body`] will even look at.
///
/// The output is capped at [`MAX_BODY_SNIPPET`] regardless, so scanning further
/// buys nothing — but scanning is proportional to the *input*, and the input is
/// a gateway response body reached on the error path inside the retry loop. A
/// hostile or MITM'd gateway that answers a charge with a 10 MB HTML error page
/// would otherwise cost tens of megabytes of transient allocation per attempt.
/// Four times the snippet gives redaction enough context to work with while
/// keeping the whole operation O(2 KiB).
const MAX_BODY_SCAN: usize = MAX_BODY_SNIPPET * 4;

/// Marker appended when a body was cut short.
const TRUNCATION_MARKER: &str = "…[truncated]";

/// Replacement for anything that looks like a secret or a card number.
const REDACTION: &str = "[redacted]";

/// Payment provider trait
///
/// Implement this trait for each payment gateway (Stripe, PayPal, etc.)
#[async_trait]
pub trait PaymentProvider: Send + Sync {
    /// Get provider name
    fn name(&self) -> &'static str;

    /// Whether this gateway honors a request-level idempotency key on **both**
    /// `charge` and `refund`.
    ///
    /// This gates retrying. [`PaymentProcessor`](crate::PaymentProcessor) will
    /// only re-send a charge or refund after an ambiguous failure when this is
    /// `true` and the request actually carries a key — because without
    /// server-side deduplication a retry after a timeout is indistinguishable
    /// from a second real transaction, and re-posting it double-charges the
    /// customer.
    ///
    /// Defaults to `false`: a provider must opt in deliberately, and only after
    /// confirming the gateway deduplicates the key on both operations.
    fn supports_idempotency(&self) -> bool {
        false
    }

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

    /// Verify a webhook's authenticity against the raw request body and headers.
    ///
    /// Implementations MUST fail closed: a missing, malformed or mismatched
    /// signature returns [`crate::PaymentError::InvalidWebhookSignature`].
    /// This is async because some providers (PayPal) authenticate a webhook by
    /// calling back into their API.
    async fn verify_webhook(&self, payload: &[u8], headers: &WebhookHeaders) -> PaymentResult<()>;

    /// Parse webhook payload
    ///
    /// # Security
    ///
    /// This method performs **no authentication whatsoever**. It trusts
    /// `payload` completely and will happily parse an attacker-forged body
    /// into a [`WebhookEvent`]. It MUST only ever be called on a payload that
    /// has already passed [`verify_webhook`](Self::verify_webhook)
    /// successfully — calling it directly on an unverified body reproduces
    /// the forged-webhook vulnerability this trait's split of verify/parse
    /// exists to close. Callers should use
    /// [`PaymentProcessor::handle_webhook`](crate::PaymentProcessor::handle_webhook),
    /// which enforces this ordering, rather than calling `parse_webhook`
    /// directly.
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

/// Build the HTTP client every provider should use.
///
/// A bare `reqwest::Client::new()` has **no request or connect timeout**. Inside
/// a retry loop that is unbounded hang: a gateway that accepts the connection
/// and then stops responding wedges `charge()` forever, holding the caller's
/// task and any transaction state open. This applies a 30s overall request
/// timeout and a 10s connect timeout so a dead gateway surfaces as a retryable
/// `Network` error instead.
pub fn build_http_client() -> PaymentResult<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .build()
        .map_err(|e| PaymentError::Config(format!("failed to build HTTP client: {e}")))
}

/// Validate a provider base URL, returning it ready to join request paths onto.
///
/// API keys are sent on every request as bearer credentials, so the transport
/// must be encrypted. Plaintext `http` is permitted **only** for loopback hosts,
/// where the traffic never leaves the machine — that keeps local mock gateways
/// and integration tests working without opening a door to shipping live
/// credentials in the clear. Every other scheme (including `ftp`, `file` and
/// `data`) and any non-loopback `http` host is rejected.
///
/// The returned string has any trailing slash stripped, so callers can keep
/// building request URLs as `format!("{base}{path}")` with a leading-slash path
/// without producing a double slash.
pub fn validate_base_url(url: &str) -> PaymentResult<String> {
    let parsed = url::Url::parse(url)
        .map_err(|e| PaymentError::Config(format!("invalid base URL {url:?}: {e}")))?;

    match parsed.scheme() {
        "https" => Ok(normalize_base(url)),
        "http" => {
            let host = parsed.host_str().unwrap_or_default();
            if is_loopback_host(host) {
                Ok(normalize_base(url))
            } else {
                Err(PaymentError::Config(format!(
                    "insecure base URL {url:?}: plaintext http is only allowed for \
                     loopback hosts (localhost, 127.0.0.1, [::1]); use https"
                )))
            }
        }
        other => Err(PaymentError::Config(format!(
            "unsupported base URL scheme {other:?} in {url:?}: expected https"
        ))),
    }
}

/// Strip trailing slashes so `format!("{base}{path}")` stays correct.
fn normalize_base(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

/// Whether `host` names the local machine.
fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

/// Map a non-2xx gateway status onto a [`PaymentError`].
///
/// Without a shared classifier each provider collapsed every failure into
/// `Provider(..)`, which made `RateLimited` unconstructable and silently turned
/// the processor's retry configuration into dead config. Routing all providers
/// through this keeps throttling and auth failures distinguishable — and
/// therefore correctly retryable or correctly fatal.
///
/// `status` accepts a `u16` or a `reqwest::StatusCode`. `retry_after` is the
/// parsed `Retry-After` header in seconds, if the gateway sent one. 404 maps to
/// `Provider`; callers with the context to know *what* was missing should refine
/// it to [`PaymentError::ChargeNotFound`], [`PaymentError::RefundNotFound`], etc.
///
/// Deliberately `pub(crate)`: this is an internal mapping table, not API. Making
/// it public would freeze the exact error strings and the status→variant mapping
/// under semver, and neither is something a third party should depend on.
pub(crate) fn classify_status(
    provider: &str,
    status: impl Into<u16>,
    body: &str,
    retry_after: Option<u32>,
) -> PaymentError {
    let status = status.into();
    let snippet = sanitize_body(body);
    match status {
        429 => PaymentError::RateLimited(retry_after.unwrap_or(1)),
        401 | 403 => PaymentError::Authentication(format!(
            "{provider} rejected credentials (HTTP {status}): {snippet}"
        )),
        // The status rides along on the variant, not just inside the message:
        // `PaymentError::is_retryable` reads it to tell a transient gateway 5xx
        // from a permanent 4xx.
        _ => PaymentError::provider(
            status,
            format!("{provider} returned HTTP {status}: {snippet}"),
        ),
    }
}

/// Reduce a gateway response body to something safe to put in an error message.
///
/// Error strings get logged, wrapped and reported upstream, so a raw body is a
/// leak risk: gateway errors routinely echo back the request, which can include
/// a PAN or the API key that was rejected. This truncates to a bounded snippet
/// and redacts long digit runs and credential-shaped tokens.
///
/// # Bounded by construction
///
/// The input is clipped to [`MAX_BODY_SCAN`] *before* anything is scanned, and
/// redaction happens in place on a single output buffer. Total work and total
/// allocation are therefore O([`MAX_BODY_SCAN`]) no matter how large the body
/// is — which matters because this runs on the error path inside the processor's
/// retry loop, on input an attacker or a compromised gateway controls.
///
/// # Tokenization
///
/// Splitting is on *non-token* characters (anything that is not ASCII
/// alphanumeric or `_`), not on whitespace. Real gateway errors are JSON with no
/// spaces at all, so a whitespace split treats `{"key":"sk_live_…"}` as one
/// token and the credential survives; splitting on punctuation reaches the
/// credential wherever it is embedded.
///
/// # Straddling the scan window
///
/// A token or digit run that crosses [`MAX_BODY_SCAN`] would be judged on its
/// truncated fragment and fall under the length thresholds below — and because
/// redaction *shrinks* the buffer before the [`MAX_BODY_SNIPPET`] output
/// truncation, that fragment can still reach the output. The partial trailing
/// token is therefore dropped before anything is scanned.
///
/// Deliberately `pub(crate)`: this is a redaction heuristic whose thresholds and
/// prefix list must stay free to tune. Exporting it would both freeze those
/// details under semver and advertise a best-effort scrubber as a security
/// control.
pub(crate) fn sanitize_body(body: &str) -> String {
    // Clip first: everything below is proportional to what we scan, and the
    // output can never exceed MAX_BODY_SNIPPET anyway.
    let mut input = clip_to_char_boundary(body, MAX_BODY_SCAN);
    let was_clipped = input.len() < body.len();
    if was_clipped {
        input = drop_partial_trailing_token(input);
    }

    let mut out = String::with_capacity(input.len() + REDACTION.len());
    // Byte offset in `out` where the current PAN candidate begins, and how many
    // digits it contains. The region may span separators — see `DigitRegion`.
    let mut digits = DigitRegion::default();
    let mut token_start: Option<usize> = None;
    let mut redact_next_token = false;

    for ch in input.chars() {
        digits.feed(&mut out, ch);

        if ch.is_ascii_alphanumeric() || ch == '_' {
            if token_start.is_none() {
                token_start = Some(out.len());
            }
        } else {
            flush_token(&mut out, &mut token_start, &mut redact_next_token, Some(ch));
        }
        out.push(ch);
    }
    digits.flush(&mut out);
    flush_token(&mut out, &mut token_start, &mut redact_next_token, None);

    let mut result = truncate_on_char_boundary(&out, MAX_BODY_SNIPPET);
    if was_clipped && !result.ends_with(TRUNCATION_MARKER) {
        result.push_str(TRUNCATION_MARKER);
    }
    result
}

/// Trim a token cut in half by the scan window off the end of `input`.
///
/// Only called when the body really was clipped, so nothing is lost from a body
/// that fit.
fn drop_partial_trailing_token(input: &str) -> &str {
    let bytes = input.as_bytes();
    let mut end = bytes.len();
    // Every byte tested is ASCII, so this can never land mid-character.
    while end > 0 && (bytes[end - 1].is_ascii_alphanumeric() || bytes[end - 1] == b'_') {
        end -= 1;
    }
    &input[..end]
}

/// A card-number-shaped digit run being accumulated at the tail of the output.
///
/// A PAN is 12–19 digits, and gateway errors echo the submitted number back
/// verbatim — but they echo it *as the user typed it*, so `4242 4242 4242 4242`
/// and `4242-4242-4242-4242` are just as common as the unseparated form. Reset
/// on every non-digit (the previous behavior) saw those as four runs of four and
/// passed the card straight through, so a single space or hyphen between digit
/// groups keeps the run alive and the region redacted as a whole.
#[derive(Default)]
struct DigitRegion {
    /// Byte offset in the output where the region starts.
    start: Option<usize>,
    /// Digits seen so far, ignoring separators.
    count: usize,
    /// A separator is being held open pending the next character; two in a row
    /// (or one at the end) closes the region.
    pending_separator: bool,
}

impl DigitRegion {
    /// Longest number of digits that is definitely not a PAN.
    const MIN_PAN_DIGITS: usize = 12;

    /// Feed the next character, flushing the region if it ends here.
    ///
    /// Must be called *before* `ch` is pushed onto `out`, because flushing
    /// truncates back to the start of the region.
    fn feed(&mut self, out: &mut String, ch: char) {
        if ch.is_ascii_digit() {
            if self.start.is_none() {
                self.start = Some(out.len());
            }
            self.count += 1;
            self.pending_separator = false;
        } else if matches!(ch, '-' | ' ') && self.count > 0 && !self.pending_separator {
            // Tentatively part of the region; if the next character is not a
            // digit the region closes and the separator stays in the output.
            self.pending_separator = true;
        } else {
            self.flush(out);
        }
    }

    /// Redact the accumulated region if it is long enough to be a card number.
    fn flush(&mut self, out: &mut String) {
        if let Some(start) = self.start {
            // `start` can exceed `out.len()` when a credential-shaped token
            // flush already truncated past it; that region is gone, so there is
            // nothing left to redact.
            if self.count >= Self::MIN_PAN_DIGITS && start <= out.len() {
                out.truncate(start);
                out.push_str(REDACTION);
            }
        }
        *self = Self::default();
    }
}

/// Replace the token ending at the tail of `out` if it looks like a credential.
///
/// `separator` is the character that terminated the token (`None` at end of
/// input); it decides whether a "redact the next token too" flag survives, which
/// is what makes a JWT work — see below.
///
/// Works on the output buffer directly rather than collecting tokens into a
/// `Vec<String>` and re-joining them, which cost a second full copy of the body
/// plus one allocation per token.
fn flush_token(
    out: &mut String,
    token_start: &mut Option<usize>,
    redact_next: &mut bool,
    separator: Option<char>,
) {
    let Some(start) = token_start.take() else {
        return;
    };
    // A digit-region flush may already have truncated past this token's start.
    if start >= out.len() {
        return;
    }
    let token = &out[start..];

    // A bare `Bearer` is only 6 characters, so the credential that matters is
    // the *next* token; `Authorization: Bearer <token>` is the usual shape.
    // `Basic` matters just as much: Braintree authenticates
    // `public_key:private_key` and Mailgun `api:<key>`, both base64'd into an
    // `Authorization: Basic` header that an error body echoing the request
    // repeats verbatim — and base64 matches none of the prefixes below.
    let was_armed = *redact_next;
    let follows_scheme = was_armed && token.len() >= 12;
    let arms_next = token.eq_ignore_ascii_case("Bearer") || token.eq_ignore_ascii_case("Basic");

    let credential_shaped = (token.len() >= 12
        && (token.starts_with("sk_")
            || token.starts_with("rk_")
            || token.starts_with("pk_")
            || token.starts_with("whsec_")
            || token.starts_with("Bearer")))
        // A JWT always begins with the base64url of `{"` + `alg`, i.e. `eyJ`.
        // PayPal's OAuth `access_token` and APNS/FCM provider tokens are all
        // JWTs, and they arrive with no `Bearer` in front of them at all.
        || (token.len() >= 16 && token.starts_with("eyJ"));

    let redacted = credential_shaped || follows_scheme;

    // Tokenization splits on every non-`[A-Za-z0-9_]` character, and that
    // includes `.` — the JWT segment separator. Disarming here (the previous
    // behavior) meant the header segment consumed the flag and the claims *and
    // signature* were emitted verbatim. Staying armed across a `.`/`-`
    // continuation of something we just redacted covers the whole token.
    *redact_next = arms_next || ((was_armed || redacted) && matches!(separator, Some('.' | '-')));

    if redacted {
        out.truncate(start);
        out.push_str(REDACTION);
    }
}

/// Borrow at most `max` bytes of `s`, without splitting a UTF-8 character.
fn clip_to_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Truncate to at most `max` bytes without splitting a UTF-8 character.
fn truncate_on_char_boundary(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    format!("{}{TRUNCATION_MARKER}", clip_to_char_boundary(s, max))
}

/// Read the `Retry-After` header as a whole number of seconds.
///
/// Gateways send this on a 429 to say how long they actually want the caller to
/// wait. Without it every throttle collapses to a hardcoded one-second backoff,
/// which ignores the delay the gateway asked for and hammers an endpoint that
/// has already said it is overloaded — so the value feeds straight into
/// [`classify_status`], which turns it into
/// [`PaymentError::RateLimited`]'s payload and hence
/// [`PaymentError::retry_after`].
///
/// A missing, non-ASCII, or unparseable header yields `None`, leaving the caller
/// on its own exponential schedule.
///
/// Shared by every provider: this was previously copy-pasted byte-identically
/// into the Stripe, PayPal and Braintree modules, all three feeding the same
/// classifier next door. `pub(crate)` rather than `pub` — it is an internal
/// helper, and a glob re-export would otherwise put it in the published API.
#[cfg(any(feature = "stripe", feature = "paypal", feature = "braintree"))]
pub(crate) fn retry_after_secs(response: &reqwest::Response) -> Option<u32> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Common HTTP client for providers
pub struct ProviderClient {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl std::fmt::Debug for ProviderClient {
    /// Hand-written so `api_key` never reaches a log or a panic message. A
    /// derived `Debug` would print `sk_live_…` verbatim from any
    /// `unwrap`/`assert` that happens to format this struct.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderClient")
            .field("base_url", &self.base_url)
            .field("api_key", &"[redacted]")
            .finish_non_exhaustive()
    }
}

impl ProviderClient {
    /// Create a new provider client.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::Config`] if the HTTP client cannot be built —
    /// in practice a TLS backend that failed to initialize.
    ///
    /// This is fallible on purpose. The obvious shortcut,
    /// `build_http_client().unwrap_or_else(|_| reqwest::Client::new())`, is
    /// worse than failing: `Client::new()` carries **no request timeout and no
    /// connect timeout**, so the fallback silently produces exactly the untimed
    /// client [`build_http_client`] exists to prevent — inside the processor's
    /// retry loop, where it can hang a charge indefinitely, and with no log line
    /// to say the degradation happened. A constructor that returns an error the
    /// caller must handle cannot degrade quietly.
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> PaymentResult<Self> {
        Self::try_new(build_http_client, base_url, api_key)
    }

    /// [`new`](Self::new) with the client factory injected, so the failure path
    /// is reachable from a test without a broken TLS backend.
    fn try_new<F>(
        build: F,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> PaymentResult<Self>
    where
        F: FnOnce() -> PaymentResult<reqwest::Client>,
    {
        Ok(Self {
            client: build()?,
            base_url: base_url.into(),
            api_key: api_key.into(),
        })
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
        self.post_form_idempotent(path, body, None).await
    }

    /// POST request with a form body, optionally carrying an idempotency key.
    ///
    /// The key is sent as the `Idempotency-Key` header, which Stripe (and
    /// providers that copy its convention) use to collapse duplicate retries of
    /// the same logical request into a single charge.
    pub async fn post_form_idempotent<T: serde::Serialize>(
        &self,
        path: &str,
        body: &T,
        idempotency_key: Option<&str>,
    ) -> PaymentResult<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.client.post(&url).bearer_auth(&self.api_key).form(body);
        if let Some(key) = idempotency_key {
            req = req.header("Idempotency-Key", key);
        }
        Ok(req.send().await?)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_base_urls_are_accepted() {
        assert!(validate_base_url("https://api.stripe.com/v1").is_ok());
        assert!(validate_base_url("https://api-m.sandbox.paypal.com").is_ok());
    }

    #[test]
    fn validated_base_url_joins_cleanly_with_a_leading_slash_path() {
        // Providers build URLs as format!("{base}{path}"); a trailing slash
        // here would emit "https://host//v1/charges".
        let base = validate_base_url("https://api.stripe.com/").unwrap();
        assert_eq!(base, "https://api.stripe.com");
        assert_eq!(
            format!("{base}/v1/charges"),
            "https://api.stripe.com/v1/charges"
        );

        let base = validate_base_url("https://api.stripe.com").unwrap();
        assert_eq!(base, "https://api.stripe.com");
    }

    #[test]
    fn plaintext_http_is_allowed_only_on_loopback() {
        for ok in [
            "http://localhost:8080",
            "http://127.0.0.1:3000/v1",
            "http://[::1]:9000",
        ] {
            assert!(validate_base_url(ok).is_ok(), "should allow {ok}");
        }
    }

    #[test]
    fn plaintext_http_to_a_remote_host_is_rejected() {
        // Bearer credentials go out on every request; in the clear to a remote
        // host that is a credential disclosure.
        let err = validate_base_url("http://api.stripe.com").unwrap_err();
        assert!(matches!(err, PaymentError::Config(_)));
        assert!(err.to_string().contains("https"));
    }

    #[test]
    fn non_http_schemes_are_rejected() {
        for bad in [
            "ftp://api.example.com",
            "file:///etc/passwd",
            "data:text/plain,hi",
            "not a url",
        ] {
            assert!(
                matches!(validate_base_url(bad), Err(PaymentError::Config(_))),
                "should reject {bad}"
            );
        }
    }

    #[test]
    fn throttling_status_becomes_rate_limited_with_server_delay() {
        let e = classify_status("stripe", 429u16, "slow down", Some(12));
        assert!(matches!(e, PaymentError::RateLimited(12)));
        assert_eq!(e.retry_after(), Some(Duration::from_secs(12)));
    }

    #[test]
    fn throttling_without_retry_after_still_rate_limits() {
        assert!(matches!(
            classify_status("paypal", 429u16, "", None),
            PaymentError::RateLimited(1)
        ));
    }

    #[test]
    fn auth_statuses_are_fatal_not_retryable() {
        for status in [401u16, 403u16] {
            let e = classify_status("braintree", status, "bad key", None);
            assert!(matches!(e, PaymentError::Authentication(_)));
            assert!(!e.is_retryable(), "HTTP {status} must not be retried");
        }
    }

    #[test]
    fn other_statuses_fall_back_to_provider_carrying_the_status() {
        for status in [404u16, 400u16, 500u16] {
            let e = classify_status("stripe", status, "nope", None);
            assert!(
                matches!(e, PaymentError::Provider { status: Some(s), .. } if s == status),
                "got {e:?}"
            );
        }
    }

    #[test]
    fn a_gateway_5xx_is_classified_retryable() {
        // A 502 from a load balancer may never have reached the processor. The
        // status has to ride on the variant for `is_retryable` to see it — with
        // it dropped, every gateway 5xx permanently failed the charge and the
        // idempotency machinery bought nothing.
        for status in [500u16, 502, 503, 504] {
            let e = classify_status("stripe", status, "bad gateway", None);
            assert!(e.is_retryable(), "HTTP {status} should be retryable: {e:?}");
        }
        assert!(!classify_status("stripe", 400u16, "bad", None).is_retryable());
        assert!(!classify_status("stripe", 404u16, "gone", None).is_retryable());
    }

    #[test]
    fn sanitize_redacts_card_shaped_digit_runs() {
        let out = sanitize_body(r#"{"number":"4242424242424242","exp":"12"}"#);
        assert!(!out.contains("4242424242424242"), "PAN leaked: {out}");
        assert!(out.contains("[redacted]"));
        // Short digit runs are ordinary data and must survive.
        assert!(out.contains("12"));
    }

    #[test]
    fn sanitize_redacts_credential_shaped_tokens() {
        let out = sanitize_body("rejected key sk_live_abcdefghijklmnop here");
        assert!(
            !out.contains("sk_live_abcdefghijklmnop"),
            "key leaked: {out}"
        );
        assert!(out.contains("[redacted]"));
    }

    #[test]
    fn sanitize_redacts_credentials_embedded_in_json() {
        // The shape that actually arrives from a gateway: minified JSON, with no
        // whitespace anywhere. Splitting on whitespace made the *entire* body a
        // single token, and `trim_matches` then stopped at the leading key name
        // — so the token examined was `key":"sk_live_…`, which does not *start
        // with* `sk_`, and nothing was redacted at all.
        //
        // Note a body with a space in front of the credential was scrubbed
        // correctly by accident, which is why the old test passed: it used the
        // one shape the old heuristic happened to handle.
        let body =
            r#"{"key":"sk_live_abcdefghij","hook":"whsec_abcdefghij","tok":"rk_live_abcdefghij"}"#;
        let out = sanitize_body(body);

        assert!(!out.contains("sk_live_abcdefghij"), "api key leaked: {out}");
        assert!(
            !out.contains("whsec_abcdefghij"),
            "webhook secret leaked: {out}"
        );
        assert!(
            !out.contains("rk_live_abcdefghij"),
            "restricted key leaked: {out}"
        );
        assert!(out.contains("[redacted]"), "got {out}");
        // Redaction is surgical: the surrounding structure survives, so the
        // snippet is still useful for diagnosis.
        assert!(out.contains("key"), "got {out}");
        assert!(out.contains("hook"), "got {out}");
    }

    #[test]
    fn sanitize_redacts_a_bearer_token_following_the_scheme_name() {
        let out = sanitize_body(r#"{"Authorization":"Bearer abcdefghijklmnopqrst"}"#);
        assert!(!out.contains("abcdefghijklmnopqrst"), "token leaked: {out}");
        assert!(out.contains("[redacted]"), "got {out}");
    }

    #[test]
    fn sanitize_redacts_a_pan_embedded_in_json_without_spaces() {
        let out = sanitize_body(r#"{"card":{"number":"4242424242424242"}}"#);
        assert!(!out.contains("4242424242424242"), "PAN leaked: {out}");
    }

    /// A realistic three-segment HS256 JWT.
    const JWT: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.\
                       eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIn0.\
                       SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJVadQssw5c";

    #[test]
    fn sanitize_redacts_every_segment_of_a_bearer_jwt() {
        // Tokenization splits on every non-[A-Za-z0-9_] character, and that
        // includes `.` — the JWT separator. The header segment consumed the
        // "redact next" flag and reset it (it is not literally `Bearer`), so the
        // claims *and the signature* were emitted verbatim. A JWT is the common
        // bearer shape: PayPal's OAuth access_token and APNS/FCM provider tokens
        // are all JWTs.
        let out = sanitize_body(&format!(
            r#"{{"error":"invalid_token","request":{{"Authorization":"Bearer {JWT}"}}}}"#
        ));
        for (n, segment) in JWT.split('.').enumerate() {
            assert!(!out.contains(segment), "JWT segment {n} leaked: {out}");
        }
        assert!(out.contains(REDACTION), "got {out}");
    }

    #[test]
    fn sanitize_redacts_a_bare_jwt_with_no_bearer_prefix() {
        // PayPal's token endpoint answers with the JWT as a plain JSON value.
        let out = sanitize_body(&format!(
            r#"{{"access_token":"{JWT}","token_type":"Bearer","expires_in":32400}}"#
        ));
        for (n, segment) in JWT.split('.').enumerate() {
            assert!(!out.contains(segment), "JWT segment {n} leaked: {out}");
        }
    }

    #[test]
    fn sanitize_redacts_an_http_basic_credential() {
        // Braintree authenticates `public_key:private_key` and Mailgun
        // `api:<key>`, both as `Authorization: Basic <base64>`. There was no
        // `Basic` follow-on rule and a base64 blob matches no prefix, so an
        // error body echoing the request headers emitted a trivially reversible
        // credential.
        let credential = "cHVibGljX2tleTpwcml2YXRlX2tleV92ZXJ5X3NlY3JldA";
        let out = sanitize_body(&format!(
            r#"{{"message":"auth failed","headers":{{"Authorization":"Basic {credential}"}}}}"#
        ));
        assert!(!out.contains(credential), "basic credential leaked: {out}");
        assert!(out.contains(REDACTION), "got {out}");
    }

    #[test]
    fn sanitize_redacts_a_secret_straddling_the_scan_window() {
        // Redaction runs on the input clipped to MAX_BODY_SCAN, so a run
        // crossing that byte was judged on its truncated fragment and fell under
        // the length thresholds. And because redaction *shrinks* the buffer
        // before the MAX_BODY_SNIPPET output truncation, the fragment still
        // reached the output — which this body arranges by making everything
        // before the boundary redactable.
        let filler = format!("{},", "4".repeat(50)); // 51 bytes -> "[redacted],"
        let repeats = MAX_BODY_SCAN / filler.len(); // 40 -> exactly 2040 bytes
        let mut body = filler.repeat(repeats);
        assert!(body.len() < MAX_BODY_SCAN && body.len() + 16 > MAX_BODY_SCAN);
        body.push_str("4242424242424242");

        let out = sanitize_body(&body);
        assert!(
            !out.contains("4242"),
            "a PAN fragment straddling the scan window leaked: {out}"
        );
        assert!(out.ends_with(TRUNCATION_MARKER), "got {out}");
    }

    #[test]
    fn sanitize_redacts_a_pan_written_with_separators() {
        // `digit_run` reset on every non-digit, so a card echoed back with the
        // spacing the user typed produced four runs of four and passed through.
        for pan in [
            "4242-4242-4242-4242",
            "4242 4242 4242 4242",
            "4111 1111 1111 1111",
            "3782 822463 10005",
        ] {
            let out = sanitize_body(&format!(
                r#"{{"error":{{"param":"card[number]","value":"{pan}"}},"exp_year":"2031"}}"#
            ));
            for group in pan.split(['-', ' ']) {
                assert!(!out.contains(group), "PAN group {group} leaked: {out}");
            }
            assert!(out.contains(REDACTION), "got {out}");
            // Short digit runs are ordinary data and must still survive.
            assert!(out.contains("2031"), "got {out}");
        }
    }

    #[test]
    fn sanitize_truncates_long_bodies_on_a_char_boundary() {
        let body = "é".repeat(4000);
        let out = sanitize_body(&body);
        assert!(out.len() <= MAX_BODY_SNIPPET + 32);
        assert!(out.ends_with("…[truncated]"));
    }

    #[test]
    fn sanitize_bounds_both_output_and_work_on_a_multi_megabyte_body() {
        // A hostile or MITM'd gateway can answer a charge with an arbitrarily
        // large error page, and this runs on the error path inside the retry
        // loop. The previous implementation copied every char, then built a
        // Vec<String> of every token, then joined it — ~4x the body in
        // transient allocation and millions of allocations — before truncating.
        let small = format!(r#"{{"error":"declined","attempt":"{}"}}"#, "y".repeat(2000));
        let huge = format!(
            r#"{{"error":"declined","filler":"{}"}}"#,
            "y".repeat(8 * 1024 * 1024)
        );

        let timed = |body: &str| {
            let started = std::time::Instant::now();
            let out = sanitize_body(body);
            (out, started.elapsed())
        };

        // Warm up so the first call's page faults do not skew the comparison.
        let _ = timed(&small);
        let (_, small_elapsed) = timed(&small);
        let (out, huge_elapsed) = timed(&huge);

        assert!(
            out.len() <= MAX_BODY_SNIPPET + TRUNCATION_MARKER.len(),
            "output not bounded: {} bytes",
            out.len()
        );
        assert!(out.ends_with(TRUNCATION_MARKER), "got {out}");

        // The real property: cost is a function of MAX_BODY_SCAN, not of the
        // body. A 4000x larger input must not cost meaningfully more, because
        // both are clipped to the same window before anything is scanned. The
        // old implementation was linear in the *body* — it pushed every char,
        // then allocated a String per whitespace token, then joined — so this
        // ratio blew out entirely.
        let budget = small_elapsed * 50 + std::time::Duration::from_millis(10);
        assert!(
            huge_elapsed < budget,
            "8 MiB took {huge_elapsed:?} against a 2 KiB baseline of \
             {small_elapsed:?}; work is scaling with the body, not with \
             MAX_BODY_SCAN"
        );
    }

    #[test]
    fn sanitize_leaves_short_clean_bodies_intact() {
        assert_eq!(sanitize_body("card_declined"), "card_declined");
        assert_eq!(
            sanitize_body(r#"{"code":"card_declined"}"#),
            r#"{"code":"card_declined"}"#
        );
    }

    #[test]
    fn http_client_builds_with_timeouts() {
        assert!(build_http_client().is_ok());
    }

    #[test]
    fn provider_client_construction_surfaces_a_builder_failure() {
        // The old code swallowed this and fell back to `reqwest::Client::new()`,
        // which has neither a request nor a connect timeout — the exact untimed
        // client inside the retry loop that build_http_client exists to rule
        // out, with no error and no log line.
        let err = ProviderClient::try_new(
            || Err(PaymentError::Config("tls backend unavailable".into())),
            "https://api.stripe.com/v1",
            "sk_test",
        )
        .unwrap_err();

        assert!(matches!(err, PaymentError::Config(_)), "got {err:?}");
        assert!(err.to_string().contains("tls backend unavailable"));
    }

    #[test]
    fn provider_client_builds_normally() {
        assert!(ProviderClient::new("https://api.stripe.com/v1", "sk_test").is_ok());
    }
}
