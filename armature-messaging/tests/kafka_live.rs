//! Live-broker integration tests for the Kafka backend.
//!
//! These are `#[ignore]`d because they require a real Kafka broker (there is
//! no Kafka broker available in CI for this crate). Run manually against a
//! local broker, e.g.:
//!
//! ```sh
//! docker run -d --name kafka -p 9092:9092 apache/kafka:latest
//! KAFKA_BROKERS=localhost:9092 cargo test -p armature-messaging --features kafka \
//!     --test kafka_live -- --ignored
//! ```

#![cfg(feature = "kafka")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use armature_messaging::config::KafkaConfig;
use armature_messaging::kafka::KafkaBroker;
use armature_messaging::{
    AckMode, FnHandler, Message, MessageBroker, MessagingError, ProcessingResult, SubscribeOptions,
    Subscription,
};

fn brokers() -> String {
    std::env::var("KAFKA_BROKERS").unwrap_or_else(|_| "localhost:9092".to_string())
}

/// Regression test for the finding that `AckMode::Manual` disabled
/// `enable.auto.commit` but the consume loop never called
/// `commit_message`/`store_offset`, so offsets were never committed and
/// every restart redelivered everything. This publishes a message, consumes
/// it once with `AckMode::Manual`, and confirms that a second consumer in
/// the same group does *not* see it again (i.e. the offset was committed).
#[tokio::test]
#[ignore = "requires a live Kafka broker; set KAFKA_BROKERS to point at one"]
async fn manual_ack_commits_offset_so_restart_does_not_redeliver() {
    let topic = format!("armature-manual-ack-test-{}", uuid::Uuid::new_v4());
    let group_id = format!("armature-manual-ack-group-{}", uuid::Uuid::new_v4());

    let producer_config = KafkaConfig::new(brokers()).from_earliest();
    let broker = KafkaBroker::connect_with_config(producer_config)
        .await
        .expect("connect to Kafka");

    broker
        .publish(Message::new(topic.clone(), b"hello".to_vec()))
        .await
        .expect("publish message");

    let received = Arc::new(AtomicUsize::new(0));
    let received_clone = received.clone();
    let handler = Arc::new(FnHandler(move |_msg: Message| {
        let received = received_clone.clone();
        async move {
            received.fetch_add(1, Ordering::SeqCst);
            Ok::<_, MessagingError>(ProcessingResult::Success)
        }
    }));

    let sub = broker
        .subscribe_with_options(
            &topic,
            handler,
            SubscribeOptions::default()
                .with_ack_mode(AckMode::Manual)
                .with_consumer_group(group_id.clone())
                .from_beginning(),
        )
        .await
        .expect("subscribe");

    // Give the consumer time to receive and commit the message.
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(received.load(Ordering::SeqCst), 1);
    sub.unsubscribe().await.expect("unsubscribe");

    // A brand new consumer in the *same* group should not see the message
    // again, since the offset should have been committed.
    let received2 = Arc::new(AtomicUsize::new(0));
    let received2_clone = received2.clone();
    let handler2 = Arc::new(FnHandler(move |_msg: Message| {
        let received2 = received2_clone.clone();
        async move {
            received2.fetch_add(1, Ordering::SeqCst);
            Ok::<_, MessagingError>(ProcessingResult::Success)
        }
    }));

    let sub2 = broker
        .subscribe_with_options(
            &topic,
            handler2,
            SubscribeOptions::default()
                .with_ack_mode(AckMode::Manual)
                .with_consumer_group(group_id),
        )
        .await
        .expect("re-subscribe");

    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(
        received2.load(Ordering::SeqCst),
        0,
        "message should not be redelivered after its offset was committed"
    );
    sub2.unsubscribe().await.expect("unsubscribe");
}
