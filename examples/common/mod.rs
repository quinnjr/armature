#![allow(dead_code)]

//! Common utilities for Armature examples.
//!
//! This module provides shared functionality used across multiple examples.

use rand::RngExt;
use std::net::{SocketAddr, TcpListener};

/// Port range for examples (50000-50999)
pub const PORT_RANGE_START: u16 = 50000;
pub const PORT_RANGE_END: u16 = 50999;

/// Find an available port in the 50000 range.
///
/// This function randomly selects ports in the range 50000-50999 until it finds
/// one that is available for binding.
///
/// # Returns
///
/// An available port number in the 50000 range.
///
/// # Panics
///
/// Panics if no available port can be found after 100 attempts.
///
/// # Example
///
/// ```ignore
/// use examples::common::find_available_port;
///
/// let port = find_available_port();
/// println!("Using port: {}", port);
/// ```
pub fn find_available_port() -> u16 {
    let mut rng = rand::rng();

    for _ in 0..100 {
        let port = rng.random_range(PORT_RANGE_START..=PORT_RANGE_END);
        let addr: SocketAddr = ([0, 0, 0, 0], port).into();

        if TcpListener::bind(addr).is_ok() {
            return port;
        }
    }

    // Fallback: try sequential ports
    for port in PORT_RANGE_START..=PORT_RANGE_END {
        let addr: SocketAddr = ([0, 0, 0, 0], port).into();
        if TcpListener::bind(addr).is_ok() {
            return port;
        }
    }

    panic!(
        "Could not find an available port in range {}-{}",
        PORT_RANGE_START, PORT_RANGE_END
    );
}

/// Check if a specific port is available.
///
/// # Arguments
///
/// * `port` - The port number to check.
///
/// # Returns
///
/// `true` if the port is available, `false` otherwise.
pub fn is_port_available(port: u16) -> bool {
    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    TcpListener::bind(addr).is_ok()
}
