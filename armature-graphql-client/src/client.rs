//! GraphQL client implementation.

use futures::StreamExt;
use reqwest::Client;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{debug, info};

use crate::request::GraphQLRequest;
use crate::subscription::protocol::{ClientMessage, ServerMessage, SubscribePayload};
use crate::{
    BatchRequest, BatchResponse, GraphQLClientConfig, GraphQLError, GraphQLResponse,
    MutationBuilder, QueryBuilder, Result, SubscriptionBuilder, SubscriptionStream,
};

/// A cached response entry, along with the time it was inserted.
type CacheEntry = (Instant, GraphQLResponse<Value>);

/// GraphQL client.
#[derive(Clone)]
pub struct GraphQLClient {
    http_client: Client,
    config: Arc<GraphQLClientConfig>,
    cache: Arc<Mutex<HashMap<String, CacheEntry>>>,
}

impl GraphQLClient {
    /// Create a new GraphQL client with the given endpoint.
    pub fn new(endpoint: impl Into<String>) -> Self {
        let config = GraphQLClientConfig::new(endpoint);
        Self::with_config(config)
    }

    /// Create a new GraphQL client with custom configuration.
    pub fn with_config(config: GraphQLClientConfig) -> Self {
        let http_client = Client::builder()
            .timeout(config.timeout)
            .user_agent(&config.user_agent)
            .gzip(true)
            .build()
            .expect("Failed to build HTTP client");

        Self {
            http_client,
            config: Arc::new(config),
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get the configuration.
    pub fn config(&self) -> &GraphQLClientConfig {
        &self.config
    }

    /// Create a query builder.
    pub fn query(&self, query: impl Into<String>) -> QueryBuilder<'_> {
        QueryBuilder::new(self, query)
    }

    /// Create a mutation builder.
    pub fn mutation(&self, mutation: impl Into<String>) -> MutationBuilder<'_> {
        MutationBuilder::new(self, mutation)
    }

    /// Create a subscription builder.
    pub fn subscribe(&self, subscription: impl Into<String>) -> SubscriptionBuilder<'_> {
        SubscriptionBuilder::new(self, subscription)
    }

    /// Execute a batch of requests.
    pub async fn batch(&self, batch: BatchRequest) -> Result<BatchResponse> {
        if batch.is_empty() {
            return Ok(BatchResponse::new(Vec::new()));
        }

        debug!(count = batch.len(), "Executing batch request");

        let requests = batch.into_requests();
        let responses: Vec<GraphQLResponse<Value>> =
            self.send_with_retry(&requests, &[], None).await?;
        Ok(BatchResponse::new(responses))
    }

    /// Execute a single request (no caching).
    pub(crate) async fn execute_request(
        &self,
        request: GraphQLRequest,
        extra_headers: Vec<(String, String)>,
        timeout: Option<Duration>,
    ) -> Result<GraphQLResponse<Value>> {
        debug!(query = %request.query, "Executing GraphQL request");
        self.send_with_retry(&request, &extra_headers, timeout)
            .await
    }

    /// Execute a single request, transparently serving/populating the response
    /// cache when caching is enabled and the request is cacheable (queries only,
    /// never mutations).
    pub(crate) async fn execute_request_cached(
        &self,
        request: GraphQLRequest,
        extra_headers: Vec<(String, String)>,
        timeout: Option<Duration>,
        cacheable: bool,
    ) -> Result<GraphQLResponse<Value>> {
        if !cacheable || !self.config.caching {
            return self.execute_request(request, extra_headers, timeout).await;
        }

        let key = Self::cache_key(&request, &extra_headers);

        if let Some((inserted_at, cached)) = self.cache.lock().unwrap().get(&key).cloned()
            && inserted_at.elapsed() < self.config.cache_ttl
        {
            debug!(query = %request.query, "Serving GraphQL response from cache");
            return Ok(cached);
        }

        let response = self
            .execute_request(request, extra_headers, timeout)
            .await?;
        self.cache
            .lock()
            .unwrap()
            .insert(key, (Instant::now(), response.clone()));
        Ok(response)
    }

    /// Build a cache key from a request's query, operation name, variables, and headers.
    fn cache_key(request: &GraphQLRequest, extra_headers: &[(String, String)]) -> String {
        let variables = request
            .variables
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default();
        let headers = extra_headers
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{}\u{0}{}\u{0}{}\u{0}{}",
            request.query,
            request.operation_name.as_deref().unwrap_or(""),
            variables,
            headers
        )
    }

    /// Whether an error is worth retrying: transport-level failures (connection
    /// errors, timeouts) or HTTP 5xx responses. A well-formed GraphQL error
    /// response (HTTP 200 with a populated `errors` array) is never an `Err`
    /// at this layer, so it is never retried.
    fn is_retryable(err: &GraphQLError) -> bool {
        match err {
            GraphQLError::Http(e) => match e.status() {
                Some(status) => status.is_server_error(),
                None => true,
            },
            _ => false,
        }
    }

    /// Send a JSON-serializable request body to the GraphQL endpoint, retrying
    /// according to the client's retry configuration.
    async fn send_with_retry<Req, Resp>(
        &self,
        body: &Req,
        extra_headers: &[(String, String)],
        timeout: Option<Duration>,
    ) -> Result<Resp>
    where
        Req: Serialize + ?Sized,
        Resp: DeserializeOwned,
    {
        let max_attempts = if self.config.retry_enabled {
            self.config.max_retries.saturating_add(1)
        } else {
            1
        };

        let mut attempt: u32 = 0;
        loop {
            attempt += 1;

            let mut http_request = self.http_client.post(&self.config.endpoint);

            for (name, value) in &self.config.default_headers {
                http_request = http_request.header(name.as_str(), value.as_str());
            }
            for (name, value) in extra_headers {
                http_request = http_request.header(name.as_str(), value.as_str());
            }
            http_request = http_request.header("Content-Type", "application/json");

            if let Some(timeout) = timeout {
                http_request = http_request.timeout(timeout);
            }

            let attempt_result: Result<Resp> = async {
                let response = http_request.json(body).send().await?;
                if !response.status().is_success() {
                    return Err(GraphQLError::Http(response.error_for_status().unwrap_err()));
                }
                let parsed: Resp = response.json().await?;
                Ok(parsed)
            }
            .await;

            match attempt_result {
                Ok(value) => return Ok(value),
                Err(err) if attempt < max_attempts && Self::is_retryable(&err) => {
                    debug!(attempt, error = %err, "Retrying GraphQL request");
                    tokio::time::sleep(Duration::from_millis(50 * attempt as u64)).await;
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// Execute a subscription.
    pub(crate) async fn execute_subscription(
        &self,
        request: GraphQLRequest,
        extra_headers: Vec<(String, String)>,
    ) -> Result<SubscriptionStream<Value>> {
        let ws_endpoint =
            self.config.ws_endpoint.as_ref().ok_or_else(|| {
                GraphQLError::Config("WebSocket endpoint not configured".to_string())
            })?;

        info!(endpoint = %ws_endpoint, "Starting GraphQL subscription");

        // Build the WS handshake request, applying default and per-subscription
        // headers (most importantly any auth header) so they actually reach the
        // server on the upgrade request instead of being silently dropped.
        let uri: http::Uri = ws_endpoint
            .parse()
            .map_err(|e: http::uri::InvalidUri| GraphQLError::InvalidUrl(e.to_string()))?;

        let mut builder = tokio_tungstenite::tungstenite::client::ClientRequestBuilder::new(uri);
        for (name, value) in &self.config.default_headers {
            builder = builder.with_header(name.clone(), value.clone());
        }
        for (name, value) in extra_headers {
            builder = builder.with_header(name, value);
        }

        // Connect to WebSocket
        let (ws_stream, _) = tokio_tungstenite::connect_async(builder)
            .await
            .map_err(|e| GraphQLError::WebSocket(e.to_string()))?;

        let (mut write, mut read) = ws_stream.split();

        // Send connection init
        let init_msg = serde_json::to_string(&ClientMessage::ConnectionInit { payload: None })
            .map_err(GraphQLError::Json)?;

        use futures::SinkExt;
        use tokio_tungstenite::tungstenite::Message;

        write
            .send(Message::Text(init_msg.into()))
            .await
            .map_err(|e| GraphQLError::WebSocket(e.to_string()))?;

        // Wait for connection_ack
        if let Some(msg) = read.next().await {
            let msg = msg.map_err(|e| GraphQLError::WebSocket(e.to_string()))?;
            if let Message::Text(text) = msg {
                let server_msg: ServerMessage = serde_json::from_str(&text)?;
                match server_msg {
                    ServerMessage::ConnectionAck { .. } => {
                        debug!("WebSocket connection acknowledged");
                    }
                    _ => {
                        return Err(GraphQLError::WebSocket(
                            "Expected connection_ack".to_string(),
                        ));
                    }
                }
            }
        }

        // Send subscribe message
        let subscription_id = format!(
            "{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        let subscribe_msg = serde_json::to_string(&ClientMessage::Subscribe {
            id: subscription_id.clone(),
            payload: SubscribePayload {
                query: request.query,
                operation_name: request.operation_name,
                variables: request.variables,
                extensions: request.extensions,
            },
        })?;

        write
            .send(Message::Text(subscribe_msg.into()))
            .await
            .map_err(|e| GraphQLError::WebSocket(e.to_string()))?;

        // Create the stream
        let stream = futures::stream::unfold((read, write), |(mut read, mut write)| async move {
            loop {
                match read.next().await {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ServerMessage>(&text) {
                            Ok(ServerMessage::Next { payload, .. }) => {
                                let response = GraphQLResponse {
                                    data: Some(payload),
                                    errors: None,
                                    extensions: None,
                                };
                                return Some((Ok(response), (read, write)));
                            }
                            Ok(ServerMessage::Error { payload, .. }) => {
                                let errors: Vec<crate::GraphQLResponseError> = payload
                                    .into_iter()
                                    .filter_map(|v| serde_json::from_value(v).ok())
                                    .collect();
                                return Some((Err(GraphQLError::GraphQL(errors)), (read, write)));
                            }
                            Ok(ServerMessage::Complete { .. }) => {
                                return None;
                            }
                            Ok(ServerMessage::Ping { payload }) => {
                                // Reply with a pong so keep-alive-based servers don't
                                // terminate the subscription for lack of a response.
                                let pong = match serde_json::to_string(&ClientMessage::Pong {
                                    payload,
                                }) {
                                    Ok(pong) => pong,
                                    Err(e) => {
                                        return Some((Err(GraphQLError::Json(e)), (read, write)));
                                    }
                                };
                                if let Err(e) = write.send(Message::Text(pong.into())).await {
                                    return Some((
                                        Err(GraphQLError::WebSocket(e.to_string())),
                                        (read, write),
                                    ));
                                }
                                continue;
                            }
                            Ok(_) => continue,
                            Err(e) => {
                                return Some((Err(GraphQLError::Json(e)), (read, write)));
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        return None;
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => {
                        return Some((Err(GraphQLError::WebSocket(e.to_string())), (read, write)));
                    }
                    None => return None,
                }
            }
        });

        Ok(SubscriptionStream::new(stream))
    }
}

impl Default for GraphQLClient {
    fn default() -> Self {
        Self::with_config(GraphQLClientConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = GraphQLClient::new("http://localhost:4000/graphql");
        assert_eq!(client.config().endpoint, "http://localhost:4000/graphql");
    }

    #[test]
    fn test_client_with_config() {
        let config = GraphQLClientConfig::builder()
            .endpoint("https://api.example.com/graphql")
            .timeout(Duration::from_secs(60))
            .bearer_auth("token123")
            .build();

        let client = GraphQLClient::with_config(config);
        assert_eq!(client.config().endpoint, "https://api.example.com/graphql");
        assert_eq!(client.config().timeout, Duration::from_secs(60));
    }
}
