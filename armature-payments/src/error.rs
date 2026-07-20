//! Error types for payment processing

use thiserror::Error;

/// Payment error types
#[derive(Error, Debug)]
pub enum PaymentError {
    /// Card declined
    #[error("Card declined: {0}")]
    CardDeclined(String),

    /// Expired card
    #[error("Card expired")]
    CardExpired,

    /// Insufficient funds
    #[error("Insufficient funds")]
    InsufficientFunds,

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

    /// The provider's API genuinely does not expose this operation.
    ///
    /// Distinct from [`PaymentError::Provider`]: this is a permanent, structural
    /// gap in the gateway's API surface, not a runtime failure, so it is never
    /// retryable and never worth surfacing to an end user as a payment problem.
    #[error("Unsupported operation: {0}")]
    Unsupported(String),
}

impl From<reqwest::Error> for PaymentError {
    /// Classify a transport failure by *cause*, not uniformly as retryable.
    ///
    /// The distinction is financially load-bearing. A connect or timeout failure
    /// may well have never reached the gateway, so retrying is correct. But a
    /// body/decode failure happens *after* the gateway returned a response —
    /// typically a 2xx for a charge it has already committed. Retrying that
    /// re-posts a transaction the customer has already been charged for, so it
    /// maps to the non-retryable [`PaymentError::Serialization`].
    fn from(err: reqwest::Error) -> Self {
        if err.is_body() || err.is_decode() {
            PaymentError::Serialization(err.to_string())
        } else if err.is_builder() {
            PaymentError::Config(err.to_string())
        } else if let Some(status) = err.status() {
            // `is_status()` means the gateway answered and reqwest turned that
            // answer into an error (via `error_for_status`). The request
            // therefore *was* committed, and a 4xx is permanently fatal — the
            // catch-all below would have classified it retryable and replayed a
            // charge the gateway already rejected for a structural reason.
            // Route it through the shared classifier so it lands on the same
            // variant a hand-checked status would.
            //
            // No provider calls `error_for_status()` today, so this is
            // unreachable in practice; it exists so that adding such a call
            // cannot silently make declines retryable.
            crate::provider::classify_status("provider", status, &err.to_string(), None)
        } else {
            // timeout / connect / request / redirect: the request may not have
            // been committed, so these stay retryable.
            PaymentError::Network(err.to_string())
        }
    }
}

impl From<serde_json::Error> for PaymentError {
    fn from(err: serde_json::Error) -> Self {
        PaymentError::Serialization(err.to_string())
    }
}

impl PaymentError {
    /// Whether retrying the same operation could plausibly succeed.
    ///
    /// Only transport-level and throttling failures are retryable; a declined
    /// card, a validation failure, or an authentication error will fail
    /// identically on every attempt and must surface immediately.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            PaymentError::Network(_) | PaymentError::RateLimited(_)
        )
    }

    /// The delay the server explicitly asked us to wait before retrying.
    ///
    /// Only [`PaymentError::RateLimited`] carries one (from the `Retry-After`
    /// header). `None` means the caller should fall back to its own backoff
    /// schedule rather than retrying immediately.
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            PaymentError::RateLimited(secs) => {
                Some(std::time::Duration::from_secs(u64::from(*secs)))
            }
            _ => None,
        }
    }

    /// Build an [`PaymentError::Unsupported`] naming the provider and operation.
    pub fn unsupported(provider: &str, operation: &str) -> Self {
        PaymentError::Unsupported(format!("{provider} does not support {operation}"))
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
    #[allow(clippy::should_implement_trait)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limited_reports_server_requested_delay() {
        assert_eq!(
            PaymentError::RateLimited(7).retry_after(),
            Some(std::time::Duration::from_secs(7))
        );
    }

    #[test]
    fn non_throttling_errors_have_no_server_delay() {
        assert_eq!(PaymentError::Network("boom".into()).retry_after(), None);
        assert_eq!(PaymentError::CardExpired.retry_after(), None);
    }

    #[test]
    fn unsupported_is_not_retryable() {
        // A structural gap in the gateway API fails identically forever;
        // retrying it just multiplies latency on a doomed call.
        let e = PaymentError::unsupported("braintree", "resume_subscription");
        assert!(!e.is_retryable());
        assert!(e.to_string().contains("braintree"));
        assert!(e.to_string().contains("resume_subscription"));
    }

    #[test]
    fn serialization_failures_are_not_retryable() {
        // Regression guard for the double-charge path: a body/decode failure
        // arrives *after* the gateway committed the transaction, so it must
        // never be replayed.
        let err: PaymentError = serde_json::from_str::<i32>("not json").unwrap_err().into();
        assert!(matches!(err, PaymentError::Serialization(_)));
        assert!(!err.is_retryable());
    }

    #[test]
    fn a_status_carrying_transport_error_is_classified_by_status() {
        // The catch-all used to swallow `is_status()` errors as retryable
        // Network failures. A reqwest error that carries a status means the
        // gateway *answered* — and a 4xx answer is permanently fatal, so
        // replaying the charge just re-posts a request the gateway already
        // rejected for a structural reason.
        let bad_request = reqwest::Response::from(
            http::Response::builder()
                .status(400)
                .body("bad request")
                .unwrap(),
        );
        let err: PaymentError = bad_request.error_for_status().unwrap_err().into();
        assert!(matches!(err, PaymentError::Provider(_)), "got {err:?}");
        assert!(!err.is_retryable(), "a 400 must never be replayed");

        let unauthorized =
            reqwest::Response::from(http::Response::builder().status(401).body("nope").unwrap());
        let err: PaymentError = unauthorized.error_for_status().unwrap_err().into();
        assert!(
            matches!(err, PaymentError::Authentication(_)),
            "got {err:?}"
        );
        assert!(!err.is_retryable());

        let throttled = reqwest::Response::from(
            http::Response::builder()
                .status(429)
                .body("slow down")
                .unwrap(),
        );
        let err: PaymentError = throttled.error_for_status().unwrap_err().into();
        assert!(matches!(err, PaymentError::RateLimited(_)), "got {err:?}");
        assert!(err.is_retryable(), "throttling is the one retryable status");
    }

    #[test]
    fn network_and_rate_limit_stay_retryable() {
        assert!(PaymentError::Network("timeout".into()).is_retryable());
        assert!(PaymentError::RateLimited(1).is_retryable());
    }
}
