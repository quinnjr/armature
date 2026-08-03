//! WebSocket client implementation.

use crate::error::{WebSocketError, WebSocketResult};
use crate::message::Message;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async_with_config, tungstenite::protocol::Message as TungsteniteMessage,
};
use tungstenite::protocol::WebSocketConfig;
use url::Url;

/// Builder for WebSocket client.
#[derive(Debug, Clone)]
pub struct WebSocketClientBuilder {
    url: Option<String>,
    connect_timeout: Duration,
    max_message_size: Option<usize>,
}

impl Default for WebSocketClientBuilder {
    fn default() -> Self {
        Self {
            url: None,
            connect_timeout: Duration::from_secs(30),
            max_message_size: None,
        }
    }
}

impl WebSocketClientBuilder {
    /// Create a new client builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the WebSocket URL.
    pub fn url<S: Into<String>>(mut self, url: S) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set the connection timeout.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Set the maximum message size.
    pub fn max_message_size(mut self, size: usize) -> Self {
        self.max_message_size = Some(size);
        self
    }

    /// Connect to the WebSocket server.
    pub async fn connect(self) -> WebSocketResult<WebSocketClient> {
        let url = self
            .url
            .ok_or_else(|| WebSocketError::InvalidUrl("URL not provided".to_string()))?;

        WebSocketClient::connect_with_config(&url, self.connect_timeout, self.max_message_size)
            .await
    }
}

/// WebSocket client for connecting to WebSocket servers.
pub struct WebSocketClient {
    tx: mpsc::UnboundedSender<Message>,
    rx: mpsc::UnboundedReceiver<Message>,
    /// Thread-safe closed flag using AtomicBool to prevent data races
    /// between send() and close() when client is shared across tasks.
    ///
    /// Shared (via `Arc`) with the reader and writer tasks so that a
    /// remote-initiated close, or either task terminating due to an error,
    /// is reflected here too -- not only an explicit local `close()`.
    closed: Arc<AtomicBool>,
}

impl WebSocketClient {
    /// Create a new client builder.
    pub fn builder() -> WebSocketClientBuilder {
        WebSocketClientBuilder::new()
    }

    /// Connect to a WebSocket server.
    pub async fn connect(url: &str) -> WebSocketResult<Self> {
        Self::connect_with_timeout(url, Duration::from_secs(30)).await
    }

    /// Connect to a WebSocket server with a timeout.
    pub async fn connect_with_timeout(url: &str, timeout: Duration) -> WebSocketResult<Self> {
        Self::connect_with_config(url, timeout, None).await
    }

    /// Connect to a WebSocket server with a timeout and an optional maximum
    /// incoming message size. When `max_message_size` is `Some`, both the
    /// max message size and max frame size are capped at that value, so
    /// oversized frames from the server cause the read to error out instead
    /// of being silently accepted.
    async fn connect_with_config(
        url: &str,
        timeout: Duration,
        max_message_size: Option<usize>,
    ) -> WebSocketResult<Self> {
        let url = Url::parse(url).map_err(|e| WebSocketError::InvalidUrl(e.to_string()))?;

        let ws_config = max_message_size.map(|size| {
            WebSocketConfig::default()
                .max_message_size(Some(size))
                .max_frame_size(Some(size))
        });

        let connect_future = connect_async_with_config(url.as_str(), ws_config, false);

        let (ws_stream, _response) = tokio::time::timeout(timeout, connect_future)
            .await
            .map_err(|_| WebSocketError::Timeout)?
            .map_err(WebSocketError::Protocol)?;

        let (write, read) = ws_stream.split();

        // Create channels for sending and receiving messages
        let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel::<Message>();
        let (incoming_tx, incoming_rx) = mpsc::unbounded_channel::<Message>();

        let closed = Arc::new(AtomicBool::new(false));

        // Spawn writer task
        tokio::spawn(Self::writer_task(write, outgoing_rx, Arc::clone(&closed)));

        // Spawn reader task
        tokio::spawn(Self::reader_task(read, incoming_tx, Arc::clone(&closed)));

        Ok(Self {
            tx: outgoing_tx,
            rx: incoming_rx,
            closed,
        })
    }

    /// Writer task that sends messages to the WebSocket.
    ///
    /// Marks `closed` when the loop exits for any reason -- a local close
    /// request, a channel shutdown, or a write error -- so `is_closed()`
    /// reflects reality even when nobody explicitly called `close()`.
    async fn writer_task(
        mut write: futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            TungsteniteMessage,
        >,
        mut rx: mpsc::UnboundedReceiver<Message>,
        closed: Arc<AtomicBool>,
    ) {
        while let Some(message) = rx.recv().await {
            let is_close = message.is_close();
            // See the matching note in `connection.rs`: a text message whose
            // payload is not UTF-8 is unsendable, so drop it rather than
            // silently corrupting it.
            let raw_message: TungsteniteMessage = match message.try_into() {
                Ok(raw) => raw,
                Err(e) => {
                    tracing::error!(error = %e, "Dropping unsendable WebSocket message");
                    continue;
                }
            };

            if write.send(raw_message).await.is_err() {
                break;
            }

            if is_close {
                break;
            }
        }

        let _ = write.close().await;
        closed.store(true, Ordering::Release);
    }

    /// Reader task that receives messages from the WebSocket.
    ///
    /// Marks `closed` when the loop exits for any reason -- a remote close
    /// frame, a read error, or the stream ending -- so `is_closed()`
    /// reflects reality even when nobody explicitly called `close()`.
    async fn reader_task(
        mut read: futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
        tx: mpsc::UnboundedSender<Message>,
        closed: Arc<AtomicBool>,
    ) {
        while let Some(result) = read.next().await {
            match result {
                Ok(msg) => {
                    if msg.is_close() {
                        let _ = tx.send(Message::close());
                        break;
                    }

                    let message: Message = msg.into();
                    if tx.send(message).is_err() {
                        break;
                    }
                }
                Err(_) => {
                    break;
                }
            }
        }

        closed.store(true, Ordering::Release);
    }

    /// Send a message to the server.
    pub fn send(&self, message: Message) -> WebSocketResult<()> {
        if self.closed.load(Ordering::Acquire) {
            return Err(WebSocketError::ConnectionClosed);
        }
        self.tx
            .send(message)
            .map_err(|e| WebSocketError::Send(e.to_string()))
    }

    /// Send a text message.
    pub fn send_text<S: Into<String>>(&self, text: S) -> WebSocketResult<()> {
        self.send(Message::text(text))
    }

    /// Send a binary message.
    pub fn send_binary<B: Into<bytes::Bytes>>(&self, data: B) -> WebSocketResult<()> {
        self.send(Message::binary(data))
    }

    /// Send a JSON message.
    pub fn send_json<T: serde::Serialize>(&self, value: &T) -> WebSocketResult<()> {
        let message = Message::json(value)?;
        self.send(message)
    }

    /// Receive the next message from the server.
    pub async fn recv(&mut self) -> Option<Message> {
        self.rx.recv().await
    }

    /// Try to receive a message without blocking.
    pub fn try_recv(&mut self) -> Option<Message> {
        self.rx.try_recv().ok()
    }

    /// Close the connection.
    ///
    /// This method uses atomic compare-and-exchange to ensure only one task
    /// sends the close message, even when called concurrently.
    pub fn close(&self) {
        // Atomically set closed from false to true; only proceed if we won the race
        if self
            .closed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let _ = self.tx.send(Message::close());
        }
    }

    /// Check if the connection is closed.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

impl Drop for WebSocketClient {
    fn drop(&mut self) {
        // close() now takes &self, but we have &mut self which coerces
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::SinkExt;
    use std::sync::atomic::AtomicUsize;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message as RawMessage;

    #[tokio::test]
    async fn is_closed_reflects_remote_close() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            // Abruptly drop the connection without a clean close handshake,
            // simulating a dead/failed peer.
            drop(ws);
        });

        let url = format!("ws://{}", addr);
        let mut client = WebSocketClient::connect(&url).await.unwrap();

        assert!(
            !client.is_closed(),
            "freshly connected client should not report closed"
        );

        // Let the reader task observe the abrupt close.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if client.is_closed() {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("client did not observe the remote close within the timeout");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        // The receive channel should also wind down.
        let _ = client.recv().await;
    }

    #[tokio::test]
    async fn client_enforces_max_message_size() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let big = RawMessage::Text("y".repeat(4096).into());
            let _ = ws.send(big).await;
            let _ = ws.close(None).await;
        });

        let url = format!("ws://{}", addr);
        let mut client = WebSocketClient::builder()
            .url(url)
            .max_message_size(64)
            .connect()
            .await
            .unwrap();

        // The oversized message must never surface as a delivered text
        // message; the reader task should hit a protocol error instead.
        let mut got_payload = false;
        for _ in 0..5 {
            match tokio::time::timeout(Duration::from_secs(2), client.recv()).await {
                Ok(Some(msg)) if msg.is_text() => {
                    got_payload = true;
                    break;
                }
                Ok(Some(_)) => continue,
                Ok(None) => break,
                Err(_) => break,
            }
        }

        assert!(
            !got_payload,
            "client configured with a small max_message_size should reject an oversized message"
        );
    }

    #[tokio::test]
    async fn concurrent_close_sends_exactly_one_close_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let close_count = Arc::new(AtomicUsize::new(0));
        let close_count_srv = Arc::clone(&close_count);

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(msg)) = ws.next().await {
                if msg.is_close() {
                    close_count_srv.fetch_add(1, Ordering::SeqCst);
                    break;
                }
            }
        });

        let url = format!("ws://{}", addr);
        let client = WebSocketClient::connect(&url).await.unwrap();

        // Call close() from multiple OS threads at once; the atomic
        // compare-and-exchange in close() must ensure only one of them
        // actually sends the close message downstream.
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| client.close());
            }
        });

        // Give the server task a moment to observe the close frame.
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert_eq!(
            close_count.load(Ordering::SeqCst),
            1,
            "exactly one close frame should have been sent despite concurrent close() calls"
        );
    }
}
