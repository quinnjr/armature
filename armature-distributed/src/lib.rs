//! Distributed Systems Support for Armature
//!
//! This crate provides distributed coordination primitives.
//!
//! ## Features
//!
//! - **Distributed Locks** - Redis-based distributed locks
//! - **Leader Election** - Automatic leader election with callbacks
//! - **TTL Management** - Automatic lock/leadership renewal
//! - **RAII Pattern** - Automatic cleanup on drop
//!
//! ## Quick Start
//!
//! ### Distributed Locks
//!
//! ```rust,ignore
//! use armature_distributed::*;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Connect to Redis
//!     let client = redis::Client::open("redis://127.0.0.1/")?;
//!     let conn = client.get_connection_manager().await?;
//!
//!     // Create a distributed lock
//!     let lock = RedisLock::new("my-resource", Duration::from_secs(30), conn);
//!
//!     // Acquire the lock
//!     let guard = lock.acquire().await?;
//!
//!     // Critical section
//!     println!("Lock acquired, doing work...");
//!
//!     // Lock is automatically released when guard is dropped
//!     drop(guard);
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Leader Election
//!
//! ```rust,ignore
//! use armature_distributed::*;
//! use std::sync::Arc;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = redis::Client::open("redis://127.0.0.1/")?;
//!     let conn = client.get_connection_manager().await?;
//!
//!     let election = Arc::new(
//!         LeaderElection::new("my-service-leader", Duration::from_secs(30), conn)
//!             .on_elected(|| async {
//!                 println!("I am the leader!");
//!             })
//!             .on_revoked(|| async {
//!                 println!("I lost leadership");
//!             })
//!     );
//!
//!     // Start election (runs in background)
//!     let election_clone = election.clone();
//!     tokio::spawn(async move {
//!         election_clone.start().await
//!     });
//!
//!     // Check leadership status
//!     if election.is_leader() {
//!         println!("This node is the leader");
//!     }
//!
//!     Ok(())
//! }
//! ```

pub mod leader;
pub mod lock;

pub use leader::{LeaderElection, LeaderElectionBuilder, LeaderError};
pub use lock::{DistributedLock, LockBuilder, LockError, LockGuard, RedisLock};

/// Lua script that only deletes the key if the caller's token still matches
/// the stored value, so a lock is never released out from under another
/// holder. Shared by [`lock::LockGuard`] release paths and by
/// [`leader::LeaderElection::resign`] (their scripts were byte-identical).
pub(crate) const RELEASE_SCRIPT: &str = r#"
    if redis.call("get", KEYS[1]) == ARGV[1] then
        return redis.call("del", KEYS[1])
    else
        return 0
    end
"#;

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_exports() {
        // Ensure module compiles
    }
}
