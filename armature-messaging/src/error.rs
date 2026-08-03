//! Error types for messaging operations

use thiserror::Error;

/// Errors that can occur during messaging operations
#[derive(Error, Debug)]
pub enum MessagingError {
    /// Failed to connect to the broker
    #[error("Connection failed: {0}")]
    Connection(String),

    /// Failed to publish a message
    #[error("Publish failed: {0}")]
    Publish(String),

    /// Failed to subscribe to a topic/queue
    #[error("Subscribe failed: {0}")]
    Subscribe(String),

    /// Failed to acknowledge a message
    #[error("Acknowledge failed: {0}")]
    Acknowledge(String),

    /// Failed to serialize a message
    #[error("Serialization failed: {0}")]
    Serialization(String),

    /// Failed to deserialize a message
    #[error("Deserialization failed: {0}")]
    Deserialization(String),

    /// Operation timed out
    #[error("Operation timed out: {0}")]
    Timeout(String),

    /// Authentication failed
    #[error("Authentication failed: {0}")]
    Authentication(String),

    /// Authorization failed
    #[error("Authorization failed: {0}")]
    Authorization(String),

    /// Invalid configuration
    #[error("Invalid configuration: {0}")]
    Configuration(String),

    /// Channel/connection is closed
    #[error("Channel closed: {0}")]
    ChannelClosed(String),

    /// Queue/topic not found
    #[error("Queue/topic not found: {0}")]
    NotFound(String),

    /// Queue/topic already exists
    #[error("Queue/topic already exists: {0}")]
    AlreadyExists(String),

    /// Resource exhausted (e.g., too many connections)
    #[error("Resource exhausted: {0}")]
    ResourceExhausted(String),

    /// Message was rejected by the broker
    #[error("Message rejected: {0}")]
    Rejected(String),

    /// Message expired (TTL exceeded)
    #[error("Message expired: {0}")]
    Expired(String),

    /// A requested option has no equivalent on the selected backend.
    ///
    /// Returned instead of silently downgrading the request. Several
    /// [`SubscribeOptions`](crate::SubscribeOptions) fields are meaningful on
    /// only some brokers; quietly aliasing them to the nearest supported
    /// behavior would leave a caller believing they narrowed a subscription or
    /// took manual control of acknowledgment when they did not.
    #[error("Unsupported on this backend: {0}")]
    Unsupported(String),

    /// Internal broker error
    #[error("Broker error: {0}")]
    BrokerError(String),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Other errors
    #[error("{0}")]
    Other(String),
}

impl MessagingError {
    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            MessagingError::Connection(_)
                | MessagingError::Timeout(_)
                | MessagingError::ChannelClosed(_)
                | MessagingError::ResourceExhausted(_)
                | MessagingError::BrokerError(_)
        )
    }

    /// Check if this error indicates a connection issue
    pub fn is_connection_error(&self) -> bool {
        matches!(
            self,
            MessagingError::Connection(_)
                | MessagingError::ChannelClosed(_)
                | MessagingError::Authentication(_)
        )
    }
}

#[cfg(feature = "rabbitmq")]
impl From<lapin::Error> for MessagingError {
    fn from(err: lapin::Error) -> Self {
        use lapin::ErrorKind;
        match err.kind() {
            ErrorKind::IOError(_) | ErrorKind::RuntimeShutdownError(_) => {
                MessagingError::Connection(err.to_string())
            }
            ErrorKind::ChannelsLimitReached => MessagingError::ResourceExhausted(err.to_string()),
            ErrorKind::InvalidChannel(_) | ErrorKind::InvalidChannelState(..) => {
                MessagingError::ChannelClosed(err.to_string())
            }
            ErrorKind::InvalidConnectionState(_) => MessagingError::Connection(err.to_string()),
            _ => MessagingError::BrokerError(err.to_string()),
        }
    }
}

#[cfg(feature = "kafka")]
impl From<rdkafka::error::KafkaError> for MessagingError {
    fn from(err: rdkafka::error::KafkaError) -> Self {
        match &err {
            rdkafka::error::KafkaError::MessageProduction(_) => {
                MessagingError::Publish(err.to_string())
            }
            rdkafka::error::KafkaError::MessageConsumption(_) => {
                MessagingError::Subscribe(err.to_string())
            }
            rdkafka::error::KafkaError::ClientCreation(_) => {
                MessagingError::Connection(err.to_string())
            }
            _ => MessagingError::BrokerError(err.to_string()),
        }
    }
}

#[cfg(feature = "nats")]
impl From<async_nats::ConnectError> for MessagingError {
    fn from(err: async_nats::ConnectError) -> Self {
        MessagingError::Connection(err.to_string())
    }
}

#[cfg(feature = "nats")]
impl From<async_nats::PublishError> for MessagingError {
    fn from(err: async_nats::PublishError) -> Self {
        MessagingError::Publish(err.to_string())
    }
}

#[cfg(feature = "nats")]
impl From<async_nats::SubscribeError> for MessagingError {
    fn from(err: async_nats::SubscribeError) -> Self {
        MessagingError::Subscribe(err.to_string())
    }
}
