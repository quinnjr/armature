//! WebSocket client implementation.

use crate::error::{WebSocketError, WebSocketResult};
use crate::message::Message;
use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as TungsteniteMessage};
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

        WebSocketClient::connect_with_timeout(&url, self.connect_timeout).await
    }
}

/// WebSocket client for connecting to WebSocket servers.
pub struct WebSocketClient {
    tx: mpsc::UnboundedSender<Message>,
    rx: mpsc::UnboundedReceiver<Message>,
    /// Thread-safe closed flag using AtomicBool to prevent data races
    /// between send() and close() when client is shared across tasks.
    closed: AtomicBool,
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
        let url = Url::parse(url).map_err(|e| WebSocketError::InvalidUrl(e.to_string()))?;

        let connect_future = connect_async(url.as_str());

        let (ws_stream, _response) = tokio::time::timeout(timeout, connect_future)
            .await
            .map_err(|_| WebSocketError::Timeout)?
            .map_err(WebSocketError::Protocol)?;

        let (write, read) = ws_stream.split();

        // Create channels for sending and receiving messages
        let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel::<Message>();
        let (incoming_tx, incoming_rx) = mpsc::unbounded_channel::<Message>();

        // Spawn writer task
        tokio::spawn(Self::writer_task(write, outgoing_rx));

        // Spawn reader task
        tokio::spawn(Self::reader_task(read, incoming_tx));

        Ok(Self {
            tx: outgoing_tx,
            rx: incoming_rx,
            closed: AtomicBool::new(false),
        })
    }

    /// Writer task that sends messages to the WebSocket.
    async fn writer_task(
        mut write: futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            TungsteniteMessage,
        >,
        mut rx: mpsc::UnboundedReceiver<Message>,
    ) {
        while let Some(message) = rx.recv().await {
            let is_close = message.is_close();
            let raw_message: TungsteniteMessage = message.into();

            if write.send(raw_message).await.is_err() {
                break;
            }

            if is_close {
                break;
            }
        }

        let _ = write.close().await;
    }

    /// Reader task that receives messages from the WebSocket.
    async fn reader_task(
        mut read: futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
        tx: mpsc::UnboundedSender<Message>,
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
