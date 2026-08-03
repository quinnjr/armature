//! Zero-allocation thread-per-core HTTP/1.1 server.
//!
//! See `docs/superpowers/specs/2026-07-29-armature-h1-design.md`.
//!
//! # Design
//!
//! Request heads are parsed into [`Bytes`](bytes::Bytes) slices of the
//! connection's own read buffer, so header values and bodies cost a refcount
//! increment rather than an allocation. That buffer is allocated once when the
//! connection is created and reused for every request on it — there is no
//! cross-connection buffer pool, because a buffer that outlives its connection
//! is a buffer a handler can still be holding a slice of. Framing decisions live
//! in one pure function and every rejection closes the connection rather than
//! resynchronizing the stream.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bytestr;
pub mod chunked;
pub mod conn;
pub mod deadline;
pub mod framing;
mod head;
pub mod header;
pub mod limits;
mod method;
pub mod parse;
pub mod server;
pub mod service;
pub mod tls;
pub mod write;

pub use bytestr::ByteStr;
pub use chunked::{ChunkEvent, ChunkedDecoder, ChunkedError};
pub use conn::{ConnConfig, Connection, Disposition};
pub use deadline::ConnDeadline;
pub use framing::{BodyKind, FramingError};
pub use head::Head;
pub use header::{HeaderId, HeaderVec};
pub use limits::Limits;
pub use method::{Method, Version};
pub use parse::{ParseError, parse_head};
pub use server::{Config, Server, ServerHandle, TcpConfig};
pub use service::{
    Body, BodyError, BodyIo, H1Service, Request, Response, ResponseBody, Transport, Upgraded,
};
pub use tls::{H2C_PREFACE, H2Fallback, Preface, is_h2c_preface};
pub use write::{DateCache, OutBody, ResponseHead};
