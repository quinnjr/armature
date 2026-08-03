//! Webhook client for sending outgoing webhooks

use crate::{
    Result, WebhookConfig, WebhookDelivery, WebhookDeliveryStatus, WebhookEndpoint, WebhookError,
    WebhookPayload, WebhookRegistry, WebhookSignature,
};
use bytes::Bytes;
use chrono::Utc;
use futures::stream::StreamExt;
use reqwest::Client;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Client for sending webhook deliveries
#[derive(Debug, Clone)]
pub struct WebhookClient {
    config: WebhookConfig,
    http_client: Client,
    registry: Option<Arc<WebhookRegistry>>,
}

impl WebhookClient {
    /// Create a new webhook client with default configuration
    pub fn new(config: WebhookConfig) -> Self {
        let http_client = Client::builder()
            .timeout(config.timeout)
            .user_agent(&config.user_agent)
            .danger_accept_invalid_certs(!config.verify_ssl)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            config,
            http_client,
            registry: None,
        }
    }

    /// Create a webhook client with a registry for endpoint management
    pub fn with_registry(config: WebhookConfig, registry: Arc<WebhookRegistry>) -> Self {
        let mut client = Self::new(config);
        client.registry = Some(registry);
        client
    }

    /// Send a webhook to a URL
    ///
    /// Note: this does **not** sign the request. `WebhookClient` has no
    /// client-level default signing secret, so without one the request is
    /// sent unsigned (no `X-Webhook-Signature` header). Use
    /// [`WebhookClient::send_with_secret`] to sign the request with a
    /// specific secret, or [`WebhookClient::send_to_endpoint`] /
    /// [`WebhookClient::dispatch`] for registry-managed endpoints, which are
    /// always signed with the endpoint's configured secret.
    pub async fn send(&self, url: &str, payload: WebhookPayload) -> Result<WebhookDelivery> {
        self.send_with_secret(url, payload, None).await
    }

    /// Send a webhook with a specific signing secret
    pub async fn send_with_secret(
        &self,
        url: &str,
        payload: WebhookPayload,
        secret: Option<&str>,
    ) -> Result<WebhookDelivery> {
        let mut delivery = WebhookDelivery::new(payload, url);
        delivery.status = WebhookDeliveryStatus::InProgress;

        let body = Bytes::from(delivery.payload.to_bytes()?);

        // Check payload size
        if body.len() > self.config.max_payload_size {
            delivery.mark_failed(
                format!(
                    "Payload too large: {} bytes (max: {})",
                    body.len(),
                    self.config.max_payload_size
                ),
                None,
            );
            return Ok(delivery);
        }

        // Build request with signature if secret provided
        let mut request = self
            .http_client
            .post(url)
            .header("Content-Type", "application/json")
            .header("X-Webhook-Id", &delivery.id)
            .header("X-Webhook-Event", &delivery.payload.event);

        if let Some(secret) = secret {
            let signer =
                WebhookSignature::new(secret).with_algorithm(self.config.signing_algorithm);
            let signature = signer.sign(&body);
            request = request.header("X-Webhook-Signature", signature);
        }

        // Execute with retries
        self.execute_with_retries(&mut delivery, request.body(body))
            .await?;

        Ok(delivery)
    }

    /// Send a webhook to a registered endpoint
    ///
    /// Serializes `payload` to JSON internally. When sending to multiple
    /// endpoints for the same payload (see [`WebhookClient::dispatch`]),
    /// prefer [`WebhookClient::send_to_endpoint_with_body`] with a
    /// pre-serialized body to avoid re-serializing per endpoint.
    pub async fn send_to_endpoint(
        &self,
        endpoint: &WebhookEndpoint,
        payload: WebhookPayload,
    ) -> Result<WebhookDelivery> {
        let body = Bytes::from(payload.to_bytes()?);
        self.send_to_endpoint_with_body(endpoint, payload, body)
            .await
    }

    /// Send a webhook to a registered endpoint using an already-serialized body
    ///
    /// This lets callers that dispatch the same payload to many endpoints
    /// serialize the body once and share it, only re-doing the (cheap,
    /// per-endpoint) HMAC signing for each endpoint.
    ///
    /// The body is a [`Bytes`] rather than a `Vec<u8>` precisely so that
    /// sharing it is a refcount bump: with a `Vec` the "shared" body was
    /// deep-copied once per endpoint, so a 1 MiB payload fanned out to 5,000
    /// subscribers allocated 5 GiB.
    pub async fn send_to_endpoint_with_body(
        &self,
        endpoint: &WebhookEndpoint,
        payload: WebhookPayload,
        body: Bytes,
    ) -> Result<WebhookDelivery> {
        let mut delivery = WebhookDelivery::new(payload, &endpoint.url);
        delivery.status = WebhookDeliveryStatus::InProgress;

        // Check payload size
        if body.len() > self.config.max_payload_size {
            delivery.mark_failed(
                format!(
                    "Payload too large: {} bytes (max: {})",
                    body.len(),
                    self.config.max_payload_size
                ),
                None,
            );
            return Ok(delivery);
        }

        // Sign the payload
        let signer =
            WebhookSignature::new(&endpoint.secret).with_algorithm(self.config.signing_algorithm);
        let signature = signer.sign(&body);

        // Build request
        let mut request = self
            .http_client
            .post(&endpoint.url)
            .header("Content-Type", "application/json")
            .header("X-Webhook-Id", &delivery.id)
            .header("X-Webhook-Event", &delivery.payload.event)
            .header("X-Webhook-Signature", signature);

        // Add custom headers from endpoint
        for (key, value) in &endpoint.headers {
            request = request.header(key, value);
        }

        // Execute with retries
        self.execute_with_retries(&mut delivery, request.body(body))
            .await?;

        // Update endpoint in registry if available
        if let Some(ref registry) = self.registry {
            if delivery.status == WebhookDeliveryStatus::Succeeded {
                let _ = registry.record_success(&endpoint.id);
            } else {
                let _ = registry.record_failure(&endpoint.id);
            }
        }

        Ok(delivery)
    }

    /// Dispatch a payload to all endpoints subscribed to the event
    ///
    /// The payload is serialized to JSON exactly once and the resulting body
    /// is shared across all endpoints; only the (cheap) HMAC signature is
    /// recomputed per endpoint.
    ///
    /// A per-endpoint failure (including a serialization or transport error)
    /// does not prevent delivery to the other endpoints: this method
    /// aggregates a [`WebhookDelivery`] for every endpoint, succeeded or
    /// failed, into the returned `Vec`. The overall `Err` case is reserved
    /// for fatal errors that occur before any delivery is attempted (e.g. no
    /// registry configured).
    ///
    /// At most [`WebhookConfig::max_concurrent_deliveries`] deliveries are in
    /// flight at a time. The previous `join_all` started every one of them at
    /// once, so an account with thousands of subscribed endpoints opened
    /// thousands of sockets simultaneously and ran the sender out of file
    /// descriptors. Results are still returned in registry order, not
    /// completion order.
    pub async fn dispatch(&self, payload: WebhookPayload) -> Result<Vec<WebhookDelivery>> {
        let registry = self.registry.as_ref().ok_or_else(|| {
            WebhookError::ConfigError("No registry configured for dispatch".to_string())
        })?;

        let endpoints = registry.get_endpoints_for_event(&payload.event);

        // Serialize the payload once and share the bytes across all endpoints;
        // `Bytes::clone` is a refcount bump, not a copy of the body.
        let body = Bytes::from(payload.to_bytes()?);
        let concurrency = self.config.max_concurrent_deliveries.max(1);

        // Each task carries its endpoint index so the out-of-order completions
        // from `buffer_unordered` can be put back in registry order.
        let completed: Vec<(usize, Result<WebhookDelivery>)> =
            futures::stream::iter(endpoints.iter().enumerate().map(|(index, endpoint)| {
                let payload = payload.clone();
                let body = body.clone();
                async move {
                    (
                        index,
                        self.send_to_endpoint_with_body(endpoint, payload, body)
                            .await,
                    )
                }
            }))
            .buffer_unordered(concurrency)
            .collect()
            .await;

        // Aggregate partial progress: a failure delivering to one endpoint
        // must not drop the results of the others.
        let mut slots: Vec<Option<WebhookDelivery>> = (0..endpoints.len()).map(|_| None).collect();
        for (index, delivery_result) in completed {
            slots[index] = Some(match delivery_result {
                Ok(delivery) => delivery,
                Err(e) => {
                    let mut delivery =
                        WebhookDelivery::new(payload.clone(), endpoints[index].url.clone());
                    delivery.mark_failed(e.to_string(), None);
                    delivery
                }
            });
        }

        Ok(slots.into_iter().flatten().collect())
    }

    /// Execute a request with retry policy
    async fn execute_with_retries(
        &self,
        delivery: &mut WebhookDelivery,
        request: reqwest::RequestBuilder,
    ) -> Result<()> {
        let policy = &self.config.retry_policy;
        let mut attempt = 0;

        loop {
            attempt += 1;
            debug!(
                "Webhook delivery attempt {} for {}",
                attempt, delivery.endpoint_url
            );

            // Clone the request for this attempt
            let request = request
                .try_clone()
                .ok_or_else(|| WebhookError::Internal("Failed to clone request".to_string()))?;

            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    let body = self.read_capped_body(response).await;

                    if status.is_success() {
                        info!(
                            "Webhook delivered successfully to {} (attempt {})",
                            delivery.endpoint_url, attempt
                        );
                        delivery.mark_succeeded(status.as_u16(), body);
                        return Ok(());
                    }

                    // Check if we should retry based on status code
                    let should_retry =
                        Self::should_retry_status(status.as_u16()) && policy.should_retry(attempt);

                    let next_retry = if should_retry {
                        let delay = policy.delay_for_attempt(attempt);
                        Some(Utc::now() + chrono::Duration::from_std(delay).unwrap_or_default())
                    } else {
                        None
                    };

                    warn!(
                        "Webhook delivery failed with status {} (attempt {})",
                        status, attempt
                    );
                    delivery.mark_failed_with_status(status.as_u16(), body, next_retry);

                    if !should_retry {
                        return Ok(());
                    }

                    // Wait before retry
                    let delay = policy.delay_for_attempt(attempt);
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    let should_retry = policy.should_retry(attempt);
                    let next_retry = if should_retry {
                        let delay = policy.delay_for_attempt(attempt);
                        Some(Utc::now() + chrono::Duration::from_std(delay).unwrap_or_default())
                    } else {
                        None
                    };

                    error!("Webhook delivery error: {} (attempt {})", e, attempt);
                    delivery.mark_failed(e.to_string(), next_retry);

                    if !should_retry {
                        return Ok(());
                    }

                    // Wait before retry
                    let delay = policy.delay_for_attempt(attempt);
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    /// Read at most [`WebhookConfig::max_response_body_size`] bytes of a
    /// response body, for the record kept on the delivery.
    ///
    /// This reads chunk by chunk and stops at the cap rather than calling
    /// `response.text()`, which buffers the whole body first: the receiver of a
    /// webhook is by definition a third party, and one that answers a delivery
    /// with a multi-gigabyte body — hostile or merely misconfigured — could
    /// otherwise take the sender's process down. Whatever is left of the body
    /// is dropped unread, which closes the connection instead of draining it.
    ///
    /// Returns `None` only when nothing at all could be read, matching the
    /// previous `response.text().await.ok()`. Truncation can land mid-codepoint,
    /// so the bytes are decoded lossily; this string is diagnostic, never
    /// something to parse.
    async fn read_capped_body(&self, mut response: reqwest::Response) -> Option<String> {
        let cap = self.config.max_response_body_size;
        let mut buf: Vec<u8> = Vec::new();
        let mut failed = false;

        while buf.len() < cap {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    let take = (cap - buf.len()).min(chunk.len());
                    buf.extend_from_slice(&chunk[..take]);
                    if take < chunk.len() {
                        warn!(
                            "Webhook response body exceeded {} bytes; recording a truncated copy",
                            cap
                        );
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    debug!("Failed to read webhook response body: {}", e);
                    failed = true;
                    break;
                }
            }
        }

        if failed && buf.is_empty() {
            return None;
        }
        Some(String::from_utf8_lossy(&buf).into_owned())
    }

    /// Determine if a status code should be retried
    fn should_retry_status(status: u16) -> bool {
        matches!(
            status,
            408 | 429 | 500 | 502 | 503 | 504 // Timeout, rate limit, server errors
        )
    }

    /// Get the configuration
    pub fn config(&self) -> &WebhookConfig {
        &self.config
    }

    /// Get the registry (if configured)
    pub fn registry(&self) -> Option<&Arc<WebhookRegistry>> {
        self.registry.as_ref()
    }
}

impl Default for WebhookClient {
    fn default() -> Self {
        Self::new(WebhookConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WebhookEndpoint;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn test_client_creation() {
        let client = WebhookClient::default();
        assert_eq!(client.config().timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_client_with_config() {
        let config = WebhookConfig::builder()
            .timeout_secs(60)
            .no_retries()
            .build();

        let client = WebhookClient::new(config);
        assert_eq!(client.config().timeout, Duration::from_secs(60));
        assert_eq!(client.config().retry_policy.max_attempts, 0);
    }

    #[test]
    fn test_should_retry_status() {
        // Should retry
        assert!(WebhookClient::should_retry_status(429));
        assert!(WebhookClient::should_retry_status(500));
        assert!(WebhookClient::should_retry_status(502));
        assert!(WebhookClient::should_retry_status(503));
        assert!(WebhookClient::should_retry_status(504));

        // Should not retry
        assert!(!WebhookClient::should_retry_status(200));
        assert!(!WebhookClient::should_retry_status(400));
        assert!(!WebhookClient::should_retry_status(401));
        assert!(!WebhookClient::should_retry_status(404));
    }

    #[tokio::test]
    async fn test_payload_too_large() {
        let config = WebhookConfig::builder().max_payload_size(10).build();
        let client = WebhookClient::new(config);

        let payload = WebhookPayload::new("test")
            .with_data(serde_json::json!({"large": "This is definitely more than 10 bytes"}));

        let delivery = client
            .send("http://localhost:9999/webhook", payload)
            .await
            .unwrap();

        assert_eq!(delivery.status, WebhookDeliveryStatus::PermanentlyFailed);
        assert!(delivery.last_error.unwrap().contains("too large"));
    }

    #[tokio::test]
    async fn test_dispatch_delivers_to_all_endpoints_and_collects_results() {
        let server1 = MockServer::start().await;
        let server2 = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server1)
            .await;

        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server2)
            .await;

        let registry = Arc::new(WebhookRegistry::new());
        registry.register(
            WebhookEndpoint::builder(format!("{}/hook", server1.uri()))
                .events(vec!["order.created"])
                .build(),
        );
        registry.register(
            WebhookEndpoint::builder(format!("{}/hook", server2.uri()))
                .events(vec!["order.created"])
                .build(),
        );

        let client = WebhookClient::with_registry(WebhookConfig::default(), registry);
        let payload = WebhookPayload::new("order.created").with_data(serde_json::json!({}));

        let deliveries = client.dispatch(payload).await.unwrap();

        assert_eq!(deliveries.len(), 2);
        assert!(
            deliveries
                .iter()
                .all(|d| d.status == WebhookDeliveryStatus::Succeeded)
        );
    }

    /// `buffer_unordered` completes deliveries out of order, so the
    /// aggregation step has to re-key each result by its endpoint index. The
    /// old `zip` of completion order against registry order would attribute
    /// the slow endpoint's outcome to the fast one.
    #[tokio::test]
    async fn test_dispatch_pairs_each_result_with_its_own_endpoint() {
        let slow_ok = MockServer::start().await;
        let fast_bad = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(150)))
            .mount(&slow_ok)
            .await;

        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&fast_bad)
            .await;

        let registry = Arc::new(WebhookRegistry::new());
        registry.register(
            WebhookEndpoint::builder(format!("{}/hook", slow_ok.uri()))
                .events(vec!["order.created"])
                .build(),
        );
        registry.register(
            WebhookEndpoint::builder(format!("{}/hook", fast_bad.uri()))
                .events(vec!["order.created"])
                .build(),
        );

        let config = WebhookConfig::builder()
            .max_concurrent_deliveries(4)
            .no_retries()
            .build();
        let client = WebhookClient::with_registry(config, registry);

        let deliveries = client
            .dispatch(WebhookPayload::new("order.created"))
            .await
            .unwrap();

        assert_eq!(deliveries.len(), 2);
        for delivery in deliveries {
            if delivery.endpoint_url.starts_with(&slow_ok.uri()) {
                assert_eq!(delivery.last_status_code, Some(200));
            } else {
                assert_eq!(delivery.last_status_code, Some(400));
            }
        }
    }

    /// A concurrency limit throttles the fan-out; it must not drop endpoints.
    #[tokio::test]
    async fn test_dispatch_delivers_to_every_endpoint_with_a_limit_of_one() {
        let registry = Arc::new(WebhookRegistry::new());
        let mut servers = Vec::new();

        for _ in 0..3 {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/hook"))
                .respond_with(ResponseTemplate::new(200))
                .expect(1)
                .mount(&server)
                .await;
            registry.register(
                WebhookEndpoint::builder(format!("{}/hook", server.uri()))
                    .events(vec!["order.created"])
                    .build(),
            );
            servers.push(server);
        }

        let config = WebhookConfig::builder()
            .max_concurrent_deliveries(1)
            .build();
        let client = WebhookClient::with_registry(config, registry);

        let deliveries = client
            .dispatch(WebhookPayload::new("order.created"))
            .await
            .unwrap();

        assert_eq!(deliveries.len(), 3);
        assert!(
            deliveries
                .iter()
                .all(|d| d.status == WebhookDeliveryStatus::Succeeded)
        );
    }

    /// A hostile receiver must not be able to make the sender buffer its whole
    /// response body.
    #[tokio::test]
    async fn test_response_body_read_stops_at_the_configured_cap() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(200_000)))
            .mount(&server)
            .await;

        let config = WebhookConfig::builder().max_response_body_size(64).build();
        let client = WebhookClient::new(config);

        let response = client
            .http_client
            .post(format!("{}/hook", server.uri()))
            .send()
            .await
            .unwrap();

        let body = client.read_capped_body(response).await.unwrap();
        assert_eq!(body.len(), 64, "response body was not capped at the read");
    }

    #[tokio::test]
    async fn test_send_to_endpoint_signs_with_configured_algorithm() {
        // Drives the REAL send path (send_to_endpoint -> send_to_endpoint_with_body
        // -> execute_with_retries) against a live mock server and inspects the
        // actual request that was sent, so this test would fail if the
        // `.with_algorithm(self.config.signing_algorithm)` wiring in
        // `send_to_endpoint_with_body` were ever deleted or broken.
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let secret = "secret";
        let config = WebhookConfig::builder()
            .signing_algorithm(crate::SigningAlgorithm::HmacSha512)
            .build();
        let client = WebhookClient::new(config);

        let endpoint = WebhookEndpoint::builder(format!("{}/hook", server.uri()))
            .secret(secret)
            .events(vec!["order.created"])
            .build();

        let payload = WebhookPayload::new("order.created").with_data(serde_json::json!({}));
        let body = payload.to_bytes().unwrap();

        let delivery = client
            .send_to_endpoint(&endpoint, payload.clone())
            .await
            .unwrap();
        assert_eq!(delivery.status, WebhookDeliveryStatus::Succeeded);

        // Inspect the actual request captured by the mock server.
        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);

        let signature_header = received[0]
            .headers
            .get("X-Webhook-Signature")
            .expect("request must carry X-Webhook-Signature")
            .to_str()
            .unwrap();

        // SHA-512 hex digest is 128 chars; SHA-256 would be 64.
        let v1 = signature_header.split("v1=").nth(1).unwrap();
        assert_eq!(v1.len(), 128);

        // And it must be an actual valid HMAC-SHA512 signature over the body,
        // independently verified via `WebhookSignature` configured for SHA-512.
        let independent_verifier =
            WebhookSignature::new(secret).with_algorithm(crate::SigningAlgorithm::HmacSha512);
        assert!(
            independent_verifier
                .verify(&body, signature_header, 300)
                .unwrap()
        );

        // And an independent SHA-256 verifier must reject it (proves the
        // configured algorithm, not the default, was actually used).
        let sha256_verifier = WebhookSignature::new(secret);
        assert!(
            !sha256_verifier
                .verify(&body, signature_header, 300)
                .unwrap()
        );
    }
}
