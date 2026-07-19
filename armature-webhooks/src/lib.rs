//! Webhook Orchestration for Armature
//!
//! This crate provides comprehensive webhook support for the Armature framework,
//! including both sending outgoing webhooks and receiving incoming webhooks.
//!
//! # Features
//!
//! - **Outgoing Webhooks**: Send HTTP callbacks to registered endpoints
//! - **Incoming Webhooks**: Receive and verify webhooks from external services
//! - **Signature Verification**: HMAC-SHA256 signing and verification
//! - **Automatic Retries**: Configurable retry policies with exponential backoff
//! - **Event System**: Subscribe endpoints to specific event types
//! - **Delivery Tracking**: Monitor webhook delivery status and history
//!
//! # Example: Sending Webhooks
//!
//! ```rust,no_run
//! use armature_webhooks::{WebhookClient, WebhookConfig, WebhookPayload};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = WebhookClient::new(WebhookConfig::default());
//!
//!     let payload = WebhookPayload::new("user.created")
//!         .with_data(serde_json::json!({
//!             "user_id": "123",
//!             "email": "user@example.com"
//!         }));
//!
//!     client.send("https://example.com/webhook", payload).await?;
//!     Ok(())
//! }
//! ```
//!
//! # Example: Receiving Webhooks
//!
//! ```rust,no_run
//! use armature_webhooks::WebhookReceiver;
//!
//! let receiver = WebhookReceiver::new("your-secret-key");
//!
//! // Verify incoming webhook
//! let payload_bytes = b"webhook payload";
//! let signature_header = "t=1234567890,v1=abc123...";
//!
//! let is_valid = receiver.verify(payload_bytes, signature_header);
//! ```
//!
//! # Example: Webhook Registry
//!
//! ```rust,no_run
//! use armature_webhooks::{WebhookRegistry, WebhookEndpoint};
//!
//! let mut registry = WebhookRegistry::new();
//!
//! // Register an endpoint for specific events
//! registry.register(WebhookEndpoint::new("https://api.example.com/webhooks")
//!     .with_events(vec!["order.created", "order.updated"])
//!     .with_secret("endpoint-secret"));
//!
//! // Get all endpoints subscribed to an event
//! let endpoints = registry.get_endpoints_for_event("order.created");
//! ```

mod client;
mod config;
mod endpoint;
mod error;
mod payload;
mod receiver;
mod registry;
mod retry;
mod signature;

pub use client::WebhookClient;
pub use config::{SigningAlgorithm, WebhookConfig, WebhookConfigBuilder};
pub use endpoint::{WebhookEndpoint, WebhookEndpointBuilder};
pub use error::WebhookError;
pub use payload::{WebhookDelivery, WebhookDeliveryStatus, WebhookPayload};
pub use receiver::WebhookReceiver;
pub use registry::WebhookRegistry;
pub use retry::{RetryPolicy, RetryResult};
pub use signature::WebhookSignature;

/// Result type for webhook operations
pub type Result<T> = std::result::Result<T, WebhookError>;
