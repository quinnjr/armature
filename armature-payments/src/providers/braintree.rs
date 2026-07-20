//! Braintree payment provider implementation

use crate::{
    error::{PaymentError, PaymentResult},
    money::{Currency, Money},
    provider::{PaymentProvider, retry_after_secs},
    types::*,
    webhook::{WebhookData, WebhookEvent, WebhookEventType, WebhookHeaders},
};
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::collections::HashMap;

/// The field Braintree signs each webhook with. Braintree posts it as a form
/// parameter alongside `bt_payload`; pass it under this name.
pub const BRAINTREE_SIGNATURE_HEADER: &str = "bt_signature";

/// Braintree provider
///
/// # Webhook framing
///
/// Braintree POSTs webhooks as two form fields: `bt_signature` and
/// `bt_payload`, where `bt_payload` is a **base64-encoded XML** notification
/// and the HMAC is computed over that raw base64 string, *not* over the decoded
/// XML.
///
/// [`verify_webhook`](Self::verify_webhook) therefore expects `payload` to be
/// `bt_payload` exactly as Braintree sent it, and
/// [`parse_webhook`](Self::parse_webhook) base64-decodes that same byte string
/// and parses the XML notification inside it. The two operate on the same bytes
/// — authenticating one representation while acting on another is precisely the
/// bug that made a signature meaningless.
pub struct BraintreeProvider {
    public_key: String,
    private_key: SecretString,
    /// Fully-qualified base URL including the `/merchants/{id}` segment,
    /// computed once instead of being reformatted on every request.
    base_url: String,
    /// Precomputed `Basic …` credential. Base64-encoding the key pair per
    /// request cost an allocation and an encode on every API call.
    auth_header: String,
    client: Client,
}

impl BraintreeProvider {
    /// Create a new Braintree provider.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::Config`] if the HTTP client cannot be built.
    /// Fallible on purpose: the obvious shortcut,
    /// `build_http_client().unwrap_or_else(|_| Client::new())`, is worse than
    /// failing outright. `Client::new()` carries **no request and no connect
    /// timeout**, so that fallback would silently produce exactly the untimed
    /// client the retry loop cannot tolerate — hanging a charge indefinitely
    /// inside the processor's retry loop, with no log line to say the timeouts
    /// were lost.
    pub fn new(
        merchant_id: impl Into<String>,
        public_key: impl Into<String>,
        private_key: impl Into<String>,
    ) -> PaymentResult<Self> {
        let merchant_id = merchant_id.into();
        let public_key = public_key.into();
        let private_key = SecretString::new(private_key.into().into());
        let auth_header = format!(
            "Basic {}",
            STANDARD.encode(format!("{}:{}", public_key, private_key.expose_secret()))
        );
        Ok(Self {
            base_url: merchant_base_url("https://api.sandbox.braintreegateway.com", &merchant_id),
            public_key,
            private_key,
            auth_header,
            // Carries the shared connect/request timeouts; an untimed client
            // inside the processor's retry loop can hang a charge indefinitely.
            client: crate::provider::build_http_client()?,
        })
    }

    /// Use production environment
    pub fn production(mut self) -> Self {
        self.base_url = merchant_base_url(
            "https://api.braintreegateway.com",
            &merchant_id_of(&self.base_url),
        );
        self
    }

    /// Point the client at an alternate API base URL (a mock or proxy).
    ///
    /// The merchant path segment is appended, matching the live gateway layout.
    ///
    /// The URL must be `https`, or `http` addressed to a loopback host
    /// (`localhost`, `127.0.0.1`, `[::1]`) for local testing. Cleartext `http`
    /// to any other host is rejected: every request carries the Braintree key
    /// pair as HTTP Basic credentials, so an unencrypted base URL hands them to
    /// anyone on the path.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::Config`] if `base_url` is unparseable or uses a
    /// scheme/host combination that would transmit credentials in cleartext.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> PaymentResult<Self> {
        let origin = crate::provider::validate_base_url(&base_url.into())?;
        self.base_url = merchant_base_url(&origin, &merchant_id_of(&self.base_url));
        Ok(self)
    }

    /// Make an authenticated API request
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, format!("{}{}", self.base_url, path))
            .header("Authorization", &self.auth_header)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .header("Braintree-Version", "2024-01-01")
    }
}

/// Join a gateway origin and merchant id into Braintree's API base URL.
fn merchant_base_url(origin: &str, merchant_id: &str) -> String {
    format!("{}/merchants/{}", origin.trim_end_matches('/'), merchant_id)
}

/// Recover the merchant id from a base URL built by [`merchant_base_url`].
///
/// The merchant id is only ever supplied through `new`, so the builder methods
/// that rebuild the base URL read it back rather than storing a second copy.
fn merchant_id_of(base_url: &str) -> String {
    base_url
        .rsplit_once("/merchants/")
        .map(|(_, id)| id.to_string())
        .unwrap_or_default()
}

/// Turn a non-2xx Braintree response into a typed [`PaymentError`].
///
/// Every failure previously became `Provider(..)` — and on the list/get paths,
/// *any* failure became `CustomerNotFound`, so expired credentials read as
/// "customer not found" and a 429 never triggered backoff.
async fn braintree_error(response: reqwest::Response) -> PaymentError {
    let status = response.status();
    let retry_after = retry_after_secs(response.headers()).and_then(|s| u32::try_from(s).ok());
    // A body that could not be read is not the same thing as an empty
    // body: `unwrap_or_default()` made a truncated response
    // indistinguishable from a gateway that legitimately sent nothing,
    // which is exactly the distinction someone debugging a failed charge
    // needs. Matches `stripe_unit`.
    let body = response
        .text()
        .await
        .unwrap_or_else(|e| format!("<body unreadable: {e}>"));
    crate::provider::classify_status("braintree", status, &body, retry_after)
}

/// As [`braintree_error`], but reporting a 404 as the resource-specific
/// not-found error the generic classifier cannot infer.
async fn braintree_error_not_found(
    response: reqwest::Response,
    not_found: PaymentError,
) -> PaymentError {
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return not_found;
    }
    braintree_error(response).await
}

/// Map a Braintree transaction `status` onto the crate's [`ChargeStatus`].
fn transaction_charge_status(status: &str) -> ChargeStatus {
    match status {
        "authorized" | "submitted_for_settlement" | "settling" | "settled" => {
            ChargeStatus::Succeeded
        }
        "voided" | "processor_declined" | "gateway_rejected" => ChargeStatus::Failed,
        _ => ChargeStatus::Pending,
    }
}

/// Map a Braintree refund transaction `status` onto the crate's
/// [`RefundStatus`].
///
/// A refund is a transaction in its own right, so it carries the same status
/// vocabulary. Hardcoding `Succeeded` reported a `processor_declined` refund —
/// money that never went back to the customer — as a completed refund.
fn transaction_refund_status(status: &str) -> RefundStatus {
    match status {
        "settled" | "settling" => RefundStatus::Succeeded,
        "submitted_for_settlement" => RefundStatus::Pending,
        "processor_declined" | "gateway_rejected" | "failed" => RefundStatus::Failed,
        "voided" => RefundStatus::Canceled,
        _ => RefundStatus::Pending,
    }
}

/// Currency reported by a Braintree transaction, defaulting to USD only when
/// the gateway genuinely omitted the field.
fn transaction_currency(iso_code: Option<&str>) -> Currency {
    iso_code
        .and_then(Currency::from_code)
        .unwrap_or(Currency::USD)
}

/// The amount Braintree reported on a transaction.
///
/// # Why this propagates instead of defaulting
///
/// This used to be `Money::from_float(txn.amount.parse().unwrap_or(0.0), ..)`.
/// Any amount string the `f64` parse rejected produced a [`Charge`] reporting
/// **$0.00 for money that actually moved** — and it did so on the *success*
/// path, so no ledger entry, receipt, or reconciliation run had any signal that
/// the figure was invented. A charge that fails loudly can be retried or
/// investigated; a charge that succeeds with the wrong amount silently corrupts
/// the books.
///
/// Parsing also goes through [`Decimal`](rust_decimal::Decimal) rather than
/// `f64`, so `"0.29"` is exactly 29 cents instead of 28.999999999999996 rounded
/// back into place.
fn transaction_amount(txn: &BraintreeTransaction) -> PaymentResult<Money> {
    let currency = transaction_currency(txn.currency_iso_code.as_deref());
    Money::from_gateway_string(&txn.amount, currency).ok_or_else(|| {
        PaymentError::Serialization(format!(
            "Braintree returned an unparseable transaction amount {:?} for transaction {:?} \
             (currency {}). The transaction may well have been processed — reconcile it against \
             the gateway rather than assuming it did not happen.",
            txn.amount,
            txn.id,
            currency.code()
        ))
    })
}

#[async_trait]
impl PaymentProvider for BraintreeProvider {
    fn name(&self) -> &'static str {
        "braintree"
    }

    async fn charge(&self, request: ChargeRequest) -> PaymentResult<Charge> {
        // Every PaymentSource variant must be routed or rejected. `_ => None`
        // silently dropped PaymentMethod, Customer and Bank, so the transaction
        // posted with neither nonce nor token and either failed opaquely or
        // charged a different instrument than the caller named.
        let (payment_method_nonce, payment_method_token) = match &request.source {
            PaymentSource::Card { token } => (Some(token.clone()), None),
            PaymentSource::PaymentMethod { id } => (None, Some(id.clone())),
            PaymentSource::Customer { .. } => {
                return Err(PaymentError::Validation(
                    "Braintree cannot charge a customer's default payment method implicitly; look \
                     up the customer's vaulted payment methods with list_payment_methods and pass \
                     one as PaymentSource::PaymentMethod."
                        .into(),
                ));
            }
            PaymentSource::Bank { .. } => {
                return Err(PaymentError::Validation(
                    "Braintree does not accept bank-account tokens through this API; ACH must be \
                     tokenized client-side into a payment method nonce and passed as \
                     PaymentSource::Card, or vaulted and passed as PaymentSource::PaymentMethod."
                        .into(),
                ));
            }
        };

        let transaction = BraintreeTransactionRequest {
            // Zero-decimal currencies must not carry two decimal places.
            amount: request.amount.to_gateway_string(),
            payment_method_nonce,
            payment_method_token,
            customer_id: request.customer_id.clone(),
            // Actually send the metadata rather than echoing back something
            // Braintree never received.
            custom_fields: if request.metadata.is_empty() {
                None
            } else {
                Some(request.metadata.clone())
            },
            options: BraintreeTransactionOptions {
                submit_for_settlement: request.capture,
            },
        };

        let wrapper = BraintreeWrapper { transaction };

        let response = self
            .request(reqwest::Method::POST, "/transactions")
            .json(&wrapper)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(braintree_error(response).await);
        }

        let result: BraintreeTransactionResponse = response.json().await?;
        let txn = result.transaction;
        let amount = transaction_amount(&txn)?;
        let currency = amount.currency;

        Ok(Charge {
            id: txn.id,
            amount,
            amount_refunded: Money::new(0, currency),
            status: transaction_charge_status(&txn.status),
            customer_id: txn.customer_id,
            payment_method: txn.payment_method_token,
            description: None,
            receipt_url: None,
            failure_reason: txn.processor_response_text,
            metadata: request.metadata,
            // Braintree reports the real creation time; inventing `Utc::now()`
            // put a wrong timestamp on every charge.
            created_at: txn.created_at.unwrap_or_else(Utc::now),
            captured: txn.status == "settled" || txn.status == "settling",
            refunded: false,
            disputed: false,
        })
    }

    async fn capture(&self, charge_id: &str, amount: Option<Money>) -> PaymentResult<Charge> {
        // Settling for less than the authorization requires sending the amount;
        // without it Braintree settles the full authorized amount.
        let body = BraintreeWrapper {
            transaction: BraintreeSettlementRequest {
                amount: amount.map(|a| a.to_gateway_string()),
            },
        };

        let response = self
            .request(
                reqwest::Method::PUT,
                &format!("/transactions/{}/submit_for_settlement", charge_id),
            )
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(braintree_error_not_found(
                response,
                PaymentError::ChargeNotFound(charge_id.to_string()),
            )
            .await);
        }

        let result: BraintreeTransactionResponse = response.json().await?;
        let txn = result.transaction;
        // Read the settled amount, currency and status rather than assuming USD
        // and success: a €120 settlement previously came back as Money { 12000,
        // USD }, which satisfies `Money::add`'s currency assertion and silently
        // corrupts a ledger.
        let amount = transaction_amount(&txn)?;
        let currency = amount.currency;
        let status = transaction_charge_status(&txn.status);

        Ok(Charge {
            id: txn.id,
            amount,
            amount_refunded: Money::new(0, currency),
            status,
            customer_id: txn.customer_id,
            payment_method: txn.payment_method_token,
            description: None,
            receipt_url: None,
            failure_reason: txn.processor_response_text,
            metadata: HashMap::new(),
            created_at: txn.created_at.unwrap_or_else(Utc::now),
            captured: txn.status == "settled" || txn.status == "settling",
            refunded: false,
            disputed: false,
        })
    }

    async fn refund(&self, request: RefundRequest) -> PaymentResult<Refund> {
        let refund_req = BraintreeRefundRequest {
            amount: request.amount.map(|a| a.to_gateway_string()),
        };

        let response = self
            .request(
                reqwest::Method::POST,
                &format!("/transactions/{}/refund", request.charge_id),
            )
            .json(&refund_req)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(braintree_error_not_found(
                response,
                PaymentError::ChargeNotFound(request.charge_id.clone()),
            )
            .await);
        }

        let result: BraintreeTransactionResponse = response.json().await?;
        let txn = result.transaction;
        // A refund reporting $0.00 for money that went back to the customer is
        // the same defect as a charge reporting $0.00 — surface it.
        let amount = transaction_amount(&txn)?;

        Ok(Refund {
            id: txn.id.clone(),
            charge_id: request.charge_id,
            amount,
            status: transaction_refund_status(&txn.status),
            reason: request.reason,
            created_at: txn.created_at.unwrap_or_else(Utc::now),
        })
    }

    async fn create_customer(&self, request: CreateCustomerRequest) -> PaymentResult<Customer> {
        let customer_req = BraintreeCustomerRequest {
            email: request.email.clone(),
            first_name: request
                .name
                .clone()
                .map(|n| n.split_whitespace().next().unwrap_or("").to_string()),
            last_name: request
                .name
                .map(|n| n.split_whitespace().skip(1).collect::<Vec<_>>().join(" ")),
            phone: request.phone,
        };

        let wrapper = BraintreeCustomerWrapper {
            customer: customer_req,
        };

        let response = self
            .request(reqwest::Method::POST, "/customers")
            .json(&wrapper)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(braintree_error(response).await);
        }

        let result: BraintreeCustomerResponse = response.json().await?;
        Ok(result.customer.into_customer(request.metadata))
    }

    async fn get_customer(&self, id: &str) -> PaymentResult<Customer> {
        let response = self
            .request(reqwest::Method::GET, &format!("/customers/{}", id))
            .send()
            .await?;

        if !response.status().is_success() {
            // Only a 404 means "no such customer". Mapping every failure here
            // made a 401 from expired credentials, and a 429 throttle, both
            // read as CustomerNotFound.
            return Err(braintree_error_not_found(
                response,
                PaymentError::CustomerNotFound(id.to_string()),
            )
            .await);
        }

        let result: BraintreeCustomerResponse = response.json().await?;
        Ok(result.customer.into_customer(HashMap::new()))
    }

    async fn update_customer(
        &self,
        id: &str,
        request: UpdateCustomerRequest,
    ) -> PaymentResult<Customer> {
        let customer_req = BraintreeCustomerRequest {
            email: request.email.clone(),
            first_name: request
                .name
                .clone()
                .map(|n| n.split_whitespace().next().unwrap_or("").to_string()),
            last_name: request
                .name
                .map(|n| n.split_whitespace().skip(1).collect::<Vec<_>>().join(" ")),
            phone: request.phone,
        };

        let wrapper = BraintreeCustomerWrapper {
            customer: customer_req,
        };

        let response = self
            .request(reqwest::Method::PUT, &format!("/customers/{}", id))
            .json(&wrapper)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(braintree_error_not_found(
                response,
                PaymentError::CustomerNotFound(id.to_string()),
            )
            .await);
        }

        // Braintree's PUT returns the full updated customer, so the follow-up
        // GET this used to make was a redundant round-trip that also opened a
        // window for a concurrent write to be read back instead of ours.
        let result: BraintreeCustomerResponse = response.json().await?;
        Ok(result
            .customer
            .into_customer(request.metadata.unwrap_or_default()))
    }

    async fn delete_customer(&self, id: &str) -> PaymentResult<()> {
        let response = self
            .request(reqwest::Method::DELETE, &format!("/customers/{}", id))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(braintree_error_not_found(
                response,
                PaymentError::CustomerNotFound(id.to_string()),
            )
            .await);
        }

        Ok(())
    }

    async fn create_payment_method(
        &self,
        request: CreatePaymentMethodRequest,
    ) -> PaymentResult<PaymentMethod> {
        // Braintree never accepts raw card data over the server API: the card
        // must be tokenized client-side into a single-use nonce. The previous
        // code discarded `request.card` and posted the literal sandbox nonce
        // `"fake-valid-nonce"`, which either vaults a test card or fails in
        // production — never the caller's actual card.
        let Some(nonce) = request.payment_method_nonce.clone() else {
            return Err(PaymentError::Validation(
                "Braintree requires a client-generated payment method nonce; collect one with the \
                 Braintree client SDK and set CreatePaymentMethodRequest::payment_method_nonce. \
                 Raw card details cannot be sent to Braintree's server API."
                    .into(),
            ));
        };

        let pm_req = BraintreePaymentMethodRequest {
            customer_id: None,
            payment_method_nonce: Some(nonce),
        };

        let wrapper = BraintreePaymentMethodWrapper {
            payment_method: pm_req,
        };

        let response = self
            .request(reqwest::Method::POST, "/payment_methods")
            .json(&wrapper)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(braintree_error(response).await);
        }

        let result: BraintreePaymentMethodResponse = response.json().await?;
        let pm = result.payment_method;

        Ok(PaymentMethod {
            id: pm.token,
            method_type: PaymentMethodType::Card,
            customer_id: pm.customer_id,
            card: pm.card_type.map(|brand| CardInfo {
                brand,
                last4: pm.last_4.unwrap_or_default(),
                exp_month: pm.expiration_month.unwrap_or(0),
                exp_year: pm.expiration_year.unwrap_or(0),
                funding: CardFunding::Unknown,
            }),
            billing_details: None,
            created_at: pm.created_at.unwrap_or_else(Utc::now),
        })
    }

    async fn attach_payment_method(
        &self,
        method_id: &str,
        customer_id: &str,
    ) -> PaymentResult<PaymentMethod> {
        let pm_req = BraintreePaymentMethodRequest {
            customer_id: Some(customer_id.to_string()),
            payment_method_nonce: None,
        };

        let wrapper = BraintreePaymentMethodWrapper {
            payment_method: pm_req,
        };

        let response = self
            .request(
                reqwest::Method::PUT,
                &format!("/payment_methods/any/{}", method_id),
            )
            .json(&wrapper)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(braintree_error_not_found(
                response,
                PaymentError::PaymentMethodNotFound(method_id.to_string()),
            )
            .await);
        }

        let result: BraintreePaymentMethodResponse = response.json().await?;
        let pm = result.payment_method;

        Ok(PaymentMethod {
            id: pm.token,
            method_type: PaymentMethodType::Card,
            customer_id: pm.customer_id,
            card: None,
            billing_details: None,
            created_at: pm.created_at.unwrap_or_else(Utc::now),
        })
    }

    async fn detach_payment_method(&self, method_id: &str) -> PaymentResult<PaymentMethod> {
        let response = self
            .request(
                reqwest::Method::DELETE,
                &format!("/payment_methods/any/{}", method_id),
            )
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(braintree_error_not_found(
                response,
                PaymentError::PaymentMethodNotFound(method_id.to_string()),
            )
            .await);
        }

        Ok(PaymentMethod {
            id: method_id.to_string(),
            method_type: PaymentMethodType::Card,
            customer_id: None,
            card: None,
            billing_details: None,
            // Braintree's delete returns an empty body, so there is no
            // creation time to report. `PaymentMethod::created_at` is not an
            // `Option`, so "now" is the only representable value; treat this
            // field as meaningless on a detached method.
            created_at: Utc::now(),
        })
    }

    async fn list_payment_methods(&self, customer_id: &str) -> PaymentResult<Vec<PaymentMethod>> {
        // Braintree embeds the vaulted payment methods in the customer
        // resource; the previous code fetched it and then threw them away.
        let response = self
            .request(reqwest::Method::GET, &format!("/customers/{}", customer_id))
            .send()
            .await?;

        if !response.status().is_success() {
            // A 401 from expired credentials and a 429 throttle are not
            // "customer not found"; only a 404 is.
            return Err(braintree_error_not_found(
                response,
                PaymentError::CustomerNotFound(customer_id.to_string()),
            )
            .await);
        }

        let result: BraintreeCustomerResponse = response.json().await?;
        let cust = result.customer;

        let mut methods: Vec<PaymentMethod> = cust
            .credit_cards
            .into_iter()
            .map(|card| PaymentMethod {
                id: card.token,
                method_type: PaymentMethodType::Card,
                customer_id: Some(customer_id.to_string()),
                card: Some(CardInfo {
                    brand: card.card_type.unwrap_or_default(),
                    last4: card.last_4.unwrap_or_default(),
                    exp_month: card
                        .expiration_month
                        .as_deref()
                        .and_then(|m| m.parse().ok())
                        .unwrap_or(0),
                    exp_year: card
                        .expiration_year
                        .as_deref()
                        .and_then(|y| y.parse().ok())
                        .unwrap_or(0),
                    funding: CardFunding::Unknown,
                }),
                billing_details: None,
                created_at: card.created_at.unwrap_or_else(Utc::now),
            })
            .collect();

        methods.extend(cust.paypal_accounts.into_iter().map(|acct| PaymentMethod {
            id: acct.token,
            method_type: PaymentMethodType::Paypal,
            customer_id: Some(customer_id.to_string()),
            card: None,
            billing_details: acct.email.map(|email| BillingDetails {
                email: Some(email),
                ..Default::default()
            }),
            created_at: acct.created_at.unwrap_or_else(Utc::now),
        }));

        Ok(methods)
    }

    async fn create_subscription(
        &self,
        request: CreateSubscriptionRequest,
    ) -> PaymentResult<Subscription> {
        let sub_req = BraintreeSubscriptionRequest {
            plan_id: request.price_id.clone(),
            payment_method_token: request.payment_method.clone(),
        };

        let wrapper = BraintreeSubscriptionWrapper {
            subscription: sub_req,
        };

        let response = self
            .request(reqwest::Method::POST, "/subscriptions")
            .json(&wrapper)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(braintree_error(response).await);
        }

        let result: BraintreeSubscriptionResponse = response.json().await?;

        let mut subscription = result.subscription.into_subscription();
        subscription.customer_id = Some(request.customer_id);
        if subscription.price_id.is_empty() {
            subscription.price_id = request.price_id;
        }
        subscription.quantity = request.quantity.unwrap_or(subscription.quantity);
        subscription.metadata = request.metadata;
        Ok(subscription)
    }

    async fn get_subscription(&self, id: &str) -> PaymentResult<Subscription> {
        let response = self
            .request(reqwest::Method::GET, &format!("/subscriptions/{}", id))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(braintree_error_not_found(
                response,
                PaymentError::SubscriptionNotFound(id.to_string()),
            )
            .await);
        }

        let result: BraintreeSubscriptionResponse = response.json().await?;
        Ok(result.subscription.into_subscription())
    }

    async fn update_subscription(&self, id: &str, price_id: &str) -> PaymentResult<Subscription> {
        let sub_req = BraintreeSubscriptionRequest {
            plan_id: price_id.to_string(),
            payment_method_token: None,
        };

        let wrapper = BraintreeSubscriptionWrapper {
            subscription: sub_req,
        };

        let response = self
            .request(reqwest::Method::PUT, &format!("/subscriptions/{}", id))
            .json(&wrapper)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(braintree_error_not_found(
                response,
                PaymentError::SubscriptionNotFound(id.to_string()),
            )
            .await);
        }

        // Braintree's PUT returns the full updated subscription; the follow-up
        // GET was a redundant round-trip.
        let result: BraintreeSubscriptionResponse = response.json().await?;
        Ok(result.subscription.into_subscription())
    }

    /// Cancel a Braintree subscription.
    ///
    /// Only immediate cancellation is supported: Braintree's cancel endpoint
    /// terminates the subscription at once and its Subscriptions API has no
    /// "cancel at period end" flag. `immediate: false` therefore returns
    /// [`PaymentError::Unsupported`] rather than silently cancelling
    /// immediately and forfeiting the remaining paid term.
    async fn cancel_subscription(&self, id: &str, immediate: bool) -> PaymentResult<Subscription> {
        if !immediate {
            return Err(PaymentError::Unsupported(format!(
                "Braintree cannot schedule subscription {id} to cancel at period end; its cancel \
                 endpoint terminates immediately. Let the current term lapse by setting the \
                 subscription's number of billing cycles, or call this with immediate = true if \
                 that is what you want."
            )));
        }

        let response = self
            .request(
                reqwest::Method::PUT,
                &format!("/subscriptions/{}/cancel", id),
            )
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(braintree_error_not_found(
                response,
                PaymentError::SubscriptionNotFound(id.to_string()),
            )
            .await);
        }

        let result: BraintreeSubscriptionResponse = response.json().await?;
        Ok(result.subscription.into_subscription())
    }

    async fn resume_subscription(&self, _id: &str) -> PaymentResult<Subscription> {
        Err(PaymentError::Unsupported(
            "Braintree has no resume endpoint: a canceled subscription is terminal and cannot be \
             reactivated. Create a new subscription instead."
                .into(),
        ))
    }

    /// Verify a Braintree webhook.
    ///
    /// `payload` must be the raw `bt_payload` form field exactly as Braintree
    /// sent it — the base64 string, not the decoded XML — because that is what
    /// Braintree HMACs. [`Self::parse_webhook`] decodes and parses those same
    /// bytes, so the signature covers precisely the content the application
    /// then acts on. See the [type-level docs](Self) for the full framing.
    async fn verify_webhook(&self, payload: &[u8], headers: &WebhookHeaders) -> PaymentResult<()> {
        // An empty public_key makes `split_once('|')` match a signature pair
        // with an empty key half (e.g. `bt_signature: "|<hmac>"`); an empty
        // private_key makes the HMAC key the SHA-1 digest of an empty
        // string, a public constant. Either turns verification into
        // something an attacker can forge outright — most realistically
        // under a missing-secret misconfiguration (e.g.
        // `env::var(..).unwrap_or_default()`). Reject both up front instead
        // of accepting a "signature" against a non-secret.
        if self.public_key.is_empty() || self.private_key.expose_secret().is_empty() {
            return Err(PaymentError::Config(
                "Braintree public_key and private_key must not be empty".into(),
            ));
        }

        let signature = headers.require(BRAINTREE_SIGNATURE_HEADER)?;

        // `bt_signature` is a comma-separated list of `public_key|hex_hmac`
        // pairs — one per key on the merchant account. Only the pair matching
        // our own public key can be verified.
        let expected_hex = signature
            .split('&')
            .flat_map(|group| group.split(','))
            .filter_map(|pair| pair.trim().split_once('|'))
            .find(|(key, _)| *key == self.public_key)
            .map(|(_, sig)| sig)
            .ok_or(PaymentError::InvalidWebhookSignature)?;

        let expected = hex::decode(expected_hex).map_err(|_| {
            // A malformed signature is a failed verification, not a pass.
            PaymentError::InvalidWebhookSignature
        })?;

        // Braintree derives the HMAC key by SHA-1-hashing the private key
        // first, then HMAC-SHA1s the payload with that digest.
        let key = Sha1::digest(self.private_key.expose_secret().as_bytes());
        let mut mac = Hmac::<Sha1>::new_from_slice(&key)
            .map_err(|_| PaymentError::InvalidWebhookSignature)?;
        mac.update(payload);

        // Constant-time comparison: a short-circuiting `==` would let an
        // attacker recover a valid signature byte by byte.
        mac.verify_slice(&expected)
            .map_err(|_| PaymentError::InvalidWebhookSignature)
    }

    /// Parse a Braintree webhook.
    ///
    /// `payload` is the raw `bt_payload` field: base64-encoded XML. This
    /// decodes the base64 and parses the `<notification>` document inside,
    /// which is the same byte string [`Self::verify_webhook`] authenticated.
    ///
    /// The `<subject>` element is converted into a [`serde_json::Value`] tree
    /// so downstream consumers see structured data rather than raw markup.
    fn parse_webhook(&self, payload: &[u8]) -> PaymentResult<WebhookEvent> {
        // Braintree tolerates whitespace/newlines in the transported field.
        let compact: Vec<u8> = payload
            .iter()
            .copied()
            .filter(|b| !b.is_ascii_whitespace())
            .collect();
        let xml = STANDARD.decode(&compact).map_err(|e| {
            PaymentError::Serialization(format!(
                "Braintree bt_payload is not valid base64: {e}. Pass the bt_payload form field \
                 verbatim, not the decoded XML."
            ))
        })?;

        let notification = parse_notification_xml(&xml)?;
        let event_type = WebhookEventType::from_str(&notification.kind);

        Ok(WebhookEvent {
            // Braintree's notification document carries no id of its own, so a
            // random UUID was previously minted per parse — which made
            // redelivery dedup impossible, since the same notification got a
            // different id every time. Deriving the id from the signed payload
            // instead makes it stable: a redelivery of the same notification
            // hashes to the same id and can be deduplicated, while distinct
            // notifications stay distinct.
            id: format!("bt_{}", hex::encode(Sha256::digest(&compact))),
            // Braintree's real timestamp, not "now".
            created_at: notification.timestamp.unwrap_or_else(Utc::now),
            // Interpret the subject according to the event type rather than
            // always emitting `Generic`. Braintree's XML-derived shapes rarely
            // match the (Stripe-modeled) typed variants, so in practice this
            // still yields `Generic` — but losslessly, and through the one code
            // path every provider now shares.
            data: WebhookData::from_event_type(&event_type, notification.subject),
            event_type,
            provider: "braintree".to_string(),
            livemode: true,
        })
    }
}

// Braintree API types

#[derive(Debug, Serialize)]
struct BraintreeWrapper<T> {
    transaction: T,
}

#[derive(Debug, Serialize)]
struct BraintreeTransactionRequest {
    amount: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    payment_method_nonce: Option<String>,
    /// Vaulted payment method token, used for `PaymentSource::PaymentMethod`.
    #[serde(skip_serializing_if = "Option::is_none")]
    payment_method_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    customer_id: Option<String>,
    /// Braintree's merchant-defined passthrough map, carrying
    /// [`ChargeRequest::metadata`].
    ///
    /// Braintree only stores keys that have been pre-registered as custom
    /// fields in the merchant's control panel; unregistered keys are rejected
    /// with a validation error rather than silently dropped.
    #[serde(skip_serializing_if = "Option::is_none")]
    custom_fields: Option<HashMap<String, String>>,
    options: BraintreeTransactionOptions,
}

#[derive(Debug, Serialize)]
struct BraintreeTransactionOptions {
    submit_for_settlement: bool,
}

#[derive(Debug, Deserialize)]
struct BraintreeTransactionResponse {
    transaction: BraintreeTransaction,
}

#[derive(Debug, Deserialize)]
struct BraintreeTransaction {
    id: String,
    amount: String,
    status: String,
    #[serde(default, alias = "currencyIsoCode")]
    currency_iso_code: Option<String>,
    #[serde(default, alias = "customerId")]
    customer_id: Option<String>,
    #[serde(default, alias = "paymentMethodToken")]
    payment_method_token: Option<String>,
    #[serde(default, alias = "processorResponseText")]
    processor_response_text: Option<String>,
    /// Braintree reports the real creation time; charges used to be stamped
    /// with `Utc::now()` at parse time instead.
    #[serde(default, alias = "createdAt")]
    created_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct BraintreeRefundRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    amount: Option<String>,
}

#[derive(Debug, Serialize)]
struct BraintreeCustomerWrapper {
    customer: BraintreeCustomerRequest,
}

#[derive(Debug, Serialize)]
struct BraintreeCustomerRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    phone: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BraintreeCustomerResponse {
    customer: BraintreeCustomer,
}

#[derive(Debug, Deserialize)]
struct BraintreeCustomer {
    id: String,
    email: Option<String>,
    #[serde(default, alias = "firstName")]
    first_name: Option<String>,
    #[serde(default, alias = "lastName")]
    last_name: Option<String>,
    phone: Option<String>,
    #[serde(default, alias = "creditCards")]
    credit_cards: Vec<BraintreeCreditCard>,
    #[serde(default, alias = "paypalAccounts")]
    paypal_accounts: Vec<BraintreePayPalAccount>,
    #[serde(default, alias = "createdAt")]
    created_at: Option<chrono::DateTime<Utc>>,
}

impl BraintreeCustomer {
    /// Project Braintree's customer resource onto the crate's [`Customer`].
    ///
    /// Shared by create/get/update so the three cannot drift apart.
    fn into_customer(self, metadata: HashMap<String, String>) -> Customer {
        let name = [self.first_name, self.last_name]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        Customer {
            id: self.id,
            email: self.email,
            // An empty join means Braintree reported neither name part; report
            // that as absent rather than as an empty name.
            name: if name.is_empty() { None } else { Some(name) },
            phone: self.phone,
            description: None,
            default_payment_method: None,
            address: None,
            metadata,
            created_at: self.created_at.unwrap_or_else(Utc::now),
        }
    }
}

#[derive(Debug, Deserialize)]
struct BraintreeCreditCard {
    token: String,
    #[serde(default, alias = "cardType")]
    card_type: Option<String>,
    #[serde(default, alias = "last4")]
    last_4: Option<String>,
    #[serde(default, alias = "expirationMonth")]
    expiration_month: Option<String>,
    #[serde(default, alias = "expirationYear")]
    expiration_year: Option<String>,
    #[serde(default, alias = "createdAt")]
    created_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct BraintreePayPalAccount {
    token: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default, alias = "createdAt")]
    created_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct BraintreeSettlementRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    amount: Option<String>,
}

#[derive(Debug, Serialize)]
struct BraintreePaymentMethodWrapper {
    payment_method: BraintreePaymentMethodRequest,
}

#[derive(Debug, Serialize)]
struct BraintreePaymentMethodRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    customer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payment_method_nonce: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BraintreePaymentMethodResponse {
    payment_method: BraintreePaymentMethod,
}

#[derive(Debug, Deserialize)]
struct BraintreePaymentMethod {
    token: String,
    #[serde(default, alias = "customerId")]
    customer_id: Option<String>,
    #[serde(default, alias = "cardType")]
    card_type: Option<String>,
    #[serde(default, alias = "last4")]
    last_4: Option<String>,
    #[serde(default, alias = "expirationMonth")]
    expiration_month: Option<u32>,
    #[serde(default, alias = "expirationYear")]
    expiration_year: Option<u32>,
    #[serde(default, alias = "createdAt")]
    created_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct BraintreeSubscriptionWrapper {
    subscription: BraintreeSubscriptionRequest,
}

#[derive(Debug, Serialize)]
struct BraintreeSubscriptionRequest {
    plan_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    payment_method_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BraintreeSubscriptionResponse {
    subscription: BraintreeSubscription,
}

#[derive(Debug, Deserialize)]
struct BraintreeSubscription {
    id: String,
    status: String,
    #[serde(default, alias = "planId")]
    plan_id: Option<String>,
    #[serde(default)]
    quantity: Option<u32>,
    #[serde(default, alias = "billingPeriodStartDate")]
    billing_period_start_date: Option<chrono::NaiveDate>,
    #[serde(default, alias = "billingPeriodEndDate")]
    billing_period_end_date: Option<chrono::NaiveDate>,
    #[serde(default, alias = "createdAt")]
    created_at: Option<chrono::DateTime<Utc>>,
    /// Braintree's flag for a subscription with no fixed end.
    ///
    /// `Some(false)` means the subscription runs for a fixed number of billing
    /// cycles and then stops of its own accord, which is exactly what
    /// [`Subscription::cancel_at_period_end`] describes.
    #[serde(default, alias = "neverExpires")]
    never_expires: Option<bool>,
}

impl BraintreeSubscription {
    /// Project Braintree's subscription resource onto the crate's
    /// [`Subscription`], leaving unreported fields as `None` instead of
    /// manufacturing a 30-day window anchored at "now".
    fn into_subscription(self) -> Subscription {
        Subscription {
            id: self.id,
            customer_id: None,
            status: match self.status.as_str() {
                "Active" => SubscriptionStatus::Active,
                "Canceled" => SubscriptionStatus::Canceled,
                "Past Due" => SubscriptionStatus::PastDue,
                "Pending" => SubscriptionStatus::Incomplete,
                "Expired" => SubscriptionStatus::Canceled,
                _ => SubscriptionStatus::Active,
            },
            current_period_start: self
                .billing_period_start_date
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|dt| dt.and_utc()),
            current_period_end: self
                .billing_period_end_date
                .and_then(|d| d.and_hms_opt(23, 59, 59))
                .map(|dt| dt.and_utc()),
            trial_end: None,
            // A subscription that does *not* never-expire will end when its
            // billing cycles run out — i.e. it is already set to stop at the
            // end of a period. Absent the flag, assume open-ended.
            cancel_at_period_end: self.never_expires.map(|never| !never).unwrap_or(false),
            canceled_at: None,
            price_id: self.plan_id.unwrap_or_default(),
            quantity: self.quantity.unwrap_or(1),
            metadata: HashMap::new(),
            created_at: self.created_at,
        }
    }
}

/// A decoded Braintree `<notification>` document.
struct BraintreeNotification {
    kind: String,
    timestamp: Option<chrono::DateTime<Utc>>,
    subject: serde_json::Value,
}

/// Maximum element nesting accepted in a notification document.
///
/// The XML tree is walked recursively, so a deeply nested document would
/// otherwise be able to exhaust the stack. Braintree's own notifications nest
/// perhaps five levels.
const MAX_XML_DEPTH: usize = 64;

/// Parse Braintree's XML webhook notification.
///
/// Braintree's notification format is
/// `<notification><timestamp/><kind/><subject/></notification>`, where
/// `<subject>` holds the resource the event concerns.
fn parse_notification_xml(xml: &[u8]) -> PaymentResult<BraintreeNotification> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    // Walk to the root <notification> element, skipping the XML declaration.
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref());
                if name != "notification" {
                    return Err(PaymentError::Serialization(format!(
                        "Braintree notification root element is <{name}>, expected <notification>"
                    )));
                }
                break;
            }
            Ok(Event::Eof) => {
                return Err(PaymentError::Serialization(
                    "Braintree notification XML contains no elements".into(),
                ));
            }
            Err(e) => {
                return Err(PaymentError::Serialization(format!(
                    "malformed Braintree notification XML: {e}"
                )));
            }
            _ => {}
        }
        buf.clear();
    }

    let root = read_element(&mut reader, false, false, 0)?;

    let kind = root
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            PaymentError::Serialization(
                "Braintree notification is missing its <kind> element".into(),
            )
        })?
        .to_string();

    let timestamp = root
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    let subject = root
        .get("subject")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    Ok(BraintreeNotification {
        kind,
        timestamp,
        subject,
    })
}

/// Strip any namespace prefix from an XML tag name.
fn local_name(raw: &[u8]) -> String {
    let name = String::from_utf8_lossy(raw);
    match name.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => name.into_owned(),
    }
}

/// Read the children of the element the reader has just entered, converting the
/// subtree into a [`serde_json::Value`].
///
/// Braintree annotates its XML with `type="array"` for repeated collections and
/// `nil="true"` for absent values; both are honoured so the resulting tree
/// distinguishes an empty string from a missing field.
fn read_element(
    reader: &mut quick_xml::Reader<&[u8]>,
    nil: bool,
    is_array: bool,
    depth: usize,
) -> PaymentResult<serde_json::Value> {
    use quick_xml::events::Event;
    use serde_json::{Map, Value};

    if depth > MAX_XML_DEPTH {
        return Err(PaymentError::Serialization(format!(
            "Braintree notification XML nests deeper than {MAX_XML_DEPTH} elements"
        )));
    }

    let mut text = String::new();
    let mut children: Vec<(String, Value)> = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref());
                let (child_nil, child_array) = element_hints(&e);
                let value = read_element(reader, child_nil, child_array, depth + 1)?;
                children.push((name, value));
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref());
                let (child_nil, child_array) = element_hints(&e);
                let value = if child_nil {
                    Value::Null
                } else if child_array {
                    Value::Array(Vec::new())
                } else {
                    Value::String(String::new())
                };
                children.push((name, value));
            }
            Ok(Event::Text(e)) => {
                let decoded = e.unescape().map_err(|err| {
                    PaymentError::Serialization(format!(
                        "malformed text in Braintree notification XML: {err}"
                    ))
                })?;
                text.push_str(&decoded);
            }
            Ok(Event::CData(e)) => {
                text.push_str(&String::from_utf8_lossy(&e));
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => {
                return Err(PaymentError::Serialization(
                    "Braintree notification XML ended inside an open element".into(),
                ));
            }
            Err(e) => {
                return Err(PaymentError::Serialization(format!(
                    "malformed Braintree notification XML: {e}"
                )));
            }
            _ => {}
        }
        buf.clear();
    }

    if nil {
        return Ok(Value::Null);
    }
    if children.is_empty() {
        return Ok(Value::String(text));
    }
    if is_array {
        return Ok(Value::Array(children.into_iter().map(|(_, v)| v).collect()));
    }

    // Repeated sibling names collapse into an array so no child is lost.
    let mut map = Map::new();
    for (name, value) in children {
        match map.entry(name) {
            serde_json::map::Entry::Vacant(slot) => {
                slot.insert(value);
            }
            serde_json::map::Entry::Occupied(mut slot) => {
                let existing = slot.get_mut();
                if let Value::Array(items) = existing {
                    items.push(value);
                } else {
                    let previous = existing.take();
                    *existing = Value::Array(vec![previous, value]);
                }
            }
        }
    }
    Ok(Value::Object(map))
}

/// Read Braintree's `nil="true"` and `type="array"` element annotations.
fn element_hints(e: &quick_xml::events::BytesStart<'_>) -> (bool, bool) {
    let mut nil = false;
    let mut is_array = false;
    for attr in e.attributes().flatten() {
        match attr.key.as_ref() {
            b"nil" => nil = attr.value.as_ref() == b"true",
            b"type" => is_array = attr.value.as_ref() == b"array",
            _ => {}
        }
    }
    (nil, is_array)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn provider() -> BraintreeProvider {
        BraintreeProvider::new("merchant123", "pubkey", "privkey").expect("HTTP client builds")
    }

    /// A realistic Braintree notification: base64-encoded XML, which is what
    /// the gateway actually POSTs as `bt_payload`.
    fn notification_payload(kind: &str) -> Vec<u8> {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<notification>
  <timestamp type="datetime">2024-03-01T10:15:30Z</timestamp>
  <kind>{kind}</kind>
  <subject>
    <subscription>
      <id>sub_42</id>
      <status>Active</status>
      <never-expires type="boolean">false</never-expires>
      <balance nil="true"/>
    </subscription>
  </subject>
</notification>"#
        );
        STANDARD.encode(xml).into_bytes()
    }

    // ---- finding 3: verify and parse operate on the same bytes ----

    #[test]
    fn parse_webhook_decodes_base64_xml() {
        let payload = notification_payload("subscription_charged_successfully");
        let event = provider().parse_webhook(&payload).expect("parses");

        assert_eq!(event.provider, "braintree");
        // The real timestamp from the document, not Utc::now().
        assert_eq!(event.created_at.to_rfc3339(), "2024-03-01T10:15:30+00:00");

        let WebhookData::Generic(subject) = &event.data else {
            panic!("expected generic webhook data");
        };
        assert_eq!(subject["subscription"]["id"], "sub_42");
        assert_eq!(subject["subscription"]["status"], "Active");
        // nil="true" is a genuine absence, not the empty string.
        assert!(subject["subscription"]["balance"].is_null());
    }

    #[test]
    fn parse_webhook_rejects_raw_json() {
        // The old implementation accepted JSON here. Real Braintree never
        // sends it, and accepting it is what let verify and parse disagree.
        let err = provider()
            .parse_webhook(br#"{"kind":"check","subject":{}}"#)
            .unwrap_err();
        assert!(matches!(err, PaymentError::Serialization(_)), "got {err:?}");
    }

    #[test]
    fn parse_webhook_rejects_non_notification_root() {
        let payload = STANDARD
            .encode("<other><kind>check</kind></other>")
            .into_bytes();
        assert!(provider().parse_webhook(&payload).is_err());
    }

    #[test]
    fn parse_webhook_event_id_is_stable_across_redeliveries() {
        // A random UUID per parse made redelivery dedup impossible.
        let payload = notification_payload("subscription_canceled");
        let first = provider().parse_webhook(&payload).unwrap();
        let second = provider().parse_webhook(&payload).unwrap();
        assert_eq!(first.id, second.id);

        let other = provider()
            .parse_webhook(&notification_payload("subscription_expired"))
            .unwrap();
        assert_ne!(first.id, other.id);
    }

    #[tokio::test]
    async fn verify_webhook_authenticates_the_bytes_parse_webhook_reads() {
        use hmac::Mac;

        let provider = provider();
        let payload = notification_payload("subscription_charged_successfully");

        // Sign exactly the bytes we will hand to parse_webhook.
        let key = Sha1::digest(b"privkey");
        let mut mac = Hmac::<Sha1>::new_from_slice(&key).unwrap();
        mac.update(&payload);
        let signature = format!("pubkey|{}", hex::encode(mac.finalize().into_bytes()));

        let headers = WebhookHeaders::new().with(BRAINTREE_SIGNATURE_HEADER, signature);

        provider
            .verify_webhook(&payload, &headers)
            .await
            .expect("signature over bt_payload verifies");
        // ...and the very same bytes parse.
        provider.parse_webhook(&payload).expect("same bytes parse");
    }

    // ---- finding 4: status and currency are read, not assumed ----

    #[test]
    fn refund_status_is_mapped_not_hardcoded() {
        assert_eq!(
            transaction_refund_status("settled"),
            RefundStatus::Succeeded
        );
        assert_eq!(
            transaction_refund_status("settling"),
            RefundStatus::Succeeded
        );
        assert_eq!(
            transaction_refund_status("submitted_for_settlement"),
            RefundStatus::Pending
        );
        assert_eq!(
            transaction_refund_status("processor_declined"),
            RefundStatus::Failed
        );
        assert_eq!(
            transaction_refund_status("gateway_rejected"),
            RefundStatus::Failed
        );
        assert_eq!(transaction_refund_status("failed"), RefundStatus::Failed);
        assert_eq!(transaction_refund_status("voided"), RefundStatus::Canceled);
    }

    #[test]
    fn charge_status_maps_declines_to_failed() {
        assert_eq!(
            transaction_charge_status("settled"),
            ChargeStatus::Succeeded
        );
        assert_eq!(
            transaction_charge_status("processor_declined"),
            ChargeStatus::Failed
        );
        assert_eq!(
            transaction_charge_status("authorization_expired"),
            ChargeStatus::Pending
        );
    }

    #[test]
    fn a_malformed_transaction_amount_is_an_error_not_zero() {
        // The defect this replaces: `parse().unwrap_or(0.0)` produced a Charge
        // reporting $0.00 for money that had moved, on the *success* path, so
        // no ledger entry or reconciliation run had any signal.
        let txn = |amount: &str, iso: &str| BraintreeTransaction {
            id: "tx_1".into(),
            amount: amount.into(),
            status: "settled".into(),
            currency_iso_code: Some(iso.into()),
            customer_id: None,
            payment_method_token: None,
            processor_response_text: None,
            created_at: None,
        };

        for bad in ["", "N/A", "12,50", "1.2.3", "$120.00", "abc"] {
            let err = transaction_amount(&txn(bad, "USD")).unwrap_err();
            assert!(matches!(err, PaymentError::Serialization(_)), "got {err:?}");
            assert!(
                err.to_string().contains("unparseable"),
                "the message must say what went wrong; got {err}"
            );
            assert!(err.to_string().contains("tx_1"), "got {err}");
        }

        // More precision than the currency has minor units is also refused
        // rather than silently rounded into the ledger.
        assert!(transaction_amount(&txn("120.005", "USD")).is_err());
        assert!(transaction_amount(&txn("1000.50", "JPY")).is_err());
    }

    #[test]
    fn a_well_formed_transaction_amount_is_exact() {
        let txn = |amount: &str, iso: &str| BraintreeTransaction {
            id: "tx_1".into(),
            amount: amount.into(),
            status: "settled".into(),
            currency_iso_code: Some(iso.into()),
            customer_id: None,
            payment_method_token: None,
            processor_response_text: None,
            created_at: None,
        };

        assert_eq!(
            transaction_amount(&txn("120.00", "EUR")).unwrap(),
            Money::eur(12000)
        );
        // Through f64 this is 28.999999999999996 before rounding.
        assert_eq!(
            transaction_amount(&txn("0.29", "USD")).unwrap(),
            Money::usd(29)
        );
        // Zero-decimal currencies keep whole units.
        assert_eq!(
            transaction_amount(&txn("1000", "JPY")).unwrap(),
            Money::new(1000, Currency::JPY)
        );
    }

    #[test]
    fn transaction_currency_is_read_from_the_gateway() {
        // A EUR settlement previously came back tagged USD, which passes
        // Money::add's currency assertion and corrupts a ledger.
        assert_eq!(transaction_currency(Some("EUR")), Currency::EUR);
        assert_eq!(transaction_currency(Some("JPY")), Currency::JPY);
        assert_eq!(transaction_currency(None), Currency::USD);
        assert_eq!(transaction_currency(Some("nonsense")), Currency::USD);
    }

    // ---- finding 10: every PaymentSource variant is routed or rejected ----

    #[tokio::test]
    async fn charge_rejects_unroutable_payment_sources() {
        let provider = provider();

        for source in [
            PaymentSource::customer("cust_1"),
            PaymentSource::Bank {
                token: "bank_1".into(),
            },
        ] {
            let err = provider
                .charge(ChargeRequest::new(Money::usd(1000), source))
                .await
                .unwrap_err();
            // Previously these posted with neither nonce nor token.
            assert!(matches!(err, PaymentError::Validation(_)), "got {err:?}");
        }
    }

    // ---- finding 11: end-of-period cancellation is not silently upgraded ----

    #[tokio::test]
    async fn cancel_subscription_refuses_end_of_period() {
        let err = provider()
            .cancel_subscription("sub_1", false)
            .await
            .unwrap_err();
        assert!(matches!(err, PaymentError::Unsupported(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn resume_subscription_is_unsupported() {
        let err = provider().resume_subscription("sub_1").await.unwrap_err();
        assert!(matches!(err, PaymentError::Unsupported(_)), "got {err:?}");
    }

    // ---- finding 1: base URL validation ----

    #[test]
    fn with_base_url_rejects_cleartext_http() {
        // Braintree sends its key pair as HTTP Basic on every request.
        assert!(
            provider()
                .with_base_url("http://gateway.example.com")
                .is_err()
        );
        assert!(provider().with_base_url("ftp://example.com").is_err());
        assert!(provider().with_base_url("not a url").is_err());
    }

    #[test]
    fn with_base_url_accepts_https_and_loopback() {
        assert!(provider().with_base_url("https://example.com").is_ok());
        assert!(provider().with_base_url("http://127.0.0.1:8080").is_ok());
        assert!(provider().with_base_url("http://localhost:8080").is_ok());
    }

    #[test]
    fn with_base_url_preserves_merchant_path_without_double_slash() {
        let p = provider().with_base_url("http://localhost:8080/").unwrap();
        assert_eq!(p.base_url, "http://localhost:8080/merchants/merchant123");
    }

    #[test]
    fn production_keeps_the_merchant_id() {
        let p = provider().production();
        assert_eq!(
            p.base_url,
            "https://api.braintreegateway.com/merchants/merchant123"
        );
    }

    // ---- finding 2: Braintree has no idempotency mechanism ----

    #[test]
    fn does_not_claim_idempotency_support() {
        // The processor must refuse to retry Braintree money movement.
        assert!(!provider().supports_idempotency());
    }

    // ---- finding 17: never_expires drives cancel_at_period_end ----

    #[test]
    fn never_expires_maps_to_cancel_at_period_end() {
        let sub = |never: Option<bool>| BraintreeSubscription {
            id: "s".into(),
            status: "Active".into(),
            plan_id: None,
            quantity: None,
            billing_period_start_date: None,
            billing_period_end_date: None,
            created_at: None,
            never_expires: never,
        };

        // A fixed-term subscription is already set to stop at period end.
        assert!(sub(Some(false)).into_subscription().cancel_at_period_end);
        assert!(!sub(Some(true)).into_subscription().cancel_at_period_end);
        assert!(!sub(None).into_subscription().cancel_at_period_end);
    }

    // ---- XML converter behaviour ----

    #[test]
    fn repeated_elements_collapse_into_an_array() {
        let xml = br#"<notification><kind>check</kind><subject>
            <items><item>a</item><item>b</item></items>
        </subject></notification>"#;
        let parsed = parse_notification_xml(xml).unwrap();
        assert_eq!(parsed.subject["items"]["item"][0], "a");
        assert_eq!(parsed.subject["items"]["item"][1], "b");
    }

    #[test]
    fn missing_kind_is_an_error() {
        let xml = br#"<notification><subject/></notification>"#;
        assert!(parse_notification_xml(xml).is_err());
    }

    #[test]
    fn deep_nesting_is_bounded() {
        let mut xml = String::from("<notification><kind>check</kind><subject>");
        for _ in 0..(MAX_XML_DEPTH + 10) {
            xml.push_str("<a>");
        }
        for _ in 0..(MAX_XML_DEPTH + 10) {
            xml.push_str("</a>");
        }
        xml.push_str("</subject></notification>");
        assert!(parse_notification_xml(xml.as_bytes()).is_err());
    }
}
