// Server-Sent Events (SSE) support for Armature

use crate::Error;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

/// Server-Sent Event
#[derive(Debug, Clone)]
pub struct ServerSentEvent {
    /// Event ID (optional)
    pub id: Option<String>,
    /// Event type (optional)
    pub event: Option<String>,
    /// Event data
    pub data: String,
    /// Retry interval in milliseconds (optional)
    pub retry: Option<u64>,
}

impl ServerSentEvent {
    /// Create a new SSE with just data
    pub fn new(data: String) -> Self {
        Self {
            id: None,
            event: None,
            data,
            retry: None,
        }
    }

    /// Create a new SSE with data and event type
    pub fn with_event(event: String, data: String) -> Self {
        Self {
            id: None,
            event: Some(event),
            data,
            retry: None,
        }
    }

    /// Create a new SSE with all fields
    pub fn full(id: String, event: String, data: String, retry: u64) -> Self {
        Self {
            id: Some(id),
            event: Some(event),
            data,
            retry: Some(retry),
        }
    }

    /// Convert to SSE format string
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        let mut output = String::new();

        if let Some(ref id) = self.id {
            output.push_str(&format!("id: {}\n", id));
        }

        if let Some(ref event) = self.event {
            output.push_str(&format!("event: {}\n", event));
        }

        // Handle multi-line data
        for line in self.data.lines() {
            output.push_str(&format!("data: {}\n", line));
        }

        if let Some(retry) = self.retry {
            output.push_str(&format!("retry: {}\n", retry));
        }

        output.push('\n');
        output
    }
}

/// SSE stream builder
pub struct SseStream {
    tx: mpsc::Sender<Result<String, Error>>,
}

impl SseStream {
    /// Create a new SSE stream
    pub fn new() -> (Self, ReceiverStream<Result<String, Error>>) {
        let (tx, rx) = mpsc::channel(100);
        let stream = ReceiverStream::new(rx);
        (Self { tx }, stream)
    }

    /// Send an event
    pub async fn send(&self, event: ServerSentEvent) -> Result<(), Error> {
        self.tx
            .send(Ok(event.to_string()))
            .await
            .map_err(|e| Error::Internal(format!("Failed to send SSE: {}", e)))
    }

    /// Send a simple message
    pub async fn send_message(&self, data: String) -> Result<(), Error> {
        self.send(ServerSentEvent::new(data)).await
    }

    /// Send a typed event
    pub async fn send_event(&self, event: String, data: String) -> Result<(), Error> {
        self.send(ServerSentEvent::with_event(event, data)).await
    }

    /// Send JSON data
    pub async fn send_json<T: serde::Serialize>(&self, data: &T) -> Result<(), Error> {
        let json = serde_json::to_string(data).map_err(|e| Error::Serialization(e.to_string()))?;
        self.send_message(json).await
    }

    /// Send a keep-alive comment
    pub async fn send_keep_alive(&self) -> Result<(), Error> {
        self.tx
            .send(Ok(": keep-alive\n\n".to_string()))
            .await
            .map_err(|e| Error::Internal(format!("Failed to send keep-alive: {}", e)))
    }
}

impl Default for SseStream {
    fn default() -> Self {
        Self::new().0
    }
}

/// SSE broadcaster for multiple clients
pub struct SseBroadcaster {
    clients: tokio::sync::RwLock<Vec<mpsc::Sender<Result<String, Error>>>>,
}

impl SseBroadcaster {
    pub fn new() -> Self {
        Self {
            clients: tokio::sync::RwLock::new(Vec::new()),
        }
    }

    /// Register a new client
    ///
    /// Prunes any senders whose receiver has been dropped before pushing the
    /// new client, so an application that registers frequently but rarely
    /// broadcasts does not accumulate dead senders (they would otherwise only
    /// be reaped during [`broadcast`](Self::broadcast) or keep-alive).
    pub async fn register(&self) -> ReceiverStream<Result<String, Error>> {
        let (tx, rx) = mpsc::channel(100);
        let mut clients = self.clients.write().await;
        clients.retain(|tx| !tx.is_closed());
        clients.push(tx);
        ReceiverStream::new(rx)
    }

    /// Broadcast an event to all clients
    ///
    /// Uses non-blocking sends so the client list lock is never held across
    /// an `.await`. Slow clients whose channel buffer is full are
    /// disconnected (dropped from the list) so a single stalled client
    /// cannot block broadcasts or new registrations.
    pub async fn broadcast(&self, event: ServerSentEvent) -> Result<(), Error> {
        let data_str = event.to_string();
        let mut clients = self.clients.write().await;

        // Send to all clients, removing those that are disconnected or
        // whose channel is full (slow consumers).
        clients.retain(|tx| tx.try_send(Ok(data_str.clone())).is_ok());

        Ok(())
    }

    /// Broadcast a simple message
    pub async fn broadcast_message(&self, data: String) -> Result<(), Error> {
        self.broadcast(ServerSentEvent::new(data)).await
    }

    /// Broadcast JSON data
    pub async fn broadcast_json<T: serde::Serialize>(&self, data: &T) -> Result<(), Error> {
        let json = serde_json::to_string(data).map_err(|e| Error::Serialization(e.to_string()))?;
        self.broadcast_message(json).await
    }

    /// Get number of connected clients
    pub async fn client_count(&self) -> usize {
        let clients = self.clients.read().await;
        clients.len()
    }

    /// Start a keep-alive task
    ///
    /// Like [`broadcast`](Self::broadcast), keep-alives use non-blocking
    /// sends: clients that are disconnected or whose channel is full are
    /// removed so a stalled client cannot deadlock the broadcaster.
    pub fn start_keep_alive(
        self: std::sync::Arc<Self>,
        interval: Duration,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);
            loop {
                interval_timer.tick().await;
                let comment_str = ": keep-alive\n\n".to_string();
                let mut clients = self.clients.write().await;
                clients.retain(|tx| tx.try_send(Ok(comment_str.clone())).is_ok());
            }
        })
    }
}

impl Default for SseBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_broadcast_does_not_block_on_stalled_client() {
        let broadcaster = SseBroadcaster::new();

        // Register a client that never consumes its stream.
        let _stalled_rx = broadcaster.register().await;

        // Fill the stalled client's bounded channel (capacity 100) and keep
        // going; before the fix this deadlocked on a full channel.
        for i in 0..150 {
            let result = tokio::time::timeout(
                Duration::from_secs(5),
                broadcaster.broadcast_message(format!("event {}", i)),
            )
            .await;
            assert!(result.is_ok(), "broadcast deadlocked on a stalled client");
        }

        // The stalled client should have been disconnected once its channel
        // filled.
        assert_eq!(broadcaster.client_count().await, 0);
    }

    #[tokio::test]
    async fn test_register_prunes_closed_senders() {
        let broadcaster = SseBroadcaster::new();

        // Register three clients, then drop two of their receivers.
        let rx1 = broadcaster.register().await;
        let rx2 = broadcaster.register().await;
        let keep = broadcaster.register().await;
        assert_eq!(broadcaster.client_count().await, 3);

        drop(rx1);
        drop(rx2);

        // Registering a new client should reap the two dead senders first, so
        // we end with the live `keep` client, the newly registered one, and no
        // dead senders left behind.
        let _new = broadcaster.register().await;
        assert_eq!(broadcaster.client_count().await, 2);

        drop(keep);
    }

    #[tokio::test]
    async fn test_broadcast_reaches_active_client() {
        use tokio_stream::StreamExt;

        let broadcaster = SseBroadcaster::new();
        let mut rx = broadcaster.register().await;

        broadcaster
            .broadcast_message("hello".to_string())
            .await
            .unwrap();

        let received = rx.next().await.unwrap().unwrap();
        assert!(received.contains("data: hello"));
    }
}
