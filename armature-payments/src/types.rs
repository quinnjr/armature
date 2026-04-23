//! Payment types and data structures

use crate::money::Money;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Charge/Payment request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChargeRequest {
    /// Amount to charge
    pub amount: Money,
    /// Payment source
    pub source: PaymentSource,
    /// Customer ID (optional)
    pub customer_id: Option<String>,
    /// Description
    pub description: Option<String>,
    /// Statement descriptor
    pub statement_descriptor: Option<String>,
    /// Metadata
    pub metadata: HashMap<String, String>,
    /// Capture immediately (false for auth-only)
    pub capture: bool,
    /// Idempotency key
    pub idempotency_key: Option<String>,
}

impl ChargeRequest {
    /// Create a simple charge request
    pub fn new(amount: Money, source: PaymentSource) -> Self {
        Self {
            amount,
            source,
            customer_id: None,
            description: None,
            statement_descriptor: None,
            metadata: HashMap::new(),
            capture: true,
            idempotency_key: None,
        }
    }

    /// With customer
    pub fn customer(mut self, customer_id: impl Into<String>) -> Self {
        self.customer_id = Some(customer_id.into());
        self
    }

    /// With description
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// With metadata
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Auth only (no capture)
    pub fn auth_only(mut self) -> Self {
        self.capture = false;
        self
    }
}

/// Payment source
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PaymentSource {
    /// Card token
    #[serde(rename = "card")]
    Card { token: String },
    /// Payment method ID
    #[serde(rename = "payment_method")]
    PaymentMethod { id: String },
    /// Customer's default payment method
    #[serde(rename = "customer")]
    Customer { customer_id: String },
    /// Bank account
    #[serde(rename = "bank")]
    Bank { token: String },
}

impl PaymentSource {
    /// Card token
    pub fn card(token: impl Into<String>) -> Self {
        Self::Card {
            token: token.into(),
        }
    }

    /// Payment method
    pub fn payment_method(id: impl Into<String>) -> Self {
        Self::PaymentMethod { id: id.into() }
    }

    /// Customer default
    pub fn customer(customer_id: impl Into<String>) -> Self {
        Self::Customer {
            customer_id: customer_id.into(),
        }
    }
}

/// Charge result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Charge {
    /// Charge ID
    pub id: String,
    /// Amount
    pub amount: Money,
    /// Amount refunded
    pub amount_refunded: Money,
    /// Status
    pub status: ChargeStatus,
    /// Customer ID
    pub customer_id: Option<String>,
    /// Payment method
    pub payment_method: Option<String>,
    /// Description
    pub description: Option<String>,
    /// Receipt URL
    pub receipt_url: Option<String>,
    /// Failure reason
    pub failure_reason: Option<String>,
    /// Metadata
    pub metadata: HashMap<String, String>,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
    /// Is captured
    pub captured: bool,
    /// Is refunded
    pub refunded: bool,
    /// Is disputed
    pub disputed: bool,
}

/// Charge status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeStatus {
    Pending,
    Succeeded,
    Failed,
    Canceled,
    Disputed,
}

/// Refund request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefundRequest {
    /// Charge ID to refund
    pub charge_id: String,
    /// Amount to refund (None = full refund)
    pub amount: Option<Money>,
    /// Reason
    pub reason: Option<RefundReason>,
    /// Metadata
    pub metadata: HashMap<String, String>,
}

impl RefundRequest {
    /// Create a refund request
    pub fn new(charge_id: impl Into<String>) -> Self {
        Self {
            charge_id: charge_id.into(),
            amount: None,
            reason: None,
            metadata: HashMap::new(),
        }
    }

    /// Partial refund
    pub fn amount(mut self, amount: Money) -> Self {
        self.amount = Some(amount);
        self
    }

    /// With reason
    pub fn reason(mut self, reason: RefundReason) -> Self {
        self.reason = Some(reason);
        self
    }
}

/// Refund reason
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundReason {
    Duplicate,
    Fraudulent,
    RequestedByCustomer,
}

/// Refund result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Refund {
    /// Refund ID
    pub id: String,
    /// Charge ID
    pub charge_id: String,
    /// Amount refunded
    pub amount: Money,
    /// Status
    pub status: RefundStatus,
    /// Reason
    pub reason: Option<RefundReason>,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
}

/// Refund status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundStatus {
    Pending,
    Succeeded,
    Failed,
    Canceled,
}

/// Create customer request
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateCustomerRequest {
    /// Email
    pub email: Option<String>,
    /// Name
    pub name: Option<String>,
    /// Phone
    pub phone: Option<String>,
    /// Description
    pub description: Option<String>,
    /// Default payment method
    pub payment_method: Option<String>,
    /// Metadata
    pub metadata: HashMap<String, String>,
    /// Address
    pub address: Option<Address>,
}

impl CreateCustomerRequest {
    /// Create with email
    pub fn with_email(email: impl Into<String>) -> Self {
        Self {
            email: Some(email.into()),
            ..Default::default()
        }
    }

    /// Set name
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set phone
    pub fn phone(mut self, phone: impl Into<String>) -> Self {
        self.phone = Some(phone.into());
        self
    }

    /// Set metadata
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

/// Update customer request
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateCustomerRequest {
    pub email: Option<String>,
    pub name: Option<String>,
    pub phone: Option<String>,
    pub description: Option<String>,
    pub default_payment_method: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
    pub address: Option<Address>,
}

/// Customer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Customer {
    /// Customer ID
    pub id: String,
    /// Email
    pub email: Option<String>,
    /// Name
    pub name: Option<String>,
    /// Phone
    pub phone: Option<String>,
    /// Description
    pub description: Option<String>,
    /// Default payment method
    pub default_payment_method: Option<String>,
    /// Address
    pub address: Option<Address>,
    /// Metadata
    pub metadata: HashMap<String, String>,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
}

/// Address
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Address {
    pub line1: Option<String>,
    pub line2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
}

/// Create payment method request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePaymentMethodRequest {
    /// Type of payment method
    pub method_type: PaymentMethodType,
    /// Card details (for card type)
    pub card: Option<CardDetails>,
    /// Billing details
    pub billing_details: Option<BillingDetails>,
}

/// Payment method type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentMethodType {
    Card,
    BankAccount,
    Paypal,
}

/// Card details for creating payment method
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardDetails {
    /// Card number
    pub number: String,
    /// Expiration month (1-12)
    pub exp_month: u32,
    /// Expiration year
    pub exp_year: u32,
    /// CVC
    pub cvc: String,
}

/// Billing details
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BillingDetails {
    pub name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub address: Option<Address>,
}

/// Payment method
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentMethod {
    /// Payment method ID
    pub id: String,
    /// Type
    pub method_type: PaymentMethodType,
    /// Customer ID (if attached)
    pub customer_id: Option<String>,
    /// Card info (if card type)
    pub card: Option<CardInfo>,
    /// Billing details
    pub billing_details: Option<BillingDetails>,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
}

/// Card info (masked)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardInfo {
    /// Brand (visa, mastercard, etc.)
    pub brand: String,
    /// Last 4 digits
    pub last4: String,
    /// Expiration month
    pub exp_month: u32,
    /// Expiration year
    pub exp_year: u32,
    /// Funding type
    pub funding: CardFunding,
}

/// Card funding type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardFunding {
    Credit,
    Debit,
    Prepaid,
    Unknown,
}

/// Create subscription request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSubscriptionRequest {
    /// Customer ID
    pub customer_id: String,
    /// Price/Plan ID
    pub price_id: String,
    /// Quantity
    pub quantity: Option<u32>,
    /// Trial period days
    pub trial_days: Option<u32>,
    /// Payment method ID
    pub payment_method: Option<String>,
    /// Metadata
    pub metadata: HashMap<String, String>,
    /// Coupon code
    pub coupon: Option<String>,
}

impl CreateSubscriptionRequest {
    /// Create a subscription request
    pub fn new(customer_id: impl Into<String>, price_id: impl Into<String>) -> Self {
        Self {
            customer_id: customer_id.into(),
            price_id: price_id.into(),
            quantity: None,
            trial_days: None,
            payment_method: None,
            metadata: HashMap::new(),
            coupon: None,
        }
    }

    /// Set quantity
    pub fn quantity(mut self, qty: u32) -> Self {
        self.quantity = Some(qty);
        self
    }

    /// Set trial days
    pub fn trial_days(mut self, days: u32) -> Self {
        self.trial_days = Some(days);
        self
    }
}

/// Subscription
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    /// Subscription ID
    pub id: String,
    /// Customer ID
    pub customer_id: String,
    /// Status
    pub status: SubscriptionStatus,
    /// Current period start
    pub current_period_start: DateTime<Utc>,
    /// Current period end
    pub current_period_end: DateTime<Utc>,
    /// Trial end (if in trial)
    pub trial_end: Option<DateTime<Utc>>,
    /// Cancel at period end
    pub cancel_at_period_end: bool,
    /// Canceled at
    pub canceled_at: Option<DateTime<Utc>>,
    /// Price ID
    pub price_id: String,
    /// Quantity
    pub quantity: u32,
    /// Metadata
    pub metadata: HashMap<String, String>,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
}

/// Subscription status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatus {
    Active,
    Trialing,
    PastDue,
    Canceled,
    Unpaid,
    Incomplete,
    IncompleteExpired,
    Paused,
}

impl SubscriptionStatus {
    /// Is active (can use service)
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active | Self::Trialing)
    }

    /// Needs attention
    pub fn needs_attention(&self) -> bool {
        matches!(self, Self::PastDue | Self::Incomplete | Self::Unpaid)
    }
}

/// Invoice
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    /// Invoice ID
    pub id: String,
    /// Customer ID
    pub customer_id: String,
    /// Subscription ID
    pub subscription_id: Option<String>,
    /// Status
    pub status: InvoiceStatus,
    /// Total amount
    pub total: Money,
    /// Amount paid
    pub amount_paid: Money,
    /// Amount due
    pub amount_due: Money,
    /// Invoice number
    pub number: Option<String>,
    /// PDF URL
    pub pdf_url: Option<String>,
    /// Hosted invoice URL
    pub hosted_url: Option<String>,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
    /// Due date
    pub due_date: Option<DateTime<Utc>>,
}

/// Invoice status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceStatus {
    Draft,
    Open,
    Paid,
    Void,
    Uncollectible,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_charge_request() {
        let req = ChargeRequest::new(Money::usd(2999), PaymentSource::card("tok_visa"))
            .description("Test charge")
            .metadata("order_id", "12345");

        assert_eq!(req.amount.amount, 2999);
        assert_eq!(req.description, Some("Test charge".to_string()));
        assert_eq!(req.metadata.get("order_id"), Some(&"12345".to_string()));
    }

    #[test]
    fn test_subscription_status() {
        assert!(SubscriptionStatus::Active.is_active());
        assert!(SubscriptionStatus::Trialing.is_active());
        assert!(!SubscriptionStatus::Canceled.is_active());
        assert!(SubscriptionStatus::PastDue.needs_attention());
    }
}
