//! AWS SQS/SNS message broker implementation

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use aws_sdk_sns::Client as SnsClient;
use aws_sdk_sqs::Client as SqsClient;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::{
    AckMode, Message, MessageBroker, MessageHandler, MessagingError, ProcessingResult,
    PublishOptions, SubscribeOptions, Subscription, config::AwsConfig,
};

/// AWS SQS/SNS message broker
pub struct AwsBroker {
    sqs_client: SqsClient,
    sns_client: SnsClient,
    config: AwsConfig,
    active_consumers: Arc<RwLock<Vec<Arc<AtomicBool>>>>,
    connected: Arc<AtomicBool>,
}

impl AwsBroker {
    /// Create a new AWS broker
    pub async fn new(config: AwsConfig) -> Result<Self, MessagingError> {
        info!(region = %config.region, "Connecting to AWS SQS/SNS");

        let mut aws_config_builder = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_sdk_sqs::config::Region::new(config.region.clone()));

        // Custom endpoint for LocalStack
        if let Some(ref endpoint) = config.endpoint_url {
            aws_config_builder = aws_config_builder.endpoint_url(endpoint);
        }

        // Explicit credentials if provided
        if let (Some(access_key), Some(secret_key)) =
            (&config.access_key_id, &config.secret_access_key)
        {
            let credentials = aws_sdk_sqs::config::Credentials::new(
                access_key,
                secret_key,
                config.session_token.clone(),
                None,
                "armature",
            );
            aws_config_builder = aws_config_builder.credentials_provider(credentials);
        }

        let aws_config = aws_config_builder.load().await;

        let sqs_client = SqsClient::new(&aws_config);
        let sns_client = SnsClient::new(&aws_config);

        info!("Connected to AWS successfully");

        Ok(Self {
            sqs_client,
            sns_client,
            config,
            active_consumers: Arc::new(RwLock::new(Vec::new())),
            connected: Arc::new(AtomicBool::new(true)),
        })
    }

    /// Create a broker configured for LocalStack
    pub async fn localstack() -> Result<Self, MessagingError> {
        Self::new(AwsConfig::localstack()).await
    }

    /// Get the SQS queue URL for a queue name
    pub async fn get_queue_url(&self, queue_name: &str) -> Result<String, MessagingError> {
        // Check if it's already a URL
        if queue_name.starts_with("http") {
            return Ok(queue_name.to_string());
        }

        // Check for prefix
        if let Some(ref prefix) = self.config.sqs_queue_url_prefix {
            return Ok(format!("{}/{}", prefix, queue_name));
        }

        // Get URL from AWS
        let result = self
            .sqs_client
            .get_queue_url()
            .queue_name(queue_name)
            .send()
            .await
            .map_err(|e| MessagingError::NotFound(format!("Queue not found: {}", e)))?;

        result
            .queue_url
            .ok_or_else(|| MessagingError::NotFound(format!("Queue URL not found: {}", queue_name)))
    }

    /// Get the SNS topic ARN for a topic name
    pub fn get_topic_arn(&self, topic_name: &str) -> String {
        // Check if it's already an ARN
        if topic_name.starts_with("arn:aws:sns") {
            return topic_name.to_string();
        }

        // Check for prefix
        if let Some(ref prefix) = self.config.sns_topic_arn_prefix {
            return format!("{}:{}", prefix, topic_name);
        }

        // Build ARN
        format!(
            "arn:aws:sns:{}:000000000000:{}",
            self.config.region, topic_name
        )
    }

    /// Create an SQS queue
    pub async fn create_queue(&self, name: &str) -> Result<String, MessagingError> {
        let result = self
            .sqs_client
            .create_queue()
            .queue_name(name)
            .send()
            .await
            .map_err(|e| MessagingError::BrokerError(format!("Failed to create queue: {}", e)))?;

        result
            .queue_url
            .ok_or_else(|| MessagingError::BrokerError("No queue URL returned".to_string()))
    }

    /// Create an SNS topic
    pub async fn create_topic(&self, name: &str) -> Result<String, MessagingError> {
        let result = self
            .sns_client
            .create_topic()
            .name(name)
            .send()
            .await
            .map_err(|e| MessagingError::BrokerError(format!("Failed to create topic: {}", e)))?;

        result
            .topic_arn
            .ok_or_else(|| MessagingError::BrokerError("No topic ARN returned".to_string()))
    }

    /// Subscribe an SQS queue to an SNS topic
    pub async fn subscribe_queue_to_topic(
        &self,
        queue_arn: &str,
        topic_arn: &str,
    ) -> Result<String, MessagingError> {
        let result = self
            .sns_client
            .subscribe()
            .topic_arn(topic_arn)
            .protocol("sqs")
            .endpoint(queue_arn)
            .send()
            .await
            .map_err(|e| {
                MessagingError::BrokerError(format!("Failed to subscribe queue to topic: {}", e))
            })?;

        result
            .subscription_arn
            .ok_or_else(|| MessagingError::BrokerError("No subscription ARN returned".to_string()))
    }

    fn build_message_attributes(
        message: &Message,
    ) -> HashMap<String, aws_sdk_sqs::types::MessageAttributeValue> {
        let mut attrs = HashMap::new();

        attrs.insert(
            "message_id".to_string(),
            aws_sdk_sqs::types::MessageAttributeValue::builder()
                .data_type("String")
                .string_value(&message.id)
                .build()
                .unwrap(),
        );

        attrs.insert(
            "timestamp".to_string(),
            aws_sdk_sqs::types::MessageAttributeValue::builder()
                .data_type("String")
                .string_value(message.timestamp.to_rfc3339())
                .build()
                .unwrap(),
        );

        if let Some(ref correlation_id) = message.correlation_id {
            attrs.insert(
                "correlation_id".to_string(),
                aws_sdk_sqs::types::MessageAttributeValue::builder()
                    .data_type("String")
                    .string_value(correlation_id)
                    .build()
                    .unwrap(),
            );
        }

        if let Some(ref content_type) = message.content_type {
            attrs.insert(
                "content_type".to_string(),
                aws_sdk_sqs::types::MessageAttributeValue::builder()
                    .data_type("String")
                    .string_value(content_type)
                    .build()
                    .unwrap(),
            );
        }

        for (key, value) in &message.headers {
            attrs.insert(
                key.clone(),
                aws_sdk_sqs::types::MessageAttributeValue::builder()
                    .data_type("String")
                    .string_value(value)
                    .build()
                    .unwrap(),
            );
        }

        attrs
    }
}

#[async_trait]
impl MessageBroker for AwsBroker {
    type Subscription = AwsSubscription;

    async fn publish(&self, message: Message) -> Result<(), MessagingError> {
        self.publish_with_options(message, PublishOptions::default())
            .await
    }

    async fn publish_with_options(
        &self,
        message: Message,
        options: PublishOptions,
    ) -> Result<(), MessagingError> {
        let body = String::from_utf8_lossy(&message.payload).to_string();

        // Determine if publishing to SNS topic or SQS queue
        if message.topic.starts_with("arn:aws:sns") || options.exchange.is_some() {
            // Publish to SNS
            let topic_arn = options
                .exchange
                .as_ref()
                .map(|e| self.get_topic_arn(e))
                .unwrap_or_else(|| message.topic.clone());

            debug!(topic_arn = %topic_arn, message_id = %message.id, "Publishing to SNS");

            let mut request = self
                .sns_client
                .publish()
                .topic_arn(&topic_arn)
                .message(&body);

            // Add message attributes
            for (key, value) in &message.headers {
                request = request.message_attributes(
                    key,
                    aws_sdk_sns::types::MessageAttributeValue::builder()
                        .data_type("String")
                        .string_value(value)
                        .build()
                        .unwrap(),
                );
            }

            request
                .send()
                .await
                .map_err(|e| MessagingError::Publish(format!("Failed to publish to SNS: {}", e)))?;
        } else {
            // Publish to SQS
            let queue_url = self.get_queue_url(&message.topic).await?;

            debug!(queue_url = %queue_url, message_id = %message.id, "Publishing to SQS");

            let mut request = self
                .sqs_client
                .send_message()
                .queue_url(&queue_url)
                .message_body(&body);

            // Add message attributes
            let attrs = Self::build_message_attributes(&message);
            for (key, value) in attrs {
                request = request.message_attributes(key, value);
            }

            // Add delay if TTL is set (using delay seconds)
            if let Some(ttl) = message.ttl {
                let delay_seconds = (ttl / 1000).min(900) as i32; // Max 15 minutes
                request = request.delay_seconds(delay_seconds);
            }

            request
                .send()
                .await
                .map_err(|e| MessagingError::Publish(format!("Failed to publish to SQS: {}", e)))?;
        }

        Ok(())
    }

    async fn subscribe(
        &self,
        topic: &str,
        handler: Arc<dyn MessageHandler>,
    ) -> Result<Self::Subscription, MessagingError> {
        self.subscribe_with_options(topic, handler, SubscribeOptions::default())
            .await
    }

    /// Subscribe to an SQS queue.
    ///
    /// Only [`SubscribeOptions::ack_mode`] is honored: `AckMode::None` leaves
    /// received messages in the queue (they become visible again after the
    /// visibility timeout), while `Auto`/`Manual` delete a message once the
    /// handler reports success. The remaining options are not applicable to
    /// SQS and are ignored: `prefetch_count`/`from_beginning`/`filter` have no
    /// SQS equivalent (batch size and long-poll behavior come from
    /// [`AwsConfig`]), and `concurrency` is not yet implemented (messages are
    /// dispatched sequentially).
    async fn subscribe_with_options(
        &self,
        topic: &str,
        handler: Arc<dyn MessageHandler>,
        options: SubscribeOptions,
    ) -> Result<Self::Subscription, MessagingError> {
        if let Some(concurrency) = options.concurrency
            && concurrency > 1
        {
            warn!(
                concurrency,
                "SubscribeOptions::concurrency is not implemented for SQS; messages are dispatched sequentially"
            );
        }

        let queue_url = self.get_queue_url(topic).await?;
        let active = Arc::new(AtomicBool::new(true));
        let ack_mode = options.ack_mode;

        let subscription = AwsSubscription {
            queue_url: queue_url.clone(),
            active: active.clone(),
        };

        // Store active flag for cleanup, pruning any flags whose consumer task
        // has already stopped so the vec does not grow unbounded.
        {
            let mut consumers = self.active_consumers.write().await;
            consumers.retain(|flag| flag.load(Ordering::SeqCst));
            consumers.push(active.clone());
        }

        // Spawn consumer task
        let sqs_client = self.sqs_client.clone();
        let config = self.config.clone();
        let topic_owned = topic.to_string();

        tokio::spawn(async move {
            poll_messages(
                sqs_client,
                queue_url,
                handler,
                config,
                &topic_owned,
                ack_mode,
                active,
            )
            .await;
        });

        info!(queue = topic, "Subscribed to SQS queue");
        Ok(subscription)
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn close(&self) -> Result<(), MessagingError> {
        info!("Closing AWS connections");
        self.connected.store(false, Ordering::SeqCst);

        // Stop all consumers
        let consumers = self.active_consumers.read().await;
        for active in consumers.iter() {
            active.store(false, Ordering::SeqCst);
        }

        Ok(())
    }
}

async fn poll_messages(
    client: SqsClient,
    queue_url: String,
    handler: Arc<dyn MessageHandler>,
    config: AwsConfig,
    topic: &str,
    ack_mode: AckMode,
    active: Arc<AtomicBool>,
) {
    while active.load(Ordering::SeqCst) {
        let result = client
            .receive_message()
            .queue_url(&queue_url)
            .max_number_of_messages(config.max_number_of_messages)
            .wait_time_seconds(config.long_poll_wait_seconds)
            .visibility_timeout(config.visibility_timeout)
            .message_attribute_names("All")
            .send()
            .await;

        match result {
            Ok(output) => {
                if let Some(messages) = output.messages {
                    for sqs_message in messages {
                        let message = sqs_message_to_message(&sqs_message, topic);
                        let receipt_handle = sqs_message.receipt_handle.clone();

                        match handler.handle(message).await {
                            Ok(result) => match result {
                                ProcessingResult::Success => {
                                    // Delete the message to acknowledge it. In
                                    // `AckMode::None` no acknowledgment is
                                    // performed, so the message is left in the
                                    // queue and becomes visible again after the
                                    // visibility timeout (at-least-once redelivery).
                                    if ack_mode != AckMode::None
                                        && let Some(handle) = receipt_handle
                                        && let Err(e) = client
                                            .delete_message()
                                            .queue_url(&queue_url)
                                            .receipt_handle(&handle)
                                            .send()
                                            .await
                                    {
                                        error!(error = %e, "Failed to delete message");
                                    }
                                }
                                ProcessingResult::Retry => {
                                    // Change visibility timeout to make it available again
                                    if let Some(handle) = receipt_handle
                                        && let Err(e) = client
                                            .change_message_visibility()
                                            .queue_url(&queue_url)
                                            .receipt_handle(&handle)
                                            .visibility_timeout(0)
                                            .send()
                                            .await
                                    {
                                        warn!(error = %e, "Failed to change message visibility");
                                    }
                                }
                                ProcessingResult::DeadLetter | ProcessingResult::Reject => {
                                    // Delete the message (it should go to DLQ if configured)
                                    if let Some(handle) = receipt_handle
                                        && let Err(e) = client
                                            .delete_message()
                                            .queue_url(&queue_url)
                                            .receipt_handle(&handle)
                                            .send()
                                            .await
                                    {
                                        error!(error = %e, "Failed to delete rejected message");
                                    }
                                }
                            },
                            Err(e) => {
                                error!(error = %e, "Message handler error");
                                // Message will become visible again after visibility timeout
                            }
                        }
                    }
                }
            }
            Err(e) => {
                error!(error = %e, "Failed to receive messages");
                // Wait before retrying
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
}

fn sqs_message_to_message(sqs_msg: &aws_sdk_sqs::types::Message, topic: &str) -> Message {
    let payload = sqs_msg
        .body
        .as_ref()
        .map(|b| b.as_bytes().to_vec())
        .unwrap_or_default();

    let mut headers = HashMap::new();
    let mut message_id = None;
    let mut correlation_id = None;
    let mut content_type = None;
    let mut timestamp = chrono::Utc::now();

    if let Some(attrs) = &sqs_msg.message_attributes {
        for (key, value) in attrs {
            if let Some(string_value) = &value.string_value {
                match key.as_str() {
                    "message_id" => message_id = Some(string_value.clone()),
                    "correlation_id" => correlation_id = Some(string_value.clone()),
                    "content_type" => content_type = Some(string_value.clone()),
                    "timestamp" => {
                        if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(string_value) {
                            timestamp = ts.with_timezone(&chrono::Utc);
                        }
                    }
                    _ => {
                        headers.insert(key.clone(), string_value.clone());
                    }
                }
            }
        }
    }

    // Use SQS message ID if not provided in attributes
    if message_id.is_none() {
        message_id = sqs_msg.message_id.clone();
    }

    Message {
        id: message_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        payload,
        headers,
        topic: topic.to_string(),
        timestamp,
        correlation_id,
        reply_to: None,
        content_type,
        priority: None,
        ttl: None,
    }
}

/// AWS SQS subscription handle
pub struct AwsSubscription {
    queue_url: String,
    active: Arc<AtomicBool>,
}

#[async_trait]
impl Subscription for AwsSubscription {
    async fn unsubscribe(&self) -> Result<(), MessagingError> {
        self.active.store(false, Ordering::SeqCst);
        info!(queue_url = %self.queue_url, "Unsubscribed from SQS queue");
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    fn topic(&self) -> &str {
        &self.queue_url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aws_config() {
        let config = AwsConfig::new("us-west-2")
            .with_credentials("access_key", "secret_key")
            .with_endpoint("http://localhost:4566");

        assert_eq!(config.region, "us-west-2");
        assert_eq!(config.access_key_id, Some("access_key".to_string()));
        assert_eq!(
            config.endpoint_url,
            Some("http://localhost:4566".to_string())
        );
    }

    #[test]
    fn test_localstack_config() {
        let config = AwsConfig::localstack();

        assert_eq!(config.region, "us-east-1");
        assert_eq!(
            config.endpoint_url,
            Some("http://localhost:4566".to_string())
        );
        assert_eq!(config.access_key_id, Some("test".to_string()));
    }
}
