//! Deterministic, offline test harnesses for verifying Armature integrations.

#[cfg(feature = "containers")]
pub mod acme;
#[cfg(feature = "containers")]
pub mod containers;
pub mod docker;
pub mod http_stub;

pub use docker::{REQUIRE_DOCKER_ENV, docker_available, docker_gate, require_docker_env};
pub use http_stub::{RecordedRequest, StubResponse, StubServer, StubServerBuilder};
