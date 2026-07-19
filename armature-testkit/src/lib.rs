//! Deterministic, offline test harnesses for verifying Armature integrations.

pub mod docker;
pub mod http_stub;

pub use docker::docker_available;
pub use http_stub::{RecordedRequest, StubResponse, StubServer, StubServerBuilder};
