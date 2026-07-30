//! Zero-allocation thread-per-core HTTP/1.1 server.
//!
//! See `docs/superpowers/specs/2026-07-29-armature-h1-design.md`.
//!
//! # Design
//!
//! Request heads are parsed into [`Bytes`](bytes::Bytes) slices of a per-core
//! pooled read buffer, so header values and bodies cost a refcount increment
//! rather than an allocation. Framing decisions live in one pure function and
//! every rejection closes the connection rather than resynchronizing the
//! stream.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bytestr;
mod head;
pub mod header;
pub mod limits;
mod method;
pub mod parse;

pub use bytestr::ByteStr;
pub use head::Head;
pub use header::{HeaderId, HeaderVec};
pub use limits::Limits;
pub use method::{Method, Version};
pub use parse::{ParseError, parse_head};
