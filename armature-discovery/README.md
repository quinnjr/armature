# armature-discovery

Service discovery for the Armature framework.

## Features

- **Service Registration** - Register services on startup
- **Service Discovery** - Find other services
- **Health Checks** - On-demand probing via `ServiceDiscovery::health_check`
- **Load Balancing** - Client-side load balancing (round-robin, random, or first)
- **Multiple Backends** - Consul, etcd, or in-memory (for testing)

## Installation

```toml
[dependencies]
armature-discovery = "0.1"
```

## Quick Start

```rust
use armature_discovery::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let discovery = InMemoryDiscovery::new();

    // Register this service
    let service = ServiceInstance::new("api-1", "api", "localhost", 8080)
        .with_tag("v1")
        .with_health_check("http://localhost:8080/health");

    discovery.register(&service).await?;

    // Discover other instances of a service
    let instances = discovery.discover("api").await?;
    for instance in instances {
        println!("Found: {}", instance.url());
    }

    // Get one instance, load balanced
    let resolver = ServiceResolver::new(discovery, LoadBalancingStrategy::RoundRobin);
    let instance = resolver.resolve("api").await?;

    Ok(())
}
```

## Backends

### Consul

```rust
use armature_discovery::ConsulDiscovery;

let consul = ConsulDiscovery::new("http://localhost:8500")?;
consul.register(&service).await?;
let instances = consul.discover("api").await?;
```

`discover` asks Consul to filter to instances whose health checks are
currently passing (`?passing=true`), so unhealthy instances are never
returned.

### etcd

```rust
use armature_discovery::EtcdDiscovery;
use std::time::Duration;

let etcd = EtcdDiscovery::new("http://localhost:2379", "/services")?
    .with_lease_ttl(Duration::from_secs(10)); // optional; defaults to 30s
etcd.register(&service).await?;
let instances = etcd.discover("api").await?;
```

`register` writes both of its keys in a single etcd transaction, attached to a
lease that a background task keeps alive. If the registering process dies the
lease expires and etcd drops the registration, so a crashed instance stops
being discoverable within roughly one TTL. The lease covers liveness only —
`discover` does not filter on health, and nothing probes instances in the
background.

### In-Memory

`InMemoryDiscovery` keeps everything in a process-local map. It implements
the same `ServiceDiscovery` trait as the network-backed clients, so it's a
drop-in stand-in for tests and local development.

## License

MIT OR Apache-2.0

