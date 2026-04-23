//! Webhook handling for payment events

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// Charge webhook data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChargeWebhookData {
    pub id: String,
    pub amount: i64,
    pub currency: String,
    pub status: String,
    pub customer: Option<String>,
    pub payment_method: Option<String>,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
    pub metadata: HashMap<String, String>,
}

/// Customer webhook data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerWebhookData {
    pub id: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub metadata: HashMap<String, String>,
}

/// Subscription webhook data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionWebhookData {
    pub id: String,
    pub customer: String,
    pub status: String,
    pub current_period_start: i64,
    pub current_period_end: i64,
    pub cancel_at_period_end: bool,
    pub trial_end: Option<i64>,
    pub metadata: HashMap<String, String>,
}

/// Invoice webhook data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceWebhookData {
    pub id: String,
    pub customer: String,
    pub subscription: Option<String>,
    pub status: String,
    pub amount_due: i64,
    pub amount_paid: i64,
    pub currency: String,
    pub hosted_invoice_url: Option<String>,
    pub pdf: Option<String>,
}

/// Payment method webhook data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentMethodWebhookData {
    pub id: String,
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
    handlers: HashMap<String, Box<dyn WebhookHandler>>,
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
        let key = format!("{:?}", event_type);
        self.handlers.insert(key, Box::new(handler));
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
        let key = format!("{:?}", event.event_type);

        if let Some(handler) = self.handlers.get(&key) {
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
    fn test_event_type_categories() {
        assert!(WebhookEventType::ChargeSucceeded.is_charge_event());
        assert!(!WebhookEventType::ChargeSucceeded.is_subscription_event());
        assert!(WebhookEventType::SubscriptionCreated.is_subscription_event());
        assert!(WebhookEventType::InvoicePaid.is_invoice_event());
    }
}
