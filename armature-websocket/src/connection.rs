//! WebSocket connection management.

use crate::error::{WebSocketError, WebSocketResult};
use crate::message::Message;
use futures_util::SinkExt;
use futures_util::stream::SplitSink;
use parking_lot::RwLock;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;

/// Unique identifier for a connection.
pub type ConnectionId = String;

/// Connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Connection is being established
    Connecting,
    /// Connection is open and ready
    Open,
    /// Connection is closing
    Closing,
    /// Connection is closed
    Closed,
}

/// A WebSocket connection.
pub struct Connection {
    /// Unique connection identifier
    pub id: ConnectionId,
    /// Remote address
    pub remote_addr: Option<SocketAddr>,
    /// Connection state
    state: Arc<RwLock<ConnectionState>>,
    /// Sender for outgoing messages
    tx: mpsc::UnboundedSender<Message>,
}

impl Connection {
    /// Create a new connection.
    pub(crate) fn new(
        id: ConnectionId,
        remote_addr: Option<SocketAddr>,
        tx: mpsc::UnboundedSender<Message>,
    ) -> Self {
        Self {
            id,
            remote_addr,
            state: Arc::new(RwLock::new(ConnectionState::Open)),
            tx,
        }
    }

    /// Get the connection state.
    pub fn state(&self) -> ConnectionState {
        *self.state.read()
    }

    /// Check if the connection is open.
    pub fn is_open(&self) -> bool {
        self.state() == ConnectionState::Open
    }

    /// Send a message to this connection.
    pub fn send(&self, message: Message) -> WebSocketResult<()> {
        if !self.is_open() {
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

    /// Close the connection.
    pub fn close(&self) {
        *self.state.write() = ConnectionState::Closing;
        let _ = self.tx.send(Message::close());
    }

    /// Set the connection state (internal use).
    pub(crate) fn set_state(&self, state: ConnectionState) {
        *self.state.write() = state;
    }
}

impl Clone for Connection {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            remote_addr: self.remote_addr,
            state: Arc::clone(&self.state),
            tx: self.tx.clone(),
        }
    }
}

/// Manages the write side of a WebSocket connection.
pub(crate) struct ConnectionWriter {
    sink: SplitSink<WebSocketStream<TcpStream>, tungstenite::Message>,
    rx: mpsc::UnboundedReceiver<Message>,
}

impl ConnectionWriter {
    /// Create a new connection writer.
    pub fn new(
        sink: SplitSink<WebSocketStream<TcpStream>, tungstenite::Message>,
        rx: mpsc::UnboundedReceiver<Message>,
    ) -> Self {
        Self { sink, rx }
    }

    /// Run the writer loop, sending messages from the channel to the WebSocket.
    pub async fn run(mut self) -> WebSocketResult<()> {
        while let Some(message) = self.rx.recv().await {
            let is_close = message.is_close();
            let raw_message: tungstenite::Message = message.into();

            if let Err(e) = self.sink.send(raw_message).await {
                tracing::error!(error = %e, "Failed to send WebSocket message");
                return Err(WebSocketError::Protocol(e));
            }

            if is_close {
                break;
            }
        }

        // Gracefully close the sink
        let _ = self.sink.close().await;
        Ok(())
    }
}
