//! Error types for payment processing

use thiserror::Error;

/// Payment error types
#[derive(Error, Debug)]
pub enum PaymentError {
    /// Card declined
    #[error("Card declined: {0}")]
    CardDeclined(String),

    /// Invalid card
    #[error("Invalid card: {0}")]
    InvalidCard(String),

    /// Expired card
    #[error("Card expired")]
    CardExpired,

    /// Insufficient funds
    #[error("Insufficient funds")]
    InsufficientFunds,

    /// Invalid amount
    #[error("Invalid amount: {0}")]
    InvalidAmount(String),

    /// Customer not found
    #[error("Customer not found: {0}")]
    CustomerNotFound(String),

    /// Subscription not found
    #[error("Subscription not found: {0}")]
    SubscriptionNotFound(String),

    /// Payment method not found
    #[error("Payment method not found: {0}")]
    PaymentMethodNotFound(String),

    /// Charge not found
    #[error("Charge not found: {0}")]
    ChargeNotFound(String),

    /// Refund not found
    #[error("Refund not found: {0}")]
    RefundNotFound(String),

    /// Duplicate transaction
    #[error("Duplicate transaction: {0}")]
    DuplicateTransaction(String),

    /// Invalid webhook signature
    #[error("Invalid webhook signature")]
    InvalidWebhookSignature,

    /// Provider error
    #[error("Provider error: {0}")]
    Provider(String),

    /// Network error
    #[error("Network error: {0}")]
    Network(String),

    /// Rate limited
    #[error("Rate limited, retry after {0} seconds")]
    RateLimited(u32),

    /// Authentication error
    #[error("Authentication error: {0}")]
    Authentication(String),

    /// Validation error
    #[error("Validation error: {0}")]
    Validation(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Unknown error
    #[error("Unknown error: {0}")]
    Unknown(String),
}

impl From<reqwest::Error> for PaymentError {
    fn from(err: reqwest::Error) -> Self {
        PaymentError::Network(err.to_string())
    }
}

impl From<serde_json::Error> for PaymentError {
    fn from(err: serde_json::Error) -> Self {
        PaymentError::Serialization(err.to_string())
    }
}

/// Result type for payment operations
pub type PaymentResult<T> = Result<T, PaymentError>;

/// Decline code for card errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclineCode {
    /// Generic decline
    GenericDecline,
    /// Insufficient funds
    InsufficientFunds,
    /// Lost card
    LostCard,
    /// Stolen card
    StolenCard,
    /// Expired card
    ExpiredCard,
    /// Incorrect CVC
    IncorrectCvc,
    /// Processing error
    ProcessingError,
    /// Incorrect number
    IncorrectNumber,
    /// Fraudulent
    Fraudulent,
    /// Unknown
    Unknown,
}

impl DeclineCode {
    /// Parse from string
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "generic_decline" | "do_not_honor" => Self::GenericDecline,
            "insufficient_funds" => Self::InsufficientFunds,
            "lost_card" => Self::LostCard,
            "stolen_card" => Self::StolenCard,
            "expired_card" => Self::ExpiredCard,
            "incorrect_cvc" => Self::IncorrectCvc,
            "processing_error" => Self::ProcessingError,
            "incorrect_number" => Self::IncorrectNumber,
            "fraudulent" => Self::Fraudulent,
            _ => Self::Unknown,
        }
    }

    /// Get user-friendly message
    pub fn message(&self) -> &'static str {
        match self {
            Self::GenericDecline => "Your card was declined. Please try another card.",
            Self::InsufficientFunds => "Your card has insufficient funds.",
            Self::LostCard => "This card has been reported lost.",
            Self::StolenCard => "This card has been reported stolen.",
            Self::ExpiredCard => "Your card has expired.",
            Self::IncorrectCvc => "The CVC code is incorrect.",
            Self::ProcessingError => "There was an error processing your card. Please try again.",
            Self::IncorrectNumber => "The card number is incorrect.",
            Self::Fraudulent => "This transaction has been flagged as potentially fraudulent.",
            Self::Unknown => "Your card was declined. Please contact your bank.",
        }
    }
}
