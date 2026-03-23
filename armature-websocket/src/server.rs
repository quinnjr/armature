//! WebSocket server implementation.

use crate::connection::{Connection, ConnectionWriter};
use crate::error::{WebSocketError, WebSocketResult};
use crate::handler::WebSocketHandler;
use crate::message::Message;
use crate::room::RoomManager;
use futures_util::StreamExt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::accept_async;

/// WebSocket server configuration.
#[derive(Debug, Clone)]
pub struct WebSocketServerConfig {
    /// Address to bind to
    pub bind_addr: SocketAddr,
    /// Maximum message size in bytes
    pub max_message_size: usize,
    /// Heartbeat interval
    pub heartbeat_interval: Duration,
    /// Connection timeout
    pub connection_timeout: Duration,
}

impl Default for WebSocketServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:9001".parse().unwrap(),
            max_message_size: 64 * 1024, // 64KB
            heartbeat_interval: Duration::from_secs(30),
            connection_timeout: Duration::from_secs(60),
        }
    }
}

/// Builder for WebSocket server configuration.
#[derive(Debug, Default)]
pub struct WebSocketServerBuilder {
    config: WebSocketServerConfig,
}

impl WebSocketServerBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the bind address.
    pub fn bind_addr(mut self, addr: SocketAddr) -> Self {
        self.config.bind_addr = addr;
        self
    }

    /// Set the bind address from a string.
    pub fn bind(mut self, addr: &str) -> WebSocketResult<Self> {
        self.config.bind_addr = addr
            .parse()
            .map_err(|e| WebSocketError::Server(format!("Invalid address: {}", e)))?;
        Ok(self)
    }

    /// Set the maximum message size.
    pub fn max_message_size(mut self, size: usize) -> Self {
        self.config.max_message_size = size;
        self
    }

    /// Set the heartbeat interval.
    pub fn heartbeat_interval(mut self, interval: Duration) -> Self {
        self.config.heartbeat_interval = interval;
        self
    }

    /// Set the connection timeout.
    pub fn connection_timeout(mut self, timeout: Duration) -> Self {
        self.config.connection_timeout = timeout;
        self
    }

    /// Build the server with the given handler.
    pub fn build<H: WebSocketHandler>(self, handler: H) -> WebSocketServer<H> {
        WebSocketServer::new(self.config, handler)
    }
}

/// WebSocket server.
pub struct WebSocketServer<H: WebSocketHandler> {
    config: WebSocketServerConfig,
    handler: Arc<H>,
    room_manager: Arc<RoomManager>,
}

impl<H: WebSocketHandler> WebSocketServer<H> {
    /// Create a new WebSocket server.
    pub fn new(config: WebSocketServerConfig, handler: H) -> Self {
        Self {
            config,
            handler: Arc::new(handler),
            room_manager: Arc::new(RoomManager::new()),
        }
    }

    /// Create a builder for the server.
    pub fn builder() -> WebSocketServerBuilder {
        WebSocketServerBuilder::new()
    }

    /// Get a reference to the room manager.
    pub fn room_manager(&self) -> &Arc<RoomManager> {
        &self.room_manager
    }

    /// Run the server.
    pub async fn run(&self) -> WebSocketResult<()> {
        let listener = TcpListener::bind(self.config.bind_addr).await?;
        tracing::info!(addr = %self.config.bind_addr, "WebSocket server listening");

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let handler = Arc::clone(&self.handler);
                    let room_manager = Arc::clone(&self.room_manager);
                    let config = self.config.clone();

                    tokio::spawn(async move {
                        if let Err(e) =
                            Self::handle_connection(stream, addr, handler, room_manager, config)
                                .await
                        {
                            tracing::error!(addr = %addr, error = %e, "Connection error");
                        }
                    });
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to accept connection");
                }
            }
        }
    }

    /// Handle a single connection.
    async fn handle_connection(
        stream: TcpStream,
        addr: SocketAddr,
        handler: Arc<H>,
        room_manager: Arc<RoomManager>,
        _config: WebSocketServerConfig,
    ) -> WebSocketResult<()> {
        let ws_stream = accept_async(stream).await?;
        let connection_id = uuid::Uuid::new_v4().to_string();

        tracing::debug!(connection_id = %connection_id, addr = %addr, "WebSocket connection established");

        // Split the WebSocket stream
        let (write, mut read) = ws_stream.split();

        // Create message channel
        let (tx, rx) = mpsc::unbounded_channel();

        // Create connection object
        let connection = Connection::new(connection_id.clone(), Some(addr), tx);

        // Register connection
        room_manager.register_connection(connection.clone());

        // Notify handler of connection
        handler.on_connect(&connection_id).await;

        // Spawn writer task
        let writer = ConnectionWriter::new(write, rx);
        let writer_handle = tokio::spawn(async move { writer.run().await });

        // Read messages
        while let Some(result) = read.next().await {
            match result {
                Ok(msg) => {
                    if msg.is_close() {
                        break;
                    }

                    let message: Message = msg.into();

                    // Handle ping/pong
                    if message.is_ping() {
                        let pong_payload =
                            handler.on_ping(&connection_id, message.as_bytes()).await;
                        let _ = connection.send(Message::pong(pong_payload));
                        continue;
                    }

                    if message.is_pong() {
                        handler.on_pong(&connection_id, message.as_bytes()).await;
                        continue;
                    }

                    // Handle regular message
                    handler.on_message(&connection_id, message).await;
                }
                Err(e) => {
                    let ws_error = WebSocketError::Protocol(e);
                    handler.on_error(&connection_id, &ws_error).await;
                    break;
                }
            }
        }

        // Close connection
        connection.close();

        // Wait for writer to finish
        let _ = writer_handle.await;

        // Notify handler of disconnection
        handler.on_disconnect(&connection_id).await;

        // Unregister connection
        room_manager.unregister_connection(&connection_id);

        tracing::debug!(connection_id = %connection_id, "WebSocket connection closed");

        Ok(())
    }
}
