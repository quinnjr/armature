//! GraphQL subscription support.

use futures::Stream;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use crate::{GraphQLResponse, Result};

/// A GraphQL subscription stream.
///
/// ## Lifecycle
///
/// The stream is driven by polling it (e.g. with [`futures::StreamExt::next`]).
/// It ends when the server sends a `complete`/`close` frame, when the transport
/// errors, or when the client unsubscribes.
///
/// A subscription can be cancelled by the client at any time, either explicitly
/// via [`SubscriptionStream::unsubscribe`] or implicitly by dropping the stream.
/// In both cases the client makes a best-effort attempt to send a graphql-ws
/// `complete` message carrying this subscription's id, so the server can stop
/// producing events rather than the connection just going away. The `complete`
/// is attempted at most once, and is skipped entirely when there is no tokio
/// runtime to send it on (for example when the stream is dropped on a
/// non-runtime thread or during runtime shutdown).
pub struct SubscriptionStream<T = Value> {
    inner: Pin<Box<dyn Stream<Item = Result<GraphQLResponse<T>>> + Send>>,
    subscription: Option<Subscription>,
}

impl<T> SubscriptionStream<T> {
    /// Create a new subscription stream with no client-initiated unsubscribe
    /// wiring (the stream ends only when the server or transport ends it).
    pub fn new<S>(stream: S) -> Self
    where
        S: Stream<Item = Result<GraphQLResponse<T>>> + Send + 'static,
    {
        Self {
            inner: Box::pin(stream),
            subscription: None,
        }
    }

    /// Create a subscription stream bound to a [`Subscription`] handle so the
    /// client can initiate an unsubscribe (sending a graphql-ws `complete`).
    pub(crate) fn with_subscription<S>(stream: S, subscription: Subscription) -> Self
    where
        S: Stream<Item = Result<GraphQLResponse<T>>> + Send + 'static,
    {
        Self {
            inner: Box::pin(stream),
            subscription: Some(subscription),
        }
    }

    /// The handle for this stream's subscription, if it was created with one.
    pub fn subscription(&self) -> Option<&Subscription> {
        self.subscription.as_ref()
    }

    /// Whether this subscription is still active (has not been unsubscribed).
    ///
    /// Streams created without a [`Subscription`] handle are always considered
    /// active until they end.
    pub fn is_active(&self) -> bool {
        self.subscription
            .as_ref()
            .is_none_or(Subscription::is_active)
    }

    /// Explicitly unsubscribe: best-effort send of a graphql-ws `complete` for
    /// this subscription's id (attempted at most once). No-op if the stream has
    /// no handle or has already been unsubscribed.
    pub fn unsubscribe(&mut self) {
        if let Some(subscription) = self.subscription.as_mut() {
            subscription.deactivate();
        }
    }
}

impl<T> Drop for SubscriptionStream<T> {
    fn drop(&mut self) {
        // Dropping the stream is an implicit client-initiated unsubscribe: tell
        // the server to stop producing events instead of just tearing down the
        // socket. `deactivate` is idempotent, so an explicit `unsubscribe()`
        // before drop means nothing is sent here.
        if let Some(subscription) = self.subscription.as_mut() {
            subscription.deactivate();
        }
    }
}

impl<T: DeserializeOwned + Unpin> Stream for SubscriptionStream<T> {
    type Item = Result<T>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(response))) => Poll::Ready(Some(response.into_result())),
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// A handle to an active graphql-ws subscription.
///
/// Beyond tracking the subscription id and whether it is still active, a handle
/// obtained from a live [`SubscriptionStream`] carries the wiring needed to
/// send a client-initiated `complete` message: calling [`deactivate`] tells the
/// server to stop producing events for this subscription. The `complete` is
/// attempted at most once, whether triggered by [`deactivate`], by
/// [`SubscriptionStream::unsubscribe`], or by dropping the stream, and is
/// best-effort — see [`SubscriptionStream`] for when it is skipped.
///
/// [`deactivate`]: Subscription::deactivate
#[derive(Clone)]
pub struct Subscription {
    /// Subscription ID.
    pub id: String,
    /// Whether the subscription is still active. Shared so that a handle and the
    /// stream it belongs to agree on state and only ever send one `complete`.
    active: Arc<AtomicBool>,
    /// Sends the graphql-ws `complete` for [`id`](Self::id). `None` for handles
    /// not bound to a live transport (e.g. constructed via [`Subscription::new`]).
    unsubscribe: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription")
            .field("id", &self.id)
            .field("active", &self.is_active())
            .field("bound", &self.unsubscribe.is_some())
            .finish()
    }
}

impl Subscription {
    /// Create a new, unbound subscription handle. Such a handle tracks active
    /// state but has no transport wiring, so [`deactivate`](Self::deactivate)
    /// only flips the flag.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            active: Arc::new(AtomicBool::new(true)),
            unsubscribe: None,
        }
    }

    /// Create a subscription handle bound to a transport-level unsubscribe
    /// action that emits the graphql-ws `complete` message for `id`.
    pub(crate) fn bound(id: impl Into<String>, unsubscribe: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self {
            id: id.into(),
            active: Arc::new(AtomicBool::new(true)),
            unsubscribe: Some(unsubscribe),
        }
    }

    /// Check if the subscription is active.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// Unsubscribe: mark the subscription inactive and, if this handle is bound
    /// to a live transport, make a best-effort attempt to send the graphql-ws
    /// `complete` message for its id. Idempotent — the `complete` is attempted
    /// only on the first call.
    pub fn deactivate(&mut self) {
        // `swap` returning the previous value guarantees exactly one caller
        // wins the transition from active→inactive and sends `complete`.
        if self.active.swap(false, Ordering::SeqCst)
            && let Some(unsubscribe) = self.unsubscribe.as_ref()
        {
            unsubscribe();
        }
    }
}

/// graphql-ws protocol messages.
pub mod protocol {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    /// Client to server message types.
    #[derive(Debug, Clone, Serialize)]
    #[serde(tag = "type")]
    pub enum ClientMessage {
        /// Initialize connection.
        #[serde(rename = "connection_init")]
        ConnectionInit {
            #[serde(skip_serializing_if = "Option::is_none")]
            payload: Option<Value>,
        },
        /// Start a subscription.
        #[serde(rename = "subscribe")]
        Subscribe {
            id: String,
            payload: SubscribePayload,
        },
        /// Complete a subscription.
        #[serde(rename = "complete")]
        Complete { id: String },
        /// Ping for keep-alive.
        #[serde(rename = "ping")]
        Ping {
            #[serde(skip_serializing_if = "Option::is_none")]
            payload: Option<Value>,
        },
        /// Pong response.
        #[serde(rename = "pong")]
        Pong {
            #[serde(skip_serializing_if = "Option::is_none")]
            payload: Option<Value>,
        },
    }

    /// Subscribe payload.
    #[derive(Debug, Clone, Serialize)]
    pub struct SubscribePayload {
        pub query: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub operation_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub variables: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub extensions: Option<Value>,
    }

    /// Server to client message types.
    #[derive(Debug, Clone, Deserialize)]
    #[serde(tag = "type")]
    pub enum ServerMessage {
        /// Connection acknowledged.
        #[serde(rename = "connection_ack")]
        ConnectionAck {
            #[serde(default)]
            payload: Option<Value>,
        },
        /// Subscription data.
        #[serde(rename = "next")]
        Next { id: String, payload: Value },
        /// Subscription error.
        #[serde(rename = "error")]
        Error { id: String, payload: Vec<Value> },
        /// Subscription complete.
        #[serde(rename = "complete")]
        Complete { id: String },
        /// Ping from server.
        #[serde(rename = "ping")]
        Ping {
            #[serde(default)]
            payload: Option<Value>,
        },
        /// Pong from server.
        #[serde(rename = "pong")]
        Pong {
            #[serde(default)]
            payload: Option<Value>,
        },
    }
}
