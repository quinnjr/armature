//! Worker Load Balancing
//!
//! This module provides load balancing strategies for distributing incoming
//! connections across worker threads.
//!
//! # Strategies
//!
//! - **Round Robin**: Simple rotation through workers
//! - **Least Connections**: Route to worker with fewest active connections
//! - **Weighted**: Proportional distribution based on worker capacity
//! - **Random**: Random selection for even distribution
//! - **Power of Two**: Random choice between two, pick least loaded
//!
//! # Performance
//!
//! - Round Robin: O(1), no contention
//! - Least Connections: O(n) workers, atomic reads
//! - Power of Two: O(1), near-optimal load distribution

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

// ============================================================================
// Load Balancing Strategy
// ============================================================================

/// Load balancing strategy for worker selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoadBalanceStrategy {
    /// Simple round-robin rotation
    #[default]
    RoundRobin,
    /// Route to worker with fewest connections
    LeastConnections,
    /// Weighted distribution based on capacity
    Weighted,
    /// Random selection
    Random,
    /// Power of Two Choices (pick 2 random, choose least loaded)
    PowerOfTwo,
    /// Sticky: hash-based routing for session affinity.
    ///
    /// Only honored via [`LoadBalancer::select_sticky`], which takes the
    /// hash key explicitly. The key-less [`LoadBalancer::select`] entry
    /// point has no key to hash on and falls back to round-robin (logging
    /// a warning) when this strategy is configured.
    Sticky,
}

// ============================================================================
// Worker State
// ============================================================================

/// State tracked for each worker.
#[derive(Debug)]
pub struct WorkerState {
    /// Worker ID
    pub id: usize,
    /// Current active connections
    connections: AtomicU32,
    /// Total connections handled
    total_handled: AtomicU64,
    /// Weight for weighted balancing (higher = more traffic)
    weight: u32,
    /// Is worker healthy/available
    healthy: AtomicU32,
    /// Pending requests in queue
    pending: AtomicU32,
    /// Average response time (microseconds)
    avg_response_us: AtomicU64,
}

impl WorkerState {
    /// Create new worker state.
    pub fn new(id: usize) -> Self {
        Self {
            id,
            connections: AtomicU32::new(0),
            total_handled: AtomicU64::new(0),
            weight: 1,
            healthy: AtomicU32::new(1),
            pending: AtomicU32::new(0),
            avg_response_us: AtomicU64::new(0),
        }
    }

    /// Create with weight.
    pub fn with_weight(id: usize, weight: u32) -> Self {
        Self {
            id,
            connections: AtomicU32::new(0),
            total_handled: AtomicU64::new(0),
            weight,
            healthy: AtomicU32::new(1),
            pending: AtomicU32::new(0),
            avg_response_us: AtomicU64::new(0),
        }
    }

    /// Get current connections.
    #[inline]
    pub fn connections(&self) -> u32 {
        self.connections.load(Ordering::Relaxed)
    }

    /// Increment connections.
    #[inline]
    pub fn add_connection(&self) {
        self.connections.fetch_add(1, Ordering::Relaxed);
        LB_STATS.record_connection_added();
    }

    /// Decrement connections.
    #[inline]
    pub fn remove_connection(&self) {
        self.connections.fetch_sub(1, Ordering::Relaxed);
        self.total_handled.fetch_add(1, Ordering::Relaxed);
        LB_STATS.record_connection_removed();
    }

    /// Get total handled.
    #[inline]
    pub fn total_handled(&self) -> u64 {
        self.total_handled.load(Ordering::Relaxed)
    }

    /// Get weight.
    #[inline]
    pub fn weight(&self) -> u32 {
        self.weight
    }

    /// Check if healthy.
    #[inline]
    pub fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed) != 0
    }

    /// Set healthy status.
    #[inline]
    pub fn set_healthy(&self, healthy: bool) {
        self.healthy
            .store(if healthy { 1 } else { 0 }, Ordering::Relaxed);
    }

    /// Get pending requests.
    #[inline]
    pub fn pending(&self) -> u32 {
        self.pending.load(Ordering::Relaxed)
    }

    /// Add pending request.
    #[inline]
    pub fn add_pending(&self) {
        self.pending.fetch_add(1, Ordering::Relaxed);
    }

    /// Remove pending request.
    #[inline]
    pub fn remove_pending(&self) {
        self.pending.fetch_sub(1, Ordering::Relaxed);
    }

    /// Get average response time.
    #[inline]
    pub fn avg_response_us(&self) -> u64 {
        self.avg_response_us.load(Ordering::Relaxed)
    }

    /// Update average response time (exponential moving average).
    #[inline]
    pub fn record_response_time(&self, us: u64) {
        let current = self.avg_response_us.load(Ordering::Relaxed);
        // EMA with alpha = 0.1
        let new_avg = if current == 0 {
            us
        } else {
            (current * 9 + us) / 10
        };
        self.avg_response_us.store(new_avg, Ordering::Relaxed);
    }

    /// Get load score (lower is better).
    #[inline]
    pub fn load_score(&self) -> u64 {
        // Combine connections and pending, weighted by response time
        let conn = self.connections() as u64;
        let pend = self.pending() as u64;
        let rt = self.avg_response_us().max(1);
        (conn + pend) * rt / 1000
    }
}

// ============================================================================
// Load Balancer
// ============================================================================

/// Load balancer for distributing work across workers.
#[derive(Debug)]
pub struct LoadBalancer {
    /// Worker states
    workers: Vec<Arc<WorkerState>>,
    /// Current strategy
    strategy: LoadBalanceStrategy,
    /// Round-robin counter
    rr_counter: AtomicUsize,
    /// Weighted round-robin state
    weighted_counter: AtomicUsize,
    /// Total weight (for weighted)
    total_weight: u32,
    /// Random state for power-of-two
    random_state: AtomicU64,
}

impl LoadBalancer {
    /// Create new load balancer.
    pub fn new(worker_count: usize, strategy: LoadBalanceStrategy) -> Self {
        let workers: Vec<_> = (0..worker_count)
            .map(|id| Arc::new(WorkerState::new(id)))
            .collect();

        Self {
            workers,
            strategy,
            rr_counter: AtomicUsize::new(0),
            weighted_counter: AtomicUsize::new(0),
            total_weight: worker_count as u32,
            random_state: AtomicU64::new(0x853c49e6748fea9b), // Random seed
        }
    }

    /// Create with custom weights.
    pub fn with_weights(weights: &[u32], strategy: LoadBalanceStrategy) -> Self {
        let workers: Vec<_> = weights
            .iter()
            .enumerate()
            .map(|(id, &w)| Arc::new(WorkerState::with_weight(id, w)))
            .collect();

        let total_weight = weights.iter().sum();

        Self {
            workers,
            strategy,
            rr_counter: AtomicUsize::new(0),
            weighted_counter: AtomicUsize::new(0),
            total_weight,
            random_state: AtomicU64::new(0x853c49e6748fea9b),
        }
    }

    /// Get number of workers.
    #[inline]
    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    /// Get current strategy.
    #[inline]
    pub fn strategy(&self) -> LoadBalanceStrategy {
        self.strategy
    }

    /// Set strategy.
    pub fn set_strategy(&mut self, strategy: LoadBalanceStrategy) {
        self.strategy = strategy;
    }

    /// Get worker state by ID.
    #[inline]
    pub fn worker(&self, id: usize) -> Option<&Arc<WorkerState>> {
        self.workers.get(id)
    }

    /// Select next worker based on strategy.
    ///
    /// # `Sticky` limitation
    ///
    /// This method takes no key, so it **cannot honor session affinity**
    /// for [`LoadBalanceStrategy::Sticky`]: there is nothing here to hash
    /// on. When the configured strategy is `Sticky`, this falls back to
    /// plain round-robin and logs a `tracing::warn!` on every call. Callers
    /// that need real hash-based sticky routing must call
    /// [`LoadBalancer::select_sticky`] with an explicit key (e.g. a
    /// connection or session id) instead of this method.
    #[inline]
    pub fn select(&self) -> usize {
        match self.strategy {
            LoadBalanceStrategy::RoundRobin => self.select_round_robin(),
            LoadBalanceStrategy::LeastConnections => self.select_least_connections(),
            LoadBalanceStrategy::Weighted => self.select_weighted(),
            LoadBalanceStrategy::Random => self.select_random(),
            LoadBalanceStrategy::PowerOfTwo => self.select_power_of_two(),
            LoadBalanceStrategy::Sticky => {
                tracing::warn!(
                    "LoadBalancer::select() cannot honor Sticky session affinity: \
                     no key is available at this call site. Falling back to \
                     round-robin; call select_sticky(key) instead for real \
                     hash-based routing."
                );
                self.select_round_robin()
            }
        }
    }

    /// Select with sticky hash.
    ///
    /// Falls back to healthy-aware round-robin if the hashed worker is
    /// unhealthy.
    #[inline]
    pub fn select_sticky(&self, key: u64) -> usize {
        let idx = (key % self.workers.len() as u64) as usize;
        if self.workers[idx].is_healthy() {
            LB_STATS.record_selection(self.strategy);
            idx
        } else {
            // Fallback to round-robin (skips unhealthy workers)
            self.select_round_robin()
        }
    }

    /// Get indices of healthy workers.
    fn healthy_indices(&self) -> Vec<usize> {
        self.workers
            .iter()
            .enumerate()
            .filter(|(_, w)| w.is_healthy())
            .map(|(idx, _)| idx)
            .collect()
    }

    /// Round-robin selection (healthy workers only; falls back to all
    /// workers if none are healthy).
    #[inline]
    fn select_round_robin(&self) -> usize {
        let n = self.workers.len();
        let start = self.rr_counter.fetch_add(1, Ordering::Relaxed) % n;
        LB_STATS.record_selection(LoadBalanceStrategy::RoundRobin);

        for offset in 0..n {
            let idx = (start + offset) % n;
            if self.workers[idx].is_healthy() {
                return idx;
            }
        }

        // No healthy workers - fall back to plain rotation
        start
    }

    /// Least connections selection (healthy workers only; falls back to all
    /// workers if none are healthy).
    fn select_least_connections(&self) -> usize {
        LB_STATS.record_selection(LoadBalanceStrategy::LeastConnections);

        let mut min_conn = u32::MAX;
        let mut selected = None;

        for (idx, worker) in self.workers.iter().enumerate() {
            if !worker.is_healthy() {
                continue;
            }
            let conn = worker.connections();
            if conn < min_conn {
                min_conn = conn;
                selected = Some(idx);
            }
        }

        if let Some(idx) = selected {
            return idx;
        }

        // No healthy workers - fall back to least connections across all
        let mut min_conn = u32::MAX;
        let mut selected = 0;
        for (idx, worker) in self.workers.iter().enumerate() {
            let conn = worker.connections();
            if conn < min_conn {
                min_conn = conn;
                selected = idx;
            }
        }
        selected
    }

    /// Weighted round-robin selection (healthy workers only; falls back to
    /// all workers if none are healthy).
    fn select_weighted(&self) -> usize {
        LB_STATS.record_selection(LoadBalanceStrategy::Weighted);
        let counter = self.weighted_counter.fetch_add(1, Ordering::Relaxed);

        let healthy_weight: u32 = self
            .workers
            .iter()
            .filter(|w| w.is_healthy())
            .map(|w| w.weight())
            .sum();

        if healthy_weight > 0 {
            let position = (counter as u32) % healthy_weight;
            let mut cumulative = 0u32;
            for (idx, worker) in self.workers.iter().enumerate() {
                if !worker.is_healthy() {
                    continue;
                }
                cumulative += worker.weight();
                if position < cumulative {
                    return idx;
                }
            }
        }

        // No healthy workers - fall back to weighted rotation across all
        let position = (counter as u32) % self.total_weight.max(1);
        let mut cumulative = 0u32;
        for (idx, worker) in self.workers.iter().enumerate() {
            cumulative += worker.weight();
            if position < cumulative {
                return idx;
            }
        }
        0
    }

    /// Random selection (healthy workers only; falls back to all workers if
    /// none are healthy).
    #[inline]
    fn select_random(&self) -> usize {
        LB_STATS.record_selection(LoadBalanceStrategy::Random);
        let healthy = self.healthy_indices();
        if healthy.is_empty() {
            self.next_random() as usize % self.workers.len()
        } else {
            healthy[self.next_random() as usize % healthy.len()]
        }
    }

    /// Power of Two Choices selection (healthy workers only; falls back to
    /// all workers if none are healthy).
    fn select_power_of_two(&self) -> usize {
        LB_STATS.record_selection(LoadBalanceStrategy::PowerOfTwo);

        let healthy = self.healthy_indices();
        let all: Vec<usize>;
        let candidates: &[usize] = if healthy.is_empty() {
            all = (0..self.workers.len()).collect();
            &all
        } else {
            &healthy
        };

        let n = candidates.len();
        if n <= 1 {
            return candidates.first().copied().unwrap_or(0);
        }

        // Pick two random candidates
        let pos1 = self.next_random() as usize % n;
        let mut pos2 = self.next_random() as usize % n;

        // Ensure different candidates
        if pos2 == pos1 {
            pos2 = (pos2 + 1) % n;
        }

        let idx1 = candidates[pos1];
        let idx2 = candidates[pos2];

        // Choose the one with fewer connections
        let conn1 = self.workers[idx1].connections();
        let conn2 = self.workers[idx2].connections();

        if conn1 <= conn2 { idx1 } else { idx2 }
    }

    /// Generate next random number (xorshift64).
    ///
    /// Uses an atomic read-modify-write so concurrent callers each observe
    /// a distinct state transition.
    #[inline]
    fn next_random(&self) -> u64 {
        let mut next = 0u64;
        let _ = self
            .random_state
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |state| {
                let mut x = state;
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                next = x;
                Some(x)
            });
        next
    }

    /// Get total connections across all workers.
    pub fn total_connections(&self) -> u64 {
        self.workers.iter().map(|w| w.connections() as u64).sum()
    }

    /// Get total handled across all workers.
    pub fn total_handled(&self) -> u64 {
        self.workers.iter().map(|w| w.total_handled()).sum()
    }

    /// Get connection distribution (for monitoring).
    pub fn distribution(&self) -> Vec<(usize, u32)> {
        self.workers
            .iter()
            .map(|w| (w.id, w.connections()))
            .collect()
    }

    /// Get load balance quality score (0.0 = perfectly balanced, 1.0 = worst).
    pub fn balance_score(&self) -> f64 {
        if self.workers.is_empty() {
            return 0.0;
        }

        let conns: Vec<f64> = self
            .workers
            .iter()
            .map(|w| w.connections() as f64)
            .collect();
        let mean = conns.iter().sum::<f64>() / conns.len() as f64;

        if mean == 0.0 {
            return 0.0;
        }

        // Coefficient of variation
        let variance = conns.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / conns.len() as f64;
        (variance.sqrt() / mean).min(1.0)
    }

    /// Mark worker as unhealthy.
    pub fn mark_unhealthy(&self, worker_id: usize) {
        if let Some(worker) = self.workers.get(worker_id) {
            worker.set_healthy(false);
            LB_STATS.record_health_change(false);
        }
    }

    /// Mark worker as healthy.
    pub fn mark_healthy(&self, worker_id: usize) {
        if let Some(worker) = self.workers.get(worker_id) {
            worker.set_healthy(true);
            LB_STATS.record_health_change(true);
        }
    }

    /// Get healthy worker count.
    pub fn healthy_count(&self) -> usize {
        self.workers.iter().filter(|w| w.is_healthy()).count()
    }
}

// ============================================================================
// Connection Guard
// ============================================================================

/// RAII guard for tracking connections.
///
/// Automatically decrements connection count when dropped.
pub struct ConnectionGuard {
    worker: Arc<WorkerState>,
}

impl ConnectionGuard {
    /// Create new connection guard.
    pub fn new(lb: &LoadBalancer) -> Self {
        let worker_id = lb.select();
        let worker = Arc::clone(&lb.workers[worker_id]);
        worker.add_connection();
        Self { worker }
    }

    /// Create for specific worker.
    pub fn for_worker(lb: &LoadBalancer, worker_id: usize) -> Option<Self> {
        lb.workers.get(worker_id).map(|worker| {
            let worker = Arc::clone(worker);
            worker.add_connection();
            Self { worker }
        })
    }

    /// Get worker ID.
    #[inline]
    pub fn worker_id(&self) -> usize {
        self.worker.id
    }

    /// Record response time.
    #[inline]
    pub fn record_response_time(&self, us: u64) {
        self.worker.record_response_time(us);
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.worker.remove_connection();
    }
}

// ============================================================================
// Statistics
// ============================================================================

/// Global load balancer statistics.
#[derive(Debug, Default)]
pub struct LoadBalancerStats {
    /// Round-robin selections
    rr_selections: AtomicU64,
    /// Least-connections selections
    lc_selections: AtomicU64,
    /// Weighted selections
    weighted_selections: AtomicU64,
    /// Random selections
    random_selections: AtomicU64,
    /// Power-of-two selections
    p2_selections: AtomicU64,
    /// Sticky selections
    sticky_selections: AtomicU64,
    /// Connections added
    connections_added: AtomicU64,
    /// Connections removed
    connections_removed: AtomicU64,
    /// Health changes (true = healthy)
    health_healthy: AtomicU64,
    /// Health changes (false = unhealthy)
    health_unhealthy: AtomicU64,
}

impl LoadBalancerStats {
    fn record_selection(&self, strategy: LoadBalanceStrategy) {
        match strategy {
            LoadBalanceStrategy::RoundRobin => {
                self.rr_selections.fetch_add(1, Ordering::Relaxed);
            }
            LoadBalanceStrategy::LeastConnections => {
                self.lc_selections.fetch_add(1, Ordering::Relaxed);
            }
            LoadBalanceStrategy::Weighted => {
                self.weighted_selections.fetch_add(1, Ordering::Relaxed);
            }
            LoadBalanceStrategy::Random => {
                self.random_selections.fetch_add(1, Ordering::Relaxed);
            }
            LoadBalanceStrategy::PowerOfTwo => {
                self.p2_selections.fetch_add(1, Ordering::Relaxed);
            }
            LoadBalanceStrategy::Sticky => {
                self.sticky_selections.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn record_connection_added(&self) {
        self.connections_added.fetch_add(1, Ordering::Relaxed);
    }

    fn record_connection_removed(&self) {
        self.connections_removed.fetch_add(1, Ordering::Relaxed);
    }

    fn record_health_change(&self, healthy: bool) {
        if healthy {
            self.health_healthy.fetch_add(1, Ordering::Relaxed);
        } else {
            self.health_unhealthy.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Get round-robin selections.
    pub fn rr_selections(&self) -> u64 {
        self.rr_selections.load(Ordering::Relaxed)
    }

    /// Get least-connections selections.
    pub fn lc_selections(&self) -> u64 {
        self.lc_selections.load(Ordering::Relaxed)
    }

    /// Get weighted selections.
    pub fn weighted_selections(&self) -> u64 {
        self.weighted_selections.load(Ordering::Relaxed)
    }

    /// Get random selections.
    pub fn random_selections(&self) -> u64 {
        self.random_selections.load(Ordering::Relaxed)
    }

    /// Get power-of-two selections.
    pub fn p2_selections(&self) -> u64 {
        self.p2_selections.load(Ordering::Relaxed)
    }

    /// Get sticky selections.
    pub fn sticky_selections(&self) -> u64 {
        self.sticky_selections.load(Ordering::Relaxed)
    }

    /// Get total selections.
    pub fn total_selections(&self) -> u64 {
        self.rr_selections()
            + self.lc_selections()
            + self.weighted_selections()
            + self.random_selections()
            + self.p2_selections()
            + self.sticky_selections()
    }

    /// Get connections added.
    pub fn connections_added(&self) -> u64 {
        self.connections_added.load(Ordering::Relaxed)
    }

    /// Get connections removed.
    pub fn connections_removed(&self) -> u64 {
        self.connections_removed.load(Ordering::Relaxed)
    }

    /// Get active connections.
    pub fn active_connections(&self) -> i64 {
        self.connections_added() as i64 - self.connections_removed() as i64
    }
}

/// Global statistics.
static LB_STATS: LoadBalancerStats = LoadBalancerStats {
    rr_selections: AtomicU64::new(0),
    lc_selections: AtomicU64::new(0),
    weighted_selections: AtomicU64::new(0),
    random_selections: AtomicU64::new(0),
    p2_selections: AtomicU64::new(0),
    sticky_selections: AtomicU64::new(0),
    connections_added: AtomicU64::new(0),
    connections_removed: AtomicU64::new(0),
    health_healthy: AtomicU64::new(0),
    health_unhealthy: AtomicU64::new(0),
};

/// Get global load balancer statistics.
pub fn lb_stats() -> &'static LoadBalancerStats {
    &LB_STATS
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_state() {
        let worker = WorkerState::new(0);
        assert_eq!(worker.id, 0);
        assert_eq!(worker.connections(), 0);
        assert!(worker.is_healthy());

        worker.add_connection();
        assert_eq!(worker.connections(), 1);

        worker.remove_connection();
        assert_eq!(worker.connections(), 0);
        assert_eq!(worker.total_handled(), 1);
    }

    #[test]
    fn test_worker_health() {
        let worker = WorkerState::new(0);
        assert!(worker.is_healthy());

        worker.set_healthy(false);
        assert!(!worker.is_healthy());

        worker.set_healthy(true);
        assert!(worker.is_healthy());
    }

    #[test]
    fn test_worker_response_time() {
        let worker = WorkerState::new(0);
        assert_eq!(worker.avg_response_us(), 0);

        worker.record_response_time(100);
        assert_eq!(worker.avg_response_us(), 100);

        // EMA should smooth
        worker.record_response_time(200);
        let avg = worker.avg_response_us();
        assert!(avg > 100 && avg < 200);
    }

    #[test]
    fn test_round_robin() {
        let lb = LoadBalancer::new(4, LoadBalanceStrategy::RoundRobin);

        assert_eq!(lb.select(), 0);
        assert_eq!(lb.select(), 1);
        assert_eq!(lb.select(), 2);
        assert_eq!(lb.select(), 3);
        assert_eq!(lb.select(), 0); // Wraps around
    }

    #[test]
    fn test_least_connections() {
        let lb = LoadBalancer::new(3, LoadBalanceStrategy::LeastConnections);

        // Initially all have 0, should pick first
        let first = lb.select();

        // Add connection to first
        lb.workers[first].add_connection();

        // Next should pick different worker
        let second = lb.select();
        assert_ne!(second, first);
    }

    #[test]
    fn test_weighted() {
        let lb = LoadBalancer::with_weights(&[1, 2, 1], LoadBalanceStrategy::Weighted);

        let mut counts = [0u32; 3];
        for _ in 0..400 {
            let idx = lb.select();
            counts[idx] += 1;
        }

        // Worker 1 (weight 2) should get ~2x the selections
        // Allow 20% variance
        assert!(
            counts[1] > counts[0],
            "Worker 1 should get more than worker 0"
        );
        assert!(
            counts[1] > counts[2],
            "Worker 1 should get more than worker 2"
        );
    }

    #[test]
    fn test_random() {
        let lb = LoadBalancer::new(4, LoadBalanceStrategy::Random);

        let mut counts = [0u32; 4];
        for _ in 0..1000 {
            let idx = lb.select();
            counts[idx] += 1;
        }

        // All should get some selections
        for count in counts {
            assert!(count > 0, "All workers should get some selections");
        }
    }

    #[test]
    fn test_power_of_two() {
        let lb = LoadBalancer::new(4, LoadBalanceStrategy::PowerOfTwo);

        // Add many connections to worker 0
        for _ in 0..10 {
            lb.workers[0].add_connection();
        }

        let mut counts = [0u32; 4];
        for _ in 0..100 {
            let idx = lb.select();
            counts[idx] += 1;
        }

        // Worker 0 should get fewer selections
        assert!(
            counts[0] < counts[1] + counts[2] + counts[3],
            "Loaded worker should get fewer selections"
        );
    }

    #[test]
    fn test_sticky() {
        let lb = LoadBalancer::new(4, LoadBalanceStrategy::Sticky);

        // Same key should always go to same worker
        let key = 12345u64;
        let first = lb.select_sticky(key);

        for _ in 0..10 {
            assert_eq!(lb.select_sticky(key), first);
        }

        // Different key may go to different worker
        let other = lb.select_sticky(99999u64);
        // Just verify it returns valid index
        assert!(other < 4);
    }

    #[test]
    fn test_connection_guard() {
        let lb = LoadBalancer::new(2, LoadBalanceStrategy::RoundRobin);

        {
            let guard = ConnectionGuard::new(&lb);
            assert_eq!(lb.total_connections(), 1);
            let _ = guard.worker_id();
        }

        // Connection removed after guard dropped
        assert_eq!(lb.total_connections(), 0);
    }

    #[test]
    fn test_unhealthy_worker() {
        let lb = LoadBalancer::new(3, LoadBalanceStrategy::LeastConnections);

        // Mark worker 0 as unhealthy
        lb.mark_unhealthy(0);
        assert!(!lb.workers[0].is_healthy());

        // Should never select unhealthy worker
        for _ in 0..100 {
            let idx = lb.select();
            assert_ne!(idx, 0, "Should not select unhealthy worker");
        }

        // Mark healthy again
        lb.mark_healthy(0);
        assert!(lb.workers[0].is_healthy());
    }

    #[test]
    fn test_unhealthy_worker_all_strategies() {
        // Regression: RoundRobin/Random/PowerOfTwo/Weighted used to ignore
        // the healthy flag entirely (or fall back to worker 0 blindly).
        for strategy in [
            LoadBalanceStrategy::RoundRobin,
            LoadBalanceStrategy::LeastConnections,
            LoadBalanceStrategy::Weighted,
            LoadBalanceStrategy::Random,
            LoadBalanceStrategy::PowerOfTwo,
        ] {
            let lb = LoadBalancer::new(4, strategy);
            lb.mark_unhealthy(0);
            lb.mark_unhealthy(2);

            for _ in 0..200 {
                let idx = lb.select();
                assert!(
                    idx == 1 || idx == 3,
                    "{:?} selected unhealthy worker {}",
                    strategy,
                    idx
                );
            }
        }
    }

    #[test]
    fn test_sticky_avoids_unhealthy() {
        let lb = LoadBalancer::new(4, LoadBalanceStrategy::Sticky);

        // Key that maps to worker 1
        let key = 1u64;
        assert_eq!(lb.select_sticky(key), 1);

        lb.mark_unhealthy(1);
        for _ in 0..100 {
            let idx = lb.select_sticky(key);
            assert_ne!(idx, 1, "Sticky fallback returned the unhealthy worker");
            assert!(idx < 4);
        }
    }

    #[test]
    fn test_all_unhealthy_falls_back_to_all_workers() {
        for strategy in [
            LoadBalanceStrategy::RoundRobin,
            LoadBalanceStrategy::LeastConnections,
            LoadBalanceStrategy::Weighted,
            LoadBalanceStrategy::Random,
            LoadBalanceStrategy::PowerOfTwo,
        ] {
            let lb = LoadBalancer::new(3, strategy);
            for id in 0..3 {
                lb.mark_unhealthy(id);
            }

            // Still returns a valid index instead of panicking or pinning 0
            for _ in 0..50 {
                let idx = lb.select();
                assert!(idx < 3, "{:?} returned invalid index {}", strategy, idx);
            }
        }

        let lb = LoadBalancer::new(3, LoadBalanceStrategy::Sticky);
        for id in 0..3 {
            lb.mark_unhealthy(id);
        }
        assert!(lb.select_sticky(42) < 3);
    }

    #[test]
    fn test_next_random_concurrent_uniqueness() {
        // Regression: next_random was a non-atomic load/store, so concurrent
        // callers observed identical values. With an atomic update, every
        // successful step advances the xorshift sequence, so all drawn
        // values are distinct.
        let lb = Arc::new(LoadBalancer::new(4, LoadBalanceStrategy::Random));

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let lb = Arc::clone(&lb);
                std::thread::spawn(move || {
                    (0..1000).map(|_| lb.next_random()).collect::<Vec<u64>>()
                })
            })
            .collect();

        let mut seen = std::collections::HashSet::new();
        for handle in handles {
            for value in handle.join().unwrap() {
                assert!(seen.insert(value), "duplicate random value observed");
            }
        }
        assert_eq!(seen.len(), 4000);
    }

    #[test]
    fn test_balance_score() {
        let lb = LoadBalancer::new(4, LoadBalanceStrategy::RoundRobin);

        // Initially perfectly balanced (all 0)
        assert_eq!(lb.balance_score(), 0.0);

        // Add uneven connections
        lb.workers[0].add_connection();
        lb.workers[0].add_connection();
        lb.workers[0].add_connection();
        lb.workers[1].add_connection();

        let score = lb.balance_score();
        assert!(score > 0.0, "Unbalanced should have positive score");
        assert!(score <= 1.0, "Score should be <= 1.0");
    }

    #[test]
    fn test_distribution() {
        let lb = LoadBalancer::new(3, LoadBalanceStrategy::RoundRobin);

        lb.workers[0].add_connection();
        lb.workers[1].add_connection();
        lb.workers[1].add_connection();

        let dist = lb.distribution();
        assert_eq!(dist.len(), 3);
        assert_eq!(dist[0], (0, 1));
        assert_eq!(dist[1], (1, 2));
        assert_eq!(dist[2], (2, 0));
    }

    #[test]
    fn test_lb_stats() {
        let stats = lb_stats();
        let _ = stats.total_selections();
        let _ = stats.active_connections();
    }
}
