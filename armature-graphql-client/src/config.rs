//! GraphQL client configuration.

use std::time::Duration;

/// GraphQL client configuration.
#[derive(Debug, Clone)]
pub struct GraphQLClientConfig {
    /// GraphQL endpoint URL.
    pub endpoint: String,
    /// WebSocket endpoint URL (for subscriptions).
    pub ws_endpoint: Option<String>,
    /// Request timeout.
    pub timeout: Duration,
    /// Default headers for all requests.
    pub default_headers: Vec<(String, String)>,
    /// Enable response caching.
    pub caching: bool,
    /// Cache TTL.
    pub cache_ttl: Duration,
    /// Maximum number of entries retained in the response cache before the
    /// least-recently-used entry is evicted to make room for a new one.
    pub max_cache_entries: usize,
    /// User agent string.
    pub user_agent: String,
    /// Retry failed requests.
    pub retry_enabled: bool,
    /// Maximum retry attempts.
    pub max_retries: u32,
}

impl Default for GraphQLClientConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:4000/graphql".to_string(),
            ws_endpoint: None,
            timeout: Duration::from_secs(30),
            default_headers: Vec::new(),
            caching: false,
            cache_ttl: Duration::from_secs(300),
            max_cache_entries: 1000,
            user_agent: format!("armature-graphql-client/{}", env!("CARGO_PKG_VERSION")),
            retry_enabled: true,
            max_retries: 3,
        }
    }
}

impl GraphQLClientConfig {
    /// Create a new configuration builder.
    pub fn builder() -> GraphQLClientConfigBuilder {
        GraphQLClientConfigBuilder::default()
    }

    /// Create configuration for a specific endpoint.
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            ..Default::default()
        }
    }
}

/// Builder for GraphQL client configuration.
#[derive(Debug, Default)]
pub struct GraphQLClientConfigBuilder {
    config: GraphQLClientConfig,
}

impl GraphQLClientConfigBuilder {
    /// Set the GraphQL endpoint URL.
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.config.endpoint = endpoint.into();
        self
    }

    /// Set the WebSocket endpoint for subscriptions.
    pub fn ws_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.config.ws_endpoint = Some(endpoint.into());
        self
    }

    /// Set the request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Add a default header.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.config
            .default_headers
            .push((name.into(), value.into()));
        self
    }

    /// Set bearer authentication.
    pub fn bearer_auth(mut self, token: impl Into<String>) -> Self {
        self.config.default_headers.push((
            "Authorization".to_string(),
            format!("Bearer {}", token.into()),
        ));
        self
    }

    /// Enable response caching.
    pub fn caching(mut self, enabled: bool) -> Self {
        self.config.caching = enabled;
        self
    }

    /// Set cache TTL.
    pub fn cache_ttl(mut self, ttl: Duration) -> Self {
        self.config.cache_ttl = ttl;
        self
    }

    /// Set the maximum number of entries retained in the response cache
    /// before the least-recently-used entry is evicted. Must be greater than
    /// zero; a value of `0` is coerced to `1`.
    pub fn max_cache_entries(mut self, max_entries: usize) -> Self {
        self.config.max_cache_entries = max_entries.max(1);
        self
    }

    /// Set user agent string.
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.config.user_agent = user_agent.into();
        self
    }

    /// Enable or disable retry.
    pub fn retry(mut self, enabled: bool) -> Self {
        self.config.retry_enabled = enabled;
        self
    }

    /// Set maximum retry attempts.
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.config.max_retries = retries;
        self
    }

    /// Build the configuration.
    pub fn build(self) -> GraphQLClientConfig {
        self.config
    }
}
