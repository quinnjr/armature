# armature-distributed

Distributed system primitives for the Armature framework.

## Features

- **Distributed Locks** - Redis-based distributed locks with token-guarded, auto-renewing leases
- **Leader Election** - Automatic leader election with callbacks

## Installation

```toml
[dependencies]
armature-distributed = "0.1"
```

## Quick Start

### Distributed Lock

```rust,ignore
use armature_distributed::{DistributedLock, RedisLock};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = redis::Client::open("redis://127.0.0.1/")?;
    let conn = client.get_connection_manager().await?;

    let lock = RedisLock::new("my-resource", Duration::from_secs(30), conn);

    // Acquire lock (blocks until available)
    let guard = lock.acquire().await?;

    // Do exclusive work...

    // Lock is automatically released when guard is dropped
    drop(guard);

    Ok(())
}
```

### Leader Election

```rust,ignore
use armature_distributed::LeaderElection;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = redis::Client::open("redis://127.0.0.1/")?;
    let conn = client.get_connection_manager().await?;

    let election = Arc::new(
        LeaderElection::new("my-service-leader", Duration::from_secs(30), conn)
            .on_elected(|| async {
                println!("I am the leader!");
            })
            .on_revoked(|| async {
                println!("I lost leadership");
            }),
    );

    // Start election (runs in background)
    let election_clone = election.clone();
    tokio::spawn(async move { election_clone.start().await });

    // Check leadership status
    if election.is_leader() {
        println!("This node is the leader");
    }

    Ok(())
}
```

## License

MIT OR Apache-2.0

