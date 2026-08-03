//! Service Discovery for Armature
//!
//! This crate provides service discovery and registration capabilities.
//!
//! ## Features
//!
//! - **Service Registration** - Register service instances
//! - **Service Discovery** - Find registered service instances
//! - **Health Checks** - On-demand probing via
//!   [`ServiceDiscovery::health_check`]
//! - **Load Balancing** - Round-robin, random, or custom strategies
//! - **Multiple Backends** - Consul, etcd, or in-memory
//!
//! ## Health and liveness
//!
//! `discover` returns every *registered* instance, not every *healthy* one.
//! This crate runs no background health checker:
//! [`ServiceDiscovery::health_check`] is an on-demand probe you must call
//! yourself, and [`ServiceResolver::resolve`] does not consult it.
//!
//! What each backend does about instances that never deregister differs:
//!
//! - [`EtcdDiscovery`] writes registrations under a lease with a TTL and
//!   refreshes it from a background task, so a crashed instance disappears
//!   roughly one TTL after its process dies.
//! - [`ConsulDiscovery`] queries Consul with `passing=true`, so Consul's own
//!   health checks filter the results.
//! - [`InMemoryDiscovery`] keeps registrations until they are explicitly
//!   deregistered; it is intended for tests.
//!
//! ## Quick Start
//!
//! ### In-Memory Discovery (Testing)
//!
//! ```rust,ignore
//! use armature_discovery::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let discovery = InMemoryDiscovery::new();
//!
//!     // Register a service
//!     let service = ServiceInstance::new("api-1", "api", "localhost", 8080)
//!         .with_tag("v1")
//!         .with_health_check("http://localhost:8080/health");
//!
//!     discovery.register(&service).await?;
//!
//!     // Discover services
//!     let instances = discovery.discover("api").await?;
//!     for instance in instances {
//!         println!("Found: {}", instance.url());
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Consul Discovery
//!
//! ```rust,ignore
//! use armature_discovery::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let consul = ConsulDiscovery::new("http://localhost:8500")?;
//!
//!     let service = ServiceInstance::new("api-1", "api", "localhost", 8080);
//!     consul.register(&service).await?;
//!
//!     let instances = consul.discover("api").await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Service Resolver with Load Balancing
//!
//! ```rust,ignore
//! use armature_discovery::*;
//!
//! let discovery = InMemoryDiscovery::new();
//! let resolver = ServiceResolver::new(discovery, LoadBalancingStrategy::RoundRobin);
//!
//! // Automatically selects instance using round-robin
//! let instance = resolver.resolve("api").await?;
//! ```

pub mod consul;
pub mod etcd;
pub mod memory;
pub mod service;

pub use consul::ConsulDiscovery;
pub use etcd::EtcdDiscovery;
pub use memory::InMemoryDiscovery;
pub use service::{
    DiscoveryError, LoadBalancingStrategy, ServiceDiscovery, ServiceInstance, ServiceResolver,
};

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_exports() {
        // Ensure module compiles
    }
}
