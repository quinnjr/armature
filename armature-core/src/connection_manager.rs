//! Connection Manager - Dynamic Connection and Buffer Tuning
//!
//! Provides intelligent management of connections and buffers based on traffic:
//!
//! - **Buffer Size Auto-Tuning**: Dynamically adjusts buffer sizes based on traffic
//! - **Adaptive Keep-Alive**: Adjusts keep-alive behavior based on server load
//! - **Idle Connection Culling**: Drops idle connections under memory pressure
//!
//! # Usage
//!
//! ```rust,ignore
//! use armature_core::connection_manager::{ConnectionManager, ConnectionManagerConfig};
//!
//! // Create manager
//! let config = ConnectionManagerConfig::default();
//! let manager = ConnectionManager::new(config);
//!
//! // Register connections
//! let conn_id = manager.register_connection();
//!
//! // Mark activity
//! manager.mark_active(conn_id);
//!
//! // Get optimized buffer size
//! let buf_size = manager.recommended_buffer_size();
//!
//! // Periodic maintenance (call from background task)
//! manager.maintain();
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for the connection manager.
#[derive(Debug, Clone)]
pub struct ConnectionManagerConfig {
    // Buffer tuning
    /// Minimum buffer size (bytes)
    pub min_buffer_size: usize,
    /// Maximum buffer size (bytes)
    pub max_buffer_size: usize,
    /// Initial buffer size (bytes)
    pub initial_buffer_size: usize,
    /// How often to adjust buffer sizes
    pub buffer_adjust_interval: Duration,
    /// History window for buffer size decisions
    pub buffer_history_window: Duration,

    // Keep-alive tuning
    /// Base keep-alive timeout
    pub base_keep_alive_timeout: Duration,
    /// Minimum keep-alive timeout under load
    pub min_keep_alive_timeout: Duration,
    /// Maximum keep-alive timeout when idle
    pub max_keep_alive_timeout: Duration,
    /// Load threshold to start reducing keep-alive (0.0-1.0)
    pub keep_alive_load_threshold: f64,

    // Idle connection culling
    /// Enable idle connection culling
    pub enable_culling: bool,
    /// Connection idle timeout before eligible for culling
    pub idle_timeout: Duration,
    /// Memory pressure threshold to start culling (0.0-1.0)
    pub cull_pressure_threshold: f64,
    /// Maximum connections before culling starts
    pub max_connections: usize,
    /// Minimum connections to keep even under pressure
    pub min_connections: usize,
    /// How many connections to cull per maintenance cycle
    pub cull_batch_size: usize,
}

impl Default for ConnectionManagerConfig {
    fn default() -> Self {
        Self {
            // Buffer tuning
            min_buffer_size: 256,
            max_buffer_size: 64 * 1024,
            initial_buffer_size: 4 * 1024,
            buffer_adjust_interval: Duration::from_secs(10),
            buffer_history_window: Duration::from_secs(60),

            // Keep-alive tuning
            base_keep_alive_timeout: Duration::from_secs(60),
            min_keep_alive_timeout: Duration::from_secs(5),
            max_keep_alive_timeout: Duration::from_secs(120),
            keep_alive_load_threshold: 0.7,

            // Idle connection culling
            enable_culling: true,
            idle_timeout: Duration::from_secs(30),
            cull_pressure_threshold: 0.8,
            max_connections: 10_000,
            min_connections: 100,
            cull_batch_size: 100,
        }
    }
}

impl ConnectionManagerConfig {
    /// Create config optimized for high throughput.
    pub fn high_throughput() -> Self {
        Self {
            min_buffer_size: 4 * 1024,
            max_buffer_size: 256 * 1024,
            initial_buffer_size: 16 * 1024,
            buffer_adjust_interval: Duration::from_secs(5),
            buffer_history_window: Duration::from_secs(30),

            base_keep_alive_timeout: Duration::from_secs(120),
            min_keep_alive_timeout: Duration::from_secs(10),
            max_keep_alive_timeout: Duration::from_secs(300),
            keep_alive_load_threshold: 0.85,

            enable_culling: true,
            idle_timeout: Duration::from_secs(60),
            cull_pressure_threshold: 0.9,
            max_connections: 50_000,
            min_connections: 500,
            cull_batch_size: 200,
        }
    }

    /// Create config optimized for low memory.
    pub fn low_memory() -> Self {
        Self {
            min_buffer_size: 256,
            max_buffer_size: 16 * 1024,
            initial_buffer_size: 1024,
            buffer_adjust_interval: Duration::from_secs(5),
            buffer_history_window: Duration::from_secs(30),

            base_keep_alive_timeout: Duration::from_secs(30),
            min_keep_alive_timeout: Duration::from_secs(5),
            max_keep_alive_timeout: Duration::from_secs(60),
            keep_alive_load_threshold: 0.5,

            enable_culling: true,
            idle_timeout: Duration::from_secs(15),
            cull_pressure_threshold: 0.6,
            max_connections: 1000,
            min_connections: 50,
            cull_batch_size: 50,
        }
    }

    /// Create config for development/testing.
    pub fn development() -> Self {
        Self {
            enable_culling: false,
            max_connections: 100,
            ..Default::default()
        }
    }
}

// ============================================================================
// Connection State
// ============================================================================

/// Unique connection identifier.
pub type ConnectionId = u64;

/// State tracked for each connection.
#[derive(Debug)]
struct ConnectionState {
    #[allow(dead_code)] // Reserved for connection identification
    id: ConnectionId,
    #[allow(dead_code)] // Reserved for connection lifetime tracking
    created_at: Instant,
    last_active: Instant,
    bytes_read: u64,
    bytes_written: u64,
    requests: u64,
    is_keep_alive: bool,
}

impl ConnectionState {
    fn new(id: ConnectionId) -> Self {
        let now = Instant::now();
        Self {
            id,
            created_at: now,
            last_active: now,
            bytes_read: 0,
            bytes_written: 0,
            requests: 0,
            is_keep_alive: false,
        }
    }

    fn idle_duration(&self) -> Duration {
        self.last_active.elapsed()
    }
}

// ============================================================================
// Buffer Size History
// ============================================================================

/// Historical data point for buffer sizing.
#[derive(Debug, Clone, Copy)]
struct BufferSample {
    timestamp: Instant,
    size: usize,
    was_sufficient: bool,
}

/// Tracks buffer usage patterns.
#[derive(Debug)]
struct BufferHistory {
    samples: Vec<BufferSample>,
    window: Duration,
    optimal_size: AtomicUsize,
}

impl BufferHistory {
    /// Maximum samples to retain (prevents unbounded growth under burst traffic)
    const MAX_SAMPLES: usize = 1000;

    fn new(window: Duration, initial_size: usize) -> Self {
        Self {
            samples: Vec::with_capacity(1000),
            window,
            optimal_size: AtomicUsize::new(initial_size),
        }
    }

    fn record(&mut self, size: usize, was_sufficient: bool) {
        let now = Instant::now();

        // Prune old samples by time
        self.samples
            .retain(|s| now.duration_since(s.timestamp) < self.window);

        // Prune by count if over capacity (prevents unbounded growth under burst)
        while self.samples.len() >= Self::MAX_SAMPLES {
            self.samples.remove(0);
        }

        // Add new sample
        self.samples.push(BufferSample {
            timestamp: now,
            size,
            was_sufficient,
        });
    }

    fn compute_optimal_size(&self, min: usize, max: usize) -> usize {
        if self.samples.is_empty() {
            return self.optimal_size.load(Ordering::Relaxed);
        }

        // Find p95 of successful buffer sizes
        let mut successful: Vec<usize> = self
            .samples
            .iter()
            .filter(|s| s.was_sufficient)
            .map(|s| s.size)
            .collect();

        if successful.is_empty() {
            // If no successful samples, increase size
            let current = self.optimal_size.load(Ordering::Relaxed);
            return (current * 2).min(max);
        }

        successful.sort_unstable();
        let p95_idx = (successful.len() * 95 / 100).min(successful.len() - 1);
        let p95 = successful[p95_idx];

        // Round up to power of 2
        let size = p95.next_power_of_two().clamp(min, max);
        self.optimal_size.store(size, Ordering::Relaxed);
        size
    }

    fn get_optimal_size(&self) -> usize {
        self.optimal_size.load(Ordering::Relaxed)
    }
}

// ============================================================================
// Load Tracker
// ============================================================================

/// Tracks server load for adaptive decisions.
#[derive(Debug)]
struct LoadTracker {
    /// Current active connections
    active_connections: AtomicUsize,
    /// Peak connections in last minute
    peak_connections: AtomicUsize,
    /// Requests per second (rolling average)
    requests_per_second: AtomicU64,
    /// Last RPS calculation time
    last_rps_update: Mutex<Instant>,
    /// Request count since last RPS update
    request_count: AtomicU64,
    /// Memory pressure estimate (0-100)
    memory_pressure: AtomicUsize,
}

impl Default for LoadTracker {
    fn default() -> Self {
        Self {
            active_connections: AtomicUsize::new(0),
            peak_connections: AtomicUsize::new(0),
            requests_per_second: AtomicU64::new(0),
            last_rps_update: Mutex::new(Instant::now()),
            request_count: AtomicU64::new(0),
            memory_pressure: AtomicUsize::new(0),
        }
    }
}

impl LoadTracker {
    fn connection_opened(&self) {
        let count = self.active_connections.fetch_add(1, Ordering::Relaxed) + 1;
        self.peak_connections.fetch_max(count, Ordering::Relaxed);
    }

    fn connection_closed(&self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }

    fn record_request(&self) {
        self.request_count.fetch_add(1, Ordering::Relaxed);
    }

    fn update_rps(&self) {
        let mut last_update = self.last_rps_update.lock().unwrap();
        let elapsed = last_update.elapsed();

        if elapsed >= Duration::from_secs(1) {
            let count = self.request_count.swap(0, Ordering::Relaxed);
            let rps = (count as f64 / elapsed.as_secs_f64()) as u64;
            self.requests_per_second.store(rps, Ordering::Relaxed);
            *last_update = Instant::now();
        }
    }

    fn set_memory_pressure(&self, pressure: usize) {
        self.memory_pressure
            .store(pressure.min(100), Ordering::Relaxed);
    }

    fn load_factor(&self, max_connections: usize) -> f64 {
        let active = self.active_connections.load(Ordering::Relaxed) as f64;
        let max = max_connections as f64;
        (active / max).min(1.0)
    }

    fn memory_pressure_factor(&self) -> f64 {
        self.memory_pressure.load(Ordering::Relaxed) as f64 / 100.0
    }

    fn active_connections(&self) -> usize {
        self.active_connections.load(Ordering::Relaxed)
    }

    fn rps(&self) -> u64 {
        self.requests_per_second.load(Ordering::Relaxed)
    }
}

// ============================================================================
// Connection Manager
// ============================================================================

/// Statistics for the connection manager.
#[derive(Debug, Default)]
pub struct ConnectionManagerStats {
    /// Total connections opened
    pub connections_opened: AtomicU64,
    /// Total connections closed
    pub connections_closed: AtomicU64,
    /// Connections culled due to idle
    pub connections_culled: AtomicU64,
    /// Connections rejected due to limits
    pub connections_rejected: AtomicU64,
    /// Buffer size adjustments made
    pub buffer_adjustments: AtomicU64,
    /// Keep-alive timeouts adjusted
    pub keep_alive_adjustments: AtomicU64,
    /// Current recommended buffer size
    pub current_buffer_size: AtomicUsize,
    /// Current keep-alive timeout (ms)
    pub current_keep_alive_ms: AtomicU64,
}

/// Connection manager with dynamic tuning.
pub struct ConnectionManager {
    config: ConnectionManagerConfig,
    connections: RwLock<HashMap<ConnectionId, ConnectionState>>,
    next_id: AtomicU64,
    buffer_history: Mutex<BufferHistory>,
    load: LoadTracker,
    stats: ConnectionManagerStats,
    last_maintenance: Mutex<Instant>,
    shutdown: AtomicBool,
}

impl ConnectionManager {
    /// Create a new connection manager.
    pub fn new(config: ConnectionManagerConfig) -> Self {
        let buffer_history =
            BufferHistory::new(config.buffer_history_window, config.initial_buffer_size);

        Self {
            config,
            connections: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            buffer_history: Mutex::new(buffer_history),
            load: LoadTracker::default(),
            stats: ConnectionManagerStats::default(),
            last_maintenance: Mutex::new(Instant::now()),
            shutdown: AtomicBool::new(false),
        }
    }

    /// Create with default configuration.
    pub fn default_manager() -> Self {
        Self::new(ConnectionManagerConfig::default())
    }

    // ========================================================================
    // Connection Lifecycle
    // ========================================================================

    /// Register a new connection. Returns None if at capacity.
    pub fn register_connection(&self) -> Option<ConnectionId> {
        // Check capacity
        let current = self.load.active_connections();
        if current >= self.config.max_connections {
            self.stats
                .connections_rejected
                .fetch_add(1, Ordering::Relaxed);
            return None;
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let state = ConnectionState::new(id);

        {
            let mut connections = self.connections.write().unwrap();
            connections.insert(id, state);
        }

        self.load.connection_opened();
        self.stats
            .connections_opened
            .fetch_add(1, Ordering::Relaxed);

        Some(id)
    }

    /// Unregister a connection.
    pub fn unregister_connection(&self, id: ConnectionId) {
        let mut connections = self.connections.write().unwrap();
        if connections.remove(&id).is_some() {
            self.load.connection_closed();
            self.stats
                .connections_closed
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Mark a connection as active.
    pub fn mark_active(&self, id: ConnectionId) {
        if let Ok(mut connections) = self.connections.write()
            && let Some(state) = connections.get_mut(&id)
        {
            state.last_active = Instant::now();
            state.requests += 1;
        }
        self.load.record_request();
    }

    /// Record bytes read on a connection.
    pub fn record_bytes_read(&self, id: ConnectionId, bytes: u64) {
        if let Ok(mut connections) = self.connections.write()
            && let Some(state) = connections.get_mut(&id)
        {
            state.bytes_read += bytes;
        }
    }

    /// Record bytes written on a connection.
    pub fn record_bytes_written(&self, id: ConnectionId, bytes: u64) {
        if let Ok(mut connections) = self.connections.write()
            && let Some(state) = connections.get_mut(&id)
        {
            state.bytes_written += bytes;
        }
    }

    /// Set connection keep-alive status.
    pub fn set_keep_alive(&self, id: ConnectionId, keep_alive: bool) {
        if let Ok(mut connections) = self.connections.write()
            && let Some(state) = connections.get_mut(&id)
        {
            state.is_keep_alive = keep_alive;
        }
    }

    // ========================================================================
    // Buffer Size Auto-Tuning
    // ========================================================================

    /// Get the recommended buffer size based on traffic patterns.
    pub fn recommended_buffer_size(&self) -> usize {
        self.buffer_history.lock().unwrap().get_optimal_size()
    }

    /// Record a buffer usage sample.
    pub fn record_buffer_usage(&self, size: usize, was_sufficient: bool) {
        let mut history = self.buffer_history.lock().unwrap();
        history.record(size, was_sufficient);
    }

    /// Recalculate optimal buffer size.
    fn adjust_buffer_size(&self) {
        let history = self.buffer_history.lock().unwrap();
        let optimal =
            history.compute_optimal_size(self.config.min_buffer_size, self.config.max_buffer_size);

        let previous = self
            .stats
            .current_buffer_size
            .swap(optimal, Ordering::Relaxed);
        if previous != optimal {
            self.stats
                .buffer_adjustments
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    // ========================================================================
    // Adaptive Keep-Alive
    // ========================================================================

    /// Get the current keep-alive timeout based on server load.
    pub fn keep_alive_timeout(&self) -> Duration {
        let load = self.load.load_factor(self.config.max_connections);
        let memory_pressure = self.load.memory_pressure_factor();

        // Use the higher of connection load or memory pressure
        let pressure = load.max(memory_pressure);

        if pressure < self.config.keep_alive_load_threshold {
            // Under threshold - use base or even extended timeout
            let extension = (1.0 - pressure / self.config.keep_alive_load_threshold) * 0.5;
            let timeout_ms =
                self.config.base_keep_alive_timeout.as_millis() as f64 * (1.0 + extension);
            let max_ms = self.config.max_keep_alive_timeout.as_millis() as f64;
            Duration::from_millis(timeout_ms.min(max_ms) as u64)
        } else {
            // Over threshold - reduce timeout linearly
            let reduction = (pressure - self.config.keep_alive_load_threshold)
                / (1.0 - self.config.keep_alive_load_threshold);
            let base_ms = self.config.base_keep_alive_timeout.as_millis() as f64;
            let min_ms = self.config.min_keep_alive_timeout.as_millis() as f64;
            let timeout_ms = base_ms - (base_ms - min_ms) * reduction;
            Duration::from_millis(timeout_ms.max(min_ms) as u64)
        }
    }

    /// Check if keep-alive should be allowed for new connections.
    pub fn allow_keep_alive(&self) -> bool {
        let load = self.load.load_factor(self.config.max_connections);
        load < 0.95 // Allow keep-alive up to 95% capacity
    }

    // ========================================================================
    // Idle Connection Culling
    // ========================================================================

    /// Set current memory pressure (0-100).
    pub fn set_memory_pressure(&self, pressure: usize) {
        self.load.set_memory_pressure(pressure);
    }

    /// Cull idle connections under pressure.
    /// Returns the number of connections culled.
    pub fn cull_idle_connections(&self) -> usize {
        if !self.config.enable_culling {
            return 0;
        }

        let current = self.load.active_connections();
        if current <= self.config.min_connections {
            return 0;
        }

        let load = self.load.load_factor(self.config.max_connections);
        let memory_pressure = self.load.memory_pressure_factor();
        let pressure = load.max(memory_pressure);

        if pressure < self.config.cull_pressure_threshold {
            return 0;
        }

        // Find idle connections to cull
        let to_cull: Vec<ConnectionId> = {
            let connections = self.connections.read().unwrap();
            let mut idle_connections: Vec<_> = connections
                .iter()
                .filter(|(_, state)| state.idle_duration() >= self.config.idle_timeout)
                .map(|(id, state)| (*id, state.idle_duration()))
                .collect();

            // Sort by idle duration (longest first)
            idle_connections.sort_by_key(|k| std::cmp::Reverse(k.1));

            // Take up to batch size, but keep min connections
            let max_cull = (current - self.config.min_connections).min(self.config.cull_batch_size);

            idle_connections
                .into_iter()
                .take(max_cull)
                .map(|(id, _)| id)
                .collect()
        };

        // Remove culled connections
        let culled = to_cull.len();
        {
            let mut connections = self.connections.write().unwrap();
            for id in to_cull {
                connections.remove(&id);
                self.load.connection_closed();
            }
        }

        self.stats
            .connections_culled
            .fetch_add(culled as u64, Ordering::Relaxed);
        culled
    }

    /// Get list of connections that should be closed due to idle timeout.
    pub fn get_expired_connections(&self) -> Vec<ConnectionId> {
        let connections = self.connections.read().unwrap();
        let timeout = self.keep_alive_timeout();

        connections
            .iter()
            .filter(|(_, state)| state.idle_duration() >= timeout)
            .map(|(id, _)| *id)
            .collect()
    }

    // ========================================================================
    // Maintenance
    // ========================================================================

    /// Perform periodic maintenance.
    /// Call this from a background task every few seconds.
    pub fn maintain(&self) {
        if self.shutdown.load(Ordering::Relaxed) {
            return;
        }

        // Update RPS
        self.load.update_rps();

        // Adjust buffer size periodically
        {
            let mut last = self.last_maintenance.lock().unwrap();
            if last.elapsed() >= self.config.buffer_adjust_interval {
                self.adjust_buffer_size();
                *last = Instant::now();
            }
        }

        // Update keep-alive timeout stat
        let timeout = self.keep_alive_timeout();
        self.stats
            .current_keep_alive_ms
            .store(timeout.as_millis() as u64, Ordering::Relaxed);

        // Cull idle connections if under pressure
        self.cull_idle_connections();
    }

    /// Shutdown the manager.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    // ========================================================================
    // Statistics
    // ========================================================================

    /// Get current statistics.
    pub fn stats(&self) -> &ConnectionManagerStats {
        &self.stats
    }

    /// Get current active connection count.
    pub fn active_connections(&self) -> usize {
        self.load.active_connections()
    }

    /// Get current load factor (0.0-1.0).
    pub fn load_factor(&self) -> f64 {
        self.load.load_factor(self.config.max_connections)
    }

    /// Get requests per second.
    pub fn rps(&self) -> u64 {
        self.load.rps()
    }

    /// Get a snapshot of current state.
    pub fn snapshot(&self) -> ConnectionManagerSnapshot {
        ConnectionManagerSnapshot {
            active_connections: self.load.active_connections(),
            rps: self.load.rps(),
            load_factor: self.load_factor(),
            memory_pressure: self.load.memory_pressure_factor(),
            buffer_size: self.recommended_buffer_size(),
            keep_alive_timeout: self.keep_alive_timeout(),
            connections_opened: self.stats.connections_opened.load(Ordering::Relaxed),
            connections_closed: self.stats.connections_closed.load(Ordering::Relaxed),
            connections_culled: self.stats.connections_culled.load(Ordering::Relaxed),
        }
    }
}

/// Snapshot of connection manager state.
#[derive(Debug, Clone)]
pub struct ConnectionManagerSnapshot {
    pub active_connections: usize,
    pub rps: u64,
    pub load_factor: f64,
    pub memory_pressure: f64,
    pub buffer_size: usize,
    pub keep_alive_timeout: Duration,
    pub connections_opened: u64,
    pub connections_closed: u64,
    pub connections_culled: u64,
}

// ============================================================================
// Global Manager
// ============================================================================

use std::sync::OnceLock;

static GLOBAL_MANAGER: OnceLock<Arc<ConnectionManager>> = OnceLock::new();

/// Initialize the global connection manager.
pub fn init_global_manager(config: ConnectionManagerConfig) -> &'static Arc<ConnectionManager> {
    GLOBAL_MANAGER.get_or_init(|| Arc::new(ConnectionManager::new(config)))
}

/// Get the global connection manager.
pub fn global_manager() -> Option<&'static Arc<ConnectionManager>> {
    GLOBAL_MANAGER.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_lifecycle() {
        let manager = ConnectionManager::new(ConnectionManagerConfig::default());

        // Register connection
        let id = manager.register_connection().unwrap();
        assert_eq!(manager.active_connections(), 1);

        // Mark active
        manager.mark_active(id);
        manager.record_bytes_read(id, 1000);
        manager.record_bytes_written(id, 500);

        // Unregister
        manager.unregister_connection(id);
        assert_eq!(manager.active_connections(), 0);
    }

    #[test]
    fn test_buffer_tuning() {
        let manager = ConnectionManager::new(ConnectionManagerConfig::default());

        // Record some buffer usage
        for _ in 0..100 {
            manager.record_buffer_usage(1024, true);
        }

        // Should adjust based on samples
        manager.adjust_buffer_size();
        let size = manager.recommended_buffer_size();
        assert!(size >= 1024);
    }

    #[test]
    fn test_keep_alive_under_load() {
        let config = ConnectionManagerConfig {
            max_connections: 100,
            keep_alive_load_threshold: 0.5,
            ..Default::default()
        };
        let manager = ConnectionManager::new(config);

        // Low load - full timeout
        let timeout_low = manager.keep_alive_timeout();

        // Add connections to increase load
        for _ in 0..80 {
            manager.register_connection();
        }

        // High load - reduced timeout
        let timeout_high = manager.keep_alive_timeout();
        assert!(timeout_high < timeout_low);
    }

    #[test]
    fn test_idle_culling() {
        let config = ConnectionManagerConfig {
            enable_culling: true,
            idle_timeout: Duration::from_millis(1),
            cull_pressure_threshold: 0.5,
            max_connections: 100,
            min_connections: 10,
            cull_batch_size: 10,
            ..Default::default()
        };
        let manager = ConnectionManager::new(config);

        // Register 80 connections (80% load)
        for _ in 0..80 {
            manager.register_connection();
        }

        // Wait for them to become idle
        std::thread::sleep(Duration::from_millis(5));

        // Set memory pressure
        manager.set_memory_pressure(90);

        // Cull should remove some
        let culled = manager.cull_idle_connections();
        assert!(culled > 0);
        assert!(manager.active_connections() < 80);
    }

    #[test]
    fn test_connection_limit() {
        let config = ConnectionManagerConfig {
            max_connections: 10,
            ..Default::default()
        };
        let manager = ConnectionManager::new(config);

        // Register up to limit
        for _ in 0..10 {
            assert!(manager.register_connection().is_some());
        }

        // Next should fail
        assert!(manager.register_connection().is_none());

        // After unregistering, should work again
        manager.unregister_connection(1);
        assert!(manager.register_connection().is_some());
    }

    #[test]
    fn test_snapshot() {
        let manager = ConnectionManager::new(ConnectionManagerConfig::default());

        let id = manager.register_connection().unwrap();
        manager.mark_active(id);

        let snapshot = manager.snapshot();
        assert_eq!(snapshot.active_connections, 1);
        assert!(snapshot.buffer_size > 0);
    }
}
