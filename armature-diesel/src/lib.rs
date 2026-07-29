//! # Armature Diesel
//!
//! Async Diesel database integration for the Armature framework.
//!
//! This crate provides connection pooling and transaction management for
//! Diesel's async API.
//!
//! ## Features
//!
//! - **Async Connection Pools**: Built on `diesel-async` with `deadpool` or `bb8`
//! - **Multiple Backends**: PostgreSQL and MySQL support
//! - **Transaction Management**: Easy-to-use transaction helpers
//! - **Connection Health**: Automatic connection validation
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature_diesel::{DieselPool, DieselConfig};
//!
//! // Create configuration
//! let config = DieselConfig::new("postgres://user:pass@localhost/db")
//!     .pool_size(10)
//!     .connect_timeout(Duration::from_secs(5));
//!
//! // Create pool
//! let pool = DieselPool::new(config).await?;
//!
//! // Get connection
//! let mut conn = pool.get().await?;
//!
//! // Run query
//! let users = users::table.load::<User>(&mut conn).await?;
//! ```
//!
//! ## With Transactions
//!
//! ```rust,ignore
//! use armature_diesel::TransactionExt;
//!
//! pool.transaction(async |conn| {
//!     diesel::insert_into(users::table)
//!         .values(&new_user)
//!         .execute(conn)
//!         .await?;
//!
//!     diesel::insert_into(profiles::table)
//!         .values(&new_profile)
//!         .execute(conn)
//!         .await?;
//!
//!     Ok(())
//! }).await?;
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]

mod config;
mod error;
mod pool;
mod transaction;

pub use config::*;
pub use error::*;
pub use pool::*;
pub use transaction::*;

// Re-export diesel types for convenience
pub use diesel;
pub use diesel_async;

#[cfg(feature = "deadpool")]
pub use diesel_async::pooled_connection::deadpool;

#[cfg(feature = "bb8")]
pub use diesel_async::pooled_connection::bb8;

#[cfg(feature = "mobc")]
pub use diesel_async::pooled_connection::mobc;
