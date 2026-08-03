//! WebSocket message types.

use bytes::Bytes;
use serde::{Deserialize, Serialize};

/// Message type enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum MessageType {
    /// Text message
    Text,
    /// Binary message
    Binary,
    /// Ping message
    Ping,
    /// Pong message
    Pong,
    /// Close message
    Close,
}

/// A WebSocket message.
#[derive(Debug, Clone)]
pub struct Message {
    /// The message type
    pub message_type: MessageType,
    /// The message payload
    pub payload: Bytes,
}

impl Message {
    /// Create a new text message.
    pub fn text<S: Into<String>>(text: S) -> Self {
        Self {
            message_type: MessageType::Text,
            payload: Bytes::from(text.into()),
        }
    }

    /// Create a new binary message.
    pub fn binary<B: Into<Bytes>>(data: B) -> Self {
        Self {
            message_type: MessageType::Binary,
            payload: data.into(),
        }
    }

    /// Create a new ping message.
    pub fn ping<B: Into<Bytes>>(data: B) -> Self {
        Self {
            message_type: MessageType::Ping,
            payload: data.into(),
        }
    }

    /// Create a new pong message.
    pub fn pong<B: Into<Bytes>>(data: B) -> Self {
        Self {
            message_type: MessageType::Pong,
            payload: data.into(),
        }
    }

    /// Create a close message.
    pub fn close() -> Self {
        Self {
            message_type: MessageType::Close,
            payload: Bytes::new(),
        }
    }

    /// Create a JSON message from a serializable value.
    pub fn json<T: Serialize>(value: &T) -> Result<Self, serde_json::Error> {
        let json = serde_json::to_string(value)?;
        Ok(Self::text(json))
    }

    /// Parse the message payload as JSON.
    pub fn parse_json<'a, T: Deserialize<'a>>(&'a self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.payload)
    }

    /// Get the message payload as a string.
    pub fn as_text(&self) -> Option<&str> {
        if self.message_type == MessageType::Text {
            std::str::from_utf8(&self.payload).ok()
        } else {
            None
        }
    }

    /// Get the message payload as bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.payload
    }

    /// Check if this is a text message.
    pub fn is_text(&self) -> bool {
        self.message_type == MessageType::Text
    }

    /// Check if this is a binary message.
    pub fn is_binary(&self) -> bool {
        self.message_type == MessageType::Binary
    }

    /// Check if this is a ping message.
    pub fn is_ping(&self) -> bool {
        self.message_type == MessageType::Ping
    }

    /// Check if this is a pong message.
    pub fn is_pong(&self) -> bool {
        self.message_type == MessageType::Pong
    }

    /// Check if this is a close message.
    pub fn is_close(&self) -> bool {
        self.message_type == MessageType::Close
    }
}

impl From<tungstenite::Message> for Message {
    fn from(msg: tungstenite::Message) -> Self {
        match msg {
            tungstenite::Message::Text(text) => Self::text(text.to_string()),
            tungstenite::Message::Binary(data) => Self::binary(data),
            tungstenite::Message::Ping(data) => Self::ping(data),
            tungstenite::Message::Pong(data) => Self::pong(data),
            tungstenite::Message::Close(_) => Self::close(),
            tungstenite::Message::Frame(_) => Self::binary(Bytes::new()),
        }
    }
}

/// A [`Message`] that cannot be put on the wire as-is.
///
/// Only one thing can go wrong: a text message whose payload is not valid
/// UTF-8. RFC 6455 §5.6 requires text frames to carry UTF-8, and the payload is
/// `Bytes`, so nothing stops a caller constructing `Message { message_type:
/// Text, payload: <arbitrary bytes> }`.
#[derive(Debug, Clone)]
pub struct InvalidTextPayload {
    /// Byte offset at which decoding failed.
    pub valid_up_to: usize,
}

impl std::fmt::Display for InvalidTextPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "text message payload is not valid UTF-8 (valid up to byte {})",
            self.valid_up_to
        )
    }
}

impl std::error::Error for InvalidTextPayload {}

impl TryFrom<Message> for tungstenite::Message {
    type Error = InvalidTextPayload;

    /// Convert without copying the payload.
    ///
    /// Binary/Ping/Pong hand `msg.payload` straight through: both sides are
    /// `Bytes`, so this is a refcount bump rather than the full memcpy a
    /// `to_vec()` cost - once per recipient on a broadcast.
    ///
    /// Text is checked rather than lossily converted. `from_utf8_lossy` would
    /// substitute U+FFFD for every invalid byte, silently delivering corrupted
    /// text the peer has no way to detect; an error lets the caller decide.
    fn try_from(msg: Message) -> Result<Self, Self::Error> {
        Ok(match msg.message_type {
            MessageType::Text => {
                let text = std::str::from_utf8(&msg.payload).map_err(|e| InvalidTextPayload {
                    valid_up_to: e.valid_up_to(),
                })?;
                tungstenite::Message::Text(text.to_owned().into())
            }
            MessageType::Binary => tungstenite::Message::Binary(msg.payload),
            MessageType::Ping => tungstenite::Message::Ping(msg.payload),
            MessageType::Pong => tungstenite::Message::Pong(msg.payload),
            MessageType::Close => tungstenite::Message::Close(None),
        })
    }
}
