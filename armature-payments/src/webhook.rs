//! Webhook handling for payment events

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The request headers that accompany an inbound webhook.
///
/// Providers authenticate a webhook from different pieces of the request, so
/// verification takes the whole header set rather than a single signature
/// string:
///
/// - **Stripe** reads `Stripe-Signature`.
/// - **PayPal** reads `PayPal-Auth-Algo`, `PayPal-Cert-Url`,
///   `PayPal-Transmission-Id`, `PayPal-Transmission-Sig` and
///   `PayPal-Transmission-Time`.
/// - **Braintree** reads `bt_signature` (Braintree posts it as a form field;
///   pass it here under that name).
///
/// Lookups are case-insensitive.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(from = "HashMap<String, String>", into = "HashMap<String, String>")]
pub struct WebhookHeaders {
    headers: HashMap<String, String>,
}

impl From<WebhookHeaders> for HashMap<String, String> {
    /// Serialize as a bare map, mirroring the `from` shape.
    ///
    /// Without this the derived `Serialize` emitted the struct wrapper
    /// (`{"headers":{..}}`) while `Deserialize` expected a bare map, so
    /// `from_str(&to_string(&h))` failed — breaking any path that persists or
    /// forwards headers, such as queuing a webhook for retry.
    fn from(value: WebhookHeaders) -> Self {
        value.headers
    }
}

impl From<HashMap<String, String>> for WebhookHeaders {
    /// Route deserialization through [`insert`](Self::insert) (via the
    /// `FromIterator` impl below) so a wire map with mixed-case keys — e.g.
    /// after a JSON round-trip through a webhook relay, queue, or replay tool
    /// — still normalizes to lowercase. A derived `Deserialize` would instead
    /// populate `headers` directly from the wire, silently breaking the
    /// documented case-insensitivity invariant and rejecting genuine
    /// webhooks whose producer sent e.g. `Stripe-Signature`.
    fn from(map: HashMap<String, String>) -> Self {
        map.into_iter().collect()
    }
}

impl WebhookHeaders {
    /// An empty header set.
    pub fn new() -> Self {
        Self::default()
    }

    /// A header set containing a single entry.
    pub fn single(name: impl AsRef<str>, value: impl Into<String>) -> Self {
        Self::new().with(name, value)
    }

    /// Add a header (builder style).
    pub fn with(mut self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.insert(name, value);
        self
    }

    /// Add a header.
    pub fn insert(&mut self, name: impl AsRef<str>, value: impl Into<String>) {
        self.headers
            .insert(name.as_ref().to_ascii_lowercase(), value.into());
    }

    /// Case-insensitive lookup.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Look up a header, or fail with [`crate::PaymentError::InvalidWebhookSignature`].
    ///
    /// A missing signature header is a verification failure, never a
    /// fall-through to acceptance.
    pub fn require(&self, name: &str) -> crate::PaymentResult<&str> {
        self.get(name)
            .ok_or(crate::PaymentError::InvalidWebhookSignature)
    }

    /// Number of headers present.
    pub fn len(&self) -> usize {
        self.headers.len()
    }

    /// Whether no headers are present.
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty()
    }
}

impl<K: AsRef<str>, V: Into<String>> FromIterator<(K, V)> for WebhookHeaders {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut headers = Self::new();
        for (k, v) in iter {
            headers.insert(k, v);
        }
        headers
    }
}

/// Webhook event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEvent {
    /// Event ID
    pub id: String,
    /// Event type
    pub event_type: WebhookEventType,
    /// Timestamp
    pub created_at: DateTime<Utc>,
    /// Event data
    pub data: WebhookData,
    /// Provider
    pub provider: String,
    /// Livemode
    pub livemode: bool,
}

/// Webhook event types
///
/// `Hash` is derived so [`WebhookRouter`] can key its handler map on the enum
/// itself; it previously keyed on `format!("{:?}", event_type)`, allocating a
/// `String` on every registration *and* every routed event.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEventType {
    // Charge events
    ChargeSucceeded,
    ChargeFailed,
    ChargeRefunded,
    ChargeDisputed,
    ChargeCaptured,

    // Payment intent events
    PaymentIntentSucceeded,
    PaymentIntentFailed,
    PaymentIntentCanceled,
    PaymentIntentProcessing,
    PaymentIntentRequiresAction,

    // Customer events
    CustomerCreated,
    CustomerUpdated,
    CustomerDeleted,

    // Subscription events
    SubscriptionCreated,
    SubscriptionUpdated,
    SubscriptionCanceled,
    SubscriptionTrialEnding,
    SubscriptionPastDue,

    // Invoice events
    InvoiceCreated,
    InvoicePaid,
    InvoicePaymentFailed,
    InvoiceUpcoming,
    InvoiceFinalized,

    // Payment method events
    PaymentMethodAttached,
    PaymentMethodDetached,
    PaymentMethodUpdated,

    // Payout events
    PayoutCreated,
    PayoutPaid,
    PayoutFailed,

    // Dispute events
    DisputeCreated,
    DisputeUpdated,
    DisputeClosed,

    // Unknown event
    Unknown(String),
}

impl WebhookEventType {
    /// Parse from string
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "charge.succeeded" => Self::ChargeSucceeded,
            "charge.failed" => Self::ChargeFailed,
            "charge.refunded" => Self::ChargeRefunded,
            "charge.disputed" => Self::ChargeDisputed,
            "charge.captured" => Self::ChargeCaptured,

            "payment_intent.succeeded" => Self::PaymentIntentSucceeded,
            "payment_intent.payment_failed" => Self::PaymentIntentFailed,
            "payment_intent.canceled" => Self::PaymentIntentCanceled,
            "payment_intent.processing" => Self::PaymentIntentProcessing,
            "payment_intent.requires_action" => Self::PaymentIntentRequiresAction,

            "customer.created" => Self::CustomerCreated,
            "customer.updated" => Self::CustomerUpdated,
            "customer.deleted" => Self::CustomerDeleted,

            "customer.subscription.created" => Self::SubscriptionCreated,
            "customer.subscription.updated" => Self::SubscriptionUpdated,
            "customer.subscription.deleted" => Self::SubscriptionCanceled,
            "customer.subscription.trial_will_end" => Self::SubscriptionTrialEnding,
            "customer.subscription.past_due" => Self::SubscriptionPastDue,

            "invoice.created" => Self::InvoiceCreated,
            "invoice.paid" => Self::InvoicePaid,
            "invoice.payment_failed" => Self::InvoicePaymentFailed,
            "invoice.upcoming" => Self::InvoiceUpcoming,
            "invoice.finalized" => Self::InvoiceFinalized,

            "payment_method.attached" => Self::PaymentMethodAttached,
            "payment_method.detached" => Self::PaymentMethodDetached,
            "payment_method.updated" => Self::PaymentMethodUpdated,

            "payout.created" => Self::PayoutCreated,
            "payout.paid" => Self::PayoutPaid,
            "payout.failed" => Self::PayoutFailed,

            "charge.dispute.created" => Self::DisputeCreated,
            "charge.dispute.updated" => Self::DisputeUpdated,
            "charge.dispute.closed" => Self::DisputeClosed,

            other => Self::Unknown(other.to_string()),
        }
    }

    /// Is a charge event
    pub fn is_charge_event(&self) -> bool {
        matches!(
            self,
            Self::ChargeSucceeded
                | Self::ChargeFailed
                | Self::ChargeRefunded
                | Self::ChargeDisputed
                | Self::ChargeCaptured
        )
    }

    /// Is a subscription event
    pub fn is_subscription_event(&self) -> bool {
        matches!(
            self,
            Self::SubscriptionCreated
                | Self::SubscriptionUpdated
                | Self::SubscriptionCanceled
                | Self::SubscriptionTrialEnding
                | Self::SubscriptionPastDue
        )
    }

    /// Is an invoice event
    pub fn is_invoice_event(&self) -> bool {
        matches!(
            self,
            Self::InvoiceCreated
                | Self::InvoicePaid
                | Self::InvoicePaymentFailed
                | Self::InvoiceUpcoming
                | Self::InvoiceFinalized
        )
    }
}

/// Webhook data payload
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WebhookData {
    /// Charge data
    Charge(ChargeWebhookData),
    /// Customer data
    Customer(CustomerWebhookData),
    /// Subscription data
    Subscription(SubscriptionWebhookData),
    /// Invoice data
    Invoice(InvoiceWebhookData),
    /// Payment method data
    PaymentMethod(PaymentMethodWebhookData),
    /// Generic/Unknown data
    Generic(serde_json::Value),
}

impl WebhookData {
    /// Interpret a raw webhook object according to its event type.
    ///
    /// Providers should call this instead of always emitting
    /// [`WebhookData::Generic`], so consumers can match on a typed payload
    /// rather than re-parsing `serde_json::Value` themselves.
    ///
    /// This is deliberately lossless and total: the typed shapes are modeled on
    /// Stripe's objects and other gateways differ, so any payload that does not
    /// deserialize cleanly falls back to `Generic(raw.clone())`. A handler
    /// therefore never loses data, and adding a provider whose payload does not
    /// match cannot turn into a parse failure at the webhook boundary.
    ///
    /// Note the enum is `#[serde(untagged)]`, so matching on the variant is the
    /// only reliable way to tell a typed payload from a generic one.
    ///
    /// `raw` is taken by value: the fallback hands the original tree straight
    /// back as `Generic` instead of deep-copying it, and the typed attempt
    /// deserializes out of a borrow. The previous version called
    /// `serde_json::from_value(raw.clone())` — a full recursive copy of the
    /// whole payload on the success path — and cloned a second time to build
    /// the fallback.
    pub fn from_event_type(event_type: &WebhookEventType, raw: serde_json::Value) -> WebhookData {
        use WebhookEventType as E;

        fn typed<T, F>(raw: serde_json::Value, wrap: F) -> WebhookData
        where
            T: serde::de::DeserializeOwned,
            F: FnOnce(T) -> WebhookData,
        {
            match T::deserialize(&raw) {
                Ok(v) => wrap(v),
                Err(_) => WebhookData::Generic(raw),
            }
        }

        match event_type {
            E::ChargeSucceeded
            | E::ChargeFailed
            | E::ChargeRefunded
            | E::ChargeDisputed
            | E::ChargeCaptured
            | E::PaymentIntentSucceeded
            | E::PaymentIntentFailed
            | E::PaymentIntentCanceled
            | E::PaymentIntentProcessing
            | E::PaymentIntentRequiresAction => {
                typed::<ChargeWebhookData, _>(raw, WebhookData::Charge)
            }

            E::CustomerCreated | E::CustomerUpdated | E::CustomerDeleted => {
                typed::<CustomerWebhookData, _>(raw, WebhookData::Customer)
            }

            E::SubscriptionCreated
            | E::SubscriptionUpdated
            | E::SubscriptionCanceled
            | E::SubscriptionTrialEnding
            | E::SubscriptionPastDue => {
                typed::<SubscriptionWebhookData, _>(raw, WebhookData::Subscription)
            }

            E::InvoiceCreated
            | E::InvoicePaid
            | E::InvoicePaymentFailed
            | E::InvoiceUpcoming
            | E::InvoiceFinalized => typed::<InvoiceWebhookData, _>(raw, WebhookData::Invoice),

            E::PaymentMethodAttached | E::PaymentMethodDetached | E::PaymentMethodUpdated => {
                typed::<PaymentMethodWebhookData, _>(raw, WebhookData::PaymentMethod)
            }

            // Payouts and disputes have no typed shape modeled yet, and an
            // unknown event type by definition has none.
            E::PayoutCreated
            | E::PayoutPaid
            | E::PayoutFailed
            | E::DisputeCreated
            | E::DisputeUpdated
            | E::DisputeClosed
            | E::Unknown(_) => WebhookData::Generic(raw),
        }
    }
}

/// Charge webhook data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChargeWebhookData {
    pub id: String,
    pub amount: i64,
    pub currency: String,
    pub status: String,
    #[serde(default)]
    pub customer: Option<String>,
    #[serde(default)]
    pub payment_method: Option<String>,
    #[serde(default)]
    pub failure_code: Option<String>,
    #[serde(default)]
    pub failure_message: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

/// Customer webhook data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerWebhookData {
    pub id: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

/// Subscription webhook data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionWebhookData {
    pub id: String,
    pub customer: String,
    pub status: String,
    #[serde(default)]
    pub current_period_start: i64,
    #[serde(default)]
    pub current_period_end: i64,
    #[serde(default)]
    pub cancel_at_period_end: bool,
    #[serde(default)]
    pub trial_end: Option<i64>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

/// Invoice webhook data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceWebhookData {
    pub id: String,
    pub customer: String,
    #[serde(default)]
    pub subscription: Option<String>,
    pub status: String,
    #[serde(default)]
    pub amount_due: i64,
    #[serde(default)]
    pub amount_paid: i64,
    pub currency: String,
    #[serde(default)]
    pub hosted_invoice_url: Option<String>,
    #[serde(default)]
    pub pdf: Option<String>,
}

/// Payment method webhook data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentMethodWebhookData {
    pub id: String,
    #[serde(default)]
    pub customer: Option<String>,
    #[serde(rename = "type")]
    pub method_type: String,
}

/// Webhook handler trait
#[async_trait::async_trait]
pub trait WebhookHandler: Send + Sync {
    /// Handle a webhook event
    async fn handle(
        &self,
        event: &WebhookEvent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// Webhook router for handling multiple event types
pub struct WebhookRouter {
    /// Keyed on the event type itself. Keying on `format!("{:?}", ..)` cost a
    /// `String` allocation per routed event, and made the routing table depend
    /// on `Debug` output — a formatting change would have silently unbound
    /// every handler.
    handlers: HashMap<WebhookEventType, Box<dyn WebhookHandler>>,
    default_handler: Option<Box<dyn WebhookHandler>>,
}

impl WebhookRouter {
    /// Create a new webhook router
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            default_handler: None,
        }
    }

    /// Register a handler for an event type
    pub fn on<H: WebhookHandler + 'static>(
        mut self,
        event_type: WebhookEventType,
        handler: H,
    ) -> Self {
        self.handlers.insert(event_type, Box::new(handler));
        self
    }

    /// Set default handler for unhandled events
    pub fn default<H: WebhookHandler + 'static>(mut self, handler: H) -> Self {
        self.default_handler = Some(Box::new(handler));
        self
    }

    /// Route an event to the appropriate handler
    pub async fn route(
        &self,
        event: &WebhookEvent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(handler) = self.handlers.get(&event.event_type) {
            handler.handle(event).await
        } else if let Some(ref handler) = self.default_handler {
            handler.handle(event).await
        } else {
            // No handler, ignore event
            Ok(())
        }
    }
}

impl Default for WebhookRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_type_parsing() {
        assert_eq!(
            WebhookEventType::from_str("charge.succeeded"),
            WebhookEventType::ChargeSucceeded
        );
        assert_eq!(
            WebhookEventType::from_str("customer.subscription.created"),
            WebhookEventType::SubscriptionCreated
        );
        assert!(matches!(
            WebhookEventType::from_str("unknown.event"),
            WebhookEventType::Unknown(_)
        ));
    }

    #[test]
    fn test_webhook_headers_deserialize_lowercases_keys() {
        // Regression test: WebhookHeaders used to derive Deserialize, which
        // populated the inner map straight from the wire and bypassed the
        // lowercasing `insert()` normally enforces. A mixed-case producer
        // (e.g. a relay re-encoding the original `Stripe-Signature` header)
        // would then make `get("stripe-signature")` return `None`, rejecting
        // every genuine webhook after a JSON round-trip.
        #[derive(Deserialize)]
        struct Wrapper {
            headers: WebhookHeaders,
        }

        let wrapper: Wrapper =
            serde_json::from_str(r#"{"headers":{"Stripe-Signature":"abc"}}"#).unwrap();
        assert_eq!(wrapper.headers.get("stripe-signature"), Some("abc"));
        assert_eq!(wrapper.headers.get("Stripe-Signature"), Some("abc"));
    }

    #[test]
    fn test_webhook_headers_serde_round_trip() {
        // Regression: `from = "HashMap<..>"` with a *derived* Serialize emitted
        // {"headers":{..}} while deserialization expected a bare map, so a
        // round-trip through JSON failed outright.
        let headers = WebhookHeaders::new()
            .with("Stripe-Signature", "t=1,v1=abc")
            .with("Content-Type", "application/json");

        let json = serde_json::to_string(&headers).unwrap();
        let back: WebhookHeaders = serde_json::from_str(&json).unwrap();

        assert_eq!(back.get("stripe-signature"), Some("t=1,v1=abc"));
        assert_eq!(back.get("Stripe-Signature"), Some("t=1,v1=abc"));
        assert_eq!(back.get("content-type"), Some("application/json"));
        assert_eq!(back.len(), headers.len());
    }

    #[test]
    fn test_webhook_headers_serialize_as_bare_map() {
        let headers = WebhookHeaders::single("A-Header", "v");
        let value: serde_json::Value = serde_json::to_value(&headers).unwrap();
        // A bare map, not a struct wrapper.
        assert!(value.is_object());
        assert_eq!(value.get("a-header").and_then(|v| v.as_str()), Some("v"));
        assert!(value.get("headers").is_none());
    }

    #[test]
    fn test_webhook_data_typed_charge() {
        let raw = serde_json::json!({
            "id": "ch_123",
            "amount": 2999,
            "currency": "usd",
            "status": "succeeded",
            "customer": "cus_1"
        });
        let data = WebhookData::from_event_type(&WebhookEventType::ChargeSucceeded, raw);
        match data {
            WebhookData::Charge(c) => {
                assert_eq!(c.id, "ch_123");
                assert_eq!(c.amount, 2999);
                assert_eq!(c.customer.as_deref(), Some("cus_1"));
                // Absent metadata defaults rather than failing the parse.
                assert!(c.metadata.is_empty());
            }
            other => panic!("expected Charge, got {other:?}"),
        }
    }

    #[test]
    fn test_webhook_data_typed_subscription_and_invoice() {
        let sub = serde_json::json!({
            "id": "sub_1", "customer": "cus_1", "status": "active"
        });
        assert!(matches!(
            WebhookData::from_event_type(&WebhookEventType::SubscriptionCreated, sub),
            WebhookData::Subscription(_)
        ));

        let inv = serde_json::json!({
            "id": "in_1", "customer": "cus_1", "status": "paid", "currency": "usd"
        });
        assert!(matches!(
            WebhookData::from_event_type(&WebhookEventType::InvoicePaid, inv),
            WebhookData::Invoice(_)
        ));
    }

    #[test]
    fn test_webhook_data_falls_back_to_generic_on_mismatch() {
        // A payload that does not fit the typed shape must not be lost or
        // error — it degrades to Generic with the original body intact.
        let raw = serde_json::json!({"unexpected": "shape"});
        let data = WebhookData::from_event_type(&WebhookEventType::ChargeSucceeded, raw.clone());
        match data {
            // The original tree is handed back intact, not a lossy rebuild.
            WebhookData::Generic(v) => assert_eq!(v, raw),
            other => panic!("expected Generic, got {other:?}"),
        }
    }

    #[test]
    fn test_webhook_data_unmodeled_event_types_are_generic() {
        let raw = serde_json::json!({"id": "po_1"});
        for et in [
            WebhookEventType::PayoutCreated,
            WebhookEventType::DisputeCreated,
            WebhookEventType::Unknown("something.else".into()),
        ] {
            assert!(matches!(
                WebhookData::from_event_type(&et, raw.clone()),
                WebhookData::Generic(_)
            ));
        }
    }

    #[tokio::test]
    async fn router_dispatches_on_the_event_type_enum() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Counting(Arc<AtomicUsize>);

        #[async_trait::async_trait]
        impl WebhookHandler for Counting {
            async fn handle(
                &self,
                _event: &WebhookEvent,
            ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        }

        let matched = Arc::new(AtomicUsize::new(0));
        let fallback = Arc::new(AtomicUsize::new(0));
        let router = WebhookRouter::new()
            .on(
                WebhookEventType::ChargeSucceeded,
                Counting(Arc::clone(&matched)),
            )
            // A payload-carrying variant must key on its payload too, which a
            // Debug-string key only did by accident.
            .on(
                WebhookEventType::Unknown("vendor.custom".into()),
                Counting(Arc::clone(&matched)),
            )
            .default(Counting(Arc::clone(&fallback)));

        let event = |event_type| WebhookEvent {
            id: "evt_1".into(),
            event_type,
            created_at: Utc::now(),
            data: WebhookData::Generic(serde_json::Value::Null),
            provider: "stripe".into(),
            livemode: false,
        };

        router
            .route(&event(WebhookEventType::ChargeSucceeded))
            .await
            .unwrap();
        router
            .route(&event(WebhookEventType::Unknown("vendor.custom".into())))
            .await
            .unwrap();
        router
            .route(&event(WebhookEventType::Unknown("vendor.other".into())))
            .await
            .unwrap();
        router
            .route(&event(WebhookEventType::PayoutPaid))
            .await
            .unwrap();

        assert_eq!(matched.load(Ordering::Relaxed), 2);
        assert_eq!(fallback.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_event_type_categories() {
        assert!(WebhookEventType::ChargeSucceeded.is_charge_event());
        assert!(!WebhookEventType::ChargeSucceeded.is_subscription_event());
        assert!(WebhookEventType::SubscriptionCreated.is_subscription_event());
        assert!(WebhookEventType::InvoicePaid.is_invoice_event());
    }
}
