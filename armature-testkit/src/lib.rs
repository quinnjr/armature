//! Deterministic, offline test harnesses for verifying Armature integrations.

pub mod http_stub;

pub use http_stub::{RecordedRequest, StubResponse, StubServer, StubServerBuilder};
