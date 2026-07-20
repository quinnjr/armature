//! `with_base_url` must refuse to transmit credentials in cleartext.
//!
//! Regression test for `with_base_url` accepting any string. Every provider
//! request carries a live secret — Stripe's `sk_live_…` bearer token, PayPal's
//! client secret on the token handshake, Braintree's basic-auth private key — so
//! an `http://` base URL pointed at any non-loopback host leaks those
//! credentials to everyone on the path. The setter is fallible precisely so this
//! cannot be configured by accident.
//!
//! The loopback exemption is not incidental: every stub-server test in this
//! crate depends on `http://127.0.0.1:PORT` remaining acceptable.

use armature_payments::PaymentError;

/// Cleartext to a host that is not this machine.
const INSECURE: &[&str] = &[
    "http://evil.example",
    "http://evil.example/v1",
    "http://api.stripe.com",
    "http://10.0.0.5:8080",
];

/// Not a transport that can carry an authenticated HTTP request at all.
const WRONG_SCHEME: &[&str] = &["ftp://example.com", "file:///etc/passwd", "not a url"];

const SECURE: &[&str] = &[
    "https://api.example.com",
    "https://api.example.com/v1",
    "http://127.0.0.1:8080",
    "http://localhost:3000",
    "http://[::1]:9999",
];

/// Assert the whole matrix against one provider's setter.
fn assert_validates(name: &str, build: impl Fn(&str) -> Result<(), PaymentError>) {
    for &url in INSECURE {
        match build(url) {
            Err(PaymentError::Config(_)) => {}
            Err(other) => panic!("{name}: {url} must be rejected as Config, got {other:?}"),
            Ok(()) => panic!(
                "{name}: {url} was accepted — this base URL leaks the API \
                 credential to anyone on the network path"
            ),
        }
    }

    for &url in WRONG_SCHEME {
        assert!(
            matches!(build(url), Err(PaymentError::Config(_))),
            "{name}: {url} must be rejected as Config"
        );
    }

    for &url in SECURE {
        assert!(
            build(url).is_ok(),
            "{name}: {url} must be accepted; the stub-server tests rely on the \
             loopback exemption"
        );
    }
}

#[cfg(feature = "stripe")]
#[test]
fn stripe_rejects_a_cleartext_base_url() {
    use armature_payments::providers::stripe::StripeProvider;
    assert_validates("stripe", |url| {
        StripeProvider::new("sk_live_secret")
            .with_base_url(url)
            .map(|_| ())
    });
}

#[cfg(feature = "paypal")]
#[test]
fn paypal_rejects_a_cleartext_base_url() {
    use armature_payments::providers::paypal::PayPalProvider;
    assert_validates("paypal", |url| {
        PayPalProvider::new("client-id", "client-secret")
            .with_base_url(url)
            .map(|_| ())
    });
}

#[cfg(feature = "braintree")]
#[test]
fn braintree_rejects_a_cleartext_base_url() {
    use armature_payments::providers::braintree::BraintreeProvider;
    assert_validates("braintree", |url| {
        BraintreeProvider::new("merchant-1", "pub_key", "priv_key")
            .with_base_url(url)
            .map(|_| ())
    });
}
