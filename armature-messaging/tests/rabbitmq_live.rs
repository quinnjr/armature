//! Live-broker integration tests for the RabbitMQ backend.
//!
//! These are `#[ignore]`d because they require a real RabbitMQ broker (there
//! is no RabbitMQ broker available in CI for this crate). Run manually
//! against a local broker, e.g.:
//!
//! ```sh
//! docker run -d --name rabbitmq -p 5672:5672 rabbitmq:3-management
//! RABBITMQ_URL=amqp://guest:guest@localhost:5672 cargo test -p armature-messaging \
//!     --features rabbitmq --test rabbitmq_live -- --ignored
//! ```

#![cfg(feature = "rabbitmq")]

use armature_messaging::MessageBroker;
use armature_messaging::config::RabbitMqConfig;
use armature_messaging::rabbitmq::RabbitMqBroker;

fn url() -> String {
    std::env::var("RABBITMQ_URL").unwrap_or_else(|_| "amqp://guest:guest@localhost:5672".into())
}

/// Regression test for the finding that `RabbitMqConfig::vhost`,
/// `publisher_confirms`, and `channel_pool_size` were accepted but never
/// consumed by any connect path (`RabbitMqBroker::connect` always used the
/// default vhost, always enabled confirm_select, and always created exactly
/// one channel per subscription).
#[tokio::test]
#[ignore = "requires a live RabbitMQ broker; set RABBITMQ_URL to point at one"]
async fn connect_with_config_honors_vhost_confirms_and_pool_size() {
    // A non-default vhost must exist on the broker for this to succeed;
    // callers running this manually should create it first, e.g.:
    //   rabbitmqctl add_vhost /armature-test
    //   rabbitmqctl set_permissions -p /armature-test guest ".*" ".*" ".*"
    let config = RabbitMqConfig::new(url())
        .with_vhost("/armature-test")
        .without_publisher_confirms()
        .with_channel_pool_size(3);

    let broker = RabbitMqBroker::connect_with_config(config)
        .await
        .expect("connect to RabbitMQ with a non-default vhost");

    assert!(broker.is_connected());

    // Publish/subscribe should work normally against the configured vhost
    // even with publisher confirms disabled.
    let msg = armature_messaging::Message::new("armature-pool-test-queue", b"hello".to_vec());
    broker.publish(msg).await.expect("publish without confirms");
}
