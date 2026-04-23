//! # Armature WebSocket
//!
//! WebSocket server and client support for the Armature framework using tokio-tungstenite.
//!
//! ## Features
//!
//! - WebSocket server with connection management
//! - WebSocket client for outbound connections
//! - Room-based message broadcasting
//! - Connection state management
//! - Heartbeat/ping-pong support
//! - JSON message serialization
//!
//! ## Example
//!
//! ```rust,no_run
//! use armature_websocket::{WebSocketServer, WebSocketHandler, Message};
//! use async_trait::async_trait;
//!
//! struct ChatHandler;
//!
//! #[async_trait]
//! impl WebSocketHandler for ChatHandler {
//!     async fn on_connect(&self, connection_id: &str) {
//!         println!("Client connected: {}", connection_id);
//!     }
//!
//!     async fn on_message(&self, connection_id: &str, message: Message) {
//!         println!("Received from {}: {:?}", connection_id, message);
//!     }
//!
//!     async fn on_disconnect(&self, connection_id: &str) {
//!         println!("Client disconnected: {}", connection_id);
//!     }
//! }
//! ```

#![warn(missing_docs)]
#![warn(rustdoc::missing_crate_level_docs)]

mod client;
mod connection;
mod error;
mod handler;
mod message;
mod room;
mod server;

pub use client::{WebSocketClient, WebSocketClientBuilder};
pub use connection::{Connection, ConnectionId, ConnectionState};
pub use error::{WebSocketError, WebSocketResult};
pub use handler::{LoggingHandler, WebSocketHandler};
pub use message::{Message, MessageType};
pub use room::{Room, RoomId, RoomManager};
pub use server::{WebSocketServer, WebSocketServerBuilder, WebSocketServerConfig};

// Re-export commonly used types from tungstenite
pub use tungstenite::Message as RawMessage;
pub use tungstenite::protocol::CloseFrame;
