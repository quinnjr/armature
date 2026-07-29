//! Payment provider implementations

#[cfg(feature = "stripe")]
pub mod stripe;

#[cfg(feature = "paypal")]
pub mod paypal;

#[cfg(feature = "braintree")]
pub mod braintree;

#[cfg(feature = "stripe")]
pub use stripe::StripeProvider;

#[cfg(feature = "paypal")]
pub use paypal::PayPalProvider;

#[cfg(feature = "braintree")]
pub use braintree::BraintreeProvider;

// Signature-header names, re-exported so callers can name the headers a
// provider signs instead of hardcoding the string literals at each call site.

#[cfg(feature = "stripe")]
pub use stripe::STRIPE_SIGNATURE_HEADER;

#[cfg(feature = "paypal")]
pub use paypal::{
    PAYPAL_AUTH_ALGO_HEADER, PAYPAL_CERT_URL_HEADER, PAYPAL_TRANSMISSION_ID_HEADER,
    PAYPAL_TRANSMISSION_SIG_HEADER, PAYPAL_TRANSMISSION_TIME_HEADER,
};

#[cfg(feature = "braintree")]
pub use braintree::BRAINTREE_SIGNATURE_HEADER;
