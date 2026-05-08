//! Optimized Connection State Machine
//!
//! This module provides a high-performance connection state machine (FSM) with
//! minimized branching for HTTP connection handling. Key optimizations:
//!
//! - Branchless state transitions using lookup tables
//! - Compact state representation (single byte)
//! - Pre-computed action tables for state/event combinations
//! - Cache-friendly state layout
//!
//! ## State Machine
//!
//! ```text
//!                 ┌─────────────┐
//!                 │    Idle     │◄───────────────┐
//!                 └──────┬──────┘                │
//!                        │ accept               │
//!                        ▼                      │
//!                 ┌─────────────┐               │
//!             ┌───│  Connected  │───┐           │
//!             │   └──────┬──────┘   │           │
//!        error│          │ data     │ timeout   │
//!             │          ▼          │           │
//!             │   ┌─────────────┐   │           │
//!             │   │   Reading   │───┤           │
//!             │   └──────┬──────┘   │           │
//!             │          │ complete │           │
//!             │          ▼          │           │
//!             │   ┌─────────────┐   │           │
//!             │   │ Processing  │───┤           │
//!             │   └──────┬──────┘   │           │
//!             │          │ ready    │           │
//!             │          ▼          │           │
//!             │   ┌─────────────┐   │           │
//!             │   │   Writing   │───┤           │
//!             │   └──────┬──────┘   │           │
//!             │          │ done     │ keep-alive│
//!             │          ▼          └───────────┤
//!             │   ┌─────────────┐               │
//!             └──►│   Closing   │───────────────┘
//!                 └─────────────┘
//! ```
//!
//! ## Performance Characteristics
//!
//! - State transition: O(1) table lookup, no branching
//! - Event dispatch: Single array index, predicted by CPU
//! - Memory: 1 byte per connection state + 8 bytes metadata
//!
//! ## Usage
//!
//! ```rust,ignore
//! use armature_core::connection::{Connection, ConnectionEvent};
//!
//! let mut conn = Connection::new();
//!
//! // Process events - branchless transitions
//! conn.handle_event(ConnectionEvent::Accept);
//! conn.handle_event(ConnectionEvent::DataReady);
//! conn.handle_event(ConnectionEvent::RequestComplete);
//! conn.handle_event(ConnectionEvent::ResponseReady);
//! conn.handle_event(ConnectionEvent::WriteComplete);
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

// ============================================================================
// Connection State (Compact Representation)
// ============================================================================

/// Connection state as a single byte for cache efficiency.
///
/// Using `#[repr(u8)]` ensures the enum is exactly 1 byte,
/// enabling branchless table lookups via direct indexing.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConnectionState {
    /// Connection slot is idle/available
    #[default]
    Idle = 0,
    /// TCP connection established, awaiting first request
    Connected = 1,
    /// Reading request headers/body
    Reading = 2,
    /// Processing request (handler executing)
    Processing = 3,
    /// Writing response
    Writing = 4,
    /// Connection closing (graceful)
    Closing = 5,
    /// Connection closed
    Closed = 6,
    /// Error state
    Error = 7,
}

impl ConnectionState {
    /// Number of states (for table sizing).
    pub const COUNT: usize = 8;

    /// Convert from u8 (branchless).
    #[inline(always)]
    pub const fn from_u8(v: u8) -> Self {
        // SAFETY: All values 0-7 are valid states, clamp others to Error
        match v {
            0 => Self::Idle,
            1 => Self::Connected,
            2 => Self::Reading,
            3 => Self::Processing,
            4 => Self::Writing,
            5 => Self::Closing,
            6 => Self::Closed,
            7 => Self::Error,
            _ => Self::Error,
        }
    }

    /// Convert to u8 (zero-cost).
    #[inline(always)]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Check if terminal state.
    #[inline(always)]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Closed | Self::Error)
    }

    /// Check if can accept new request.
    #[inline(always)]
    pub const fn can_accept_request(self) -> bool {
        matches!(self, Self::Connected | Self::Idle)
    }

    /// Check if active (processing a request).
    #[inline(always)]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Reading | Self::Processing | Self::Writing)
    }
}

// ============================================================================
// Connection Events
// ============================================================================

/// Events that trigger state transitions.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionEvent {
    /// New connection accepted
    Accept = 0,
    /// Data available for reading
    DataReady = 1,
    /// Request fully read
    RequestComplete = 2,
    /// Response ready to send
    ResponseReady = 3,
    /// Write completed
    WriteComplete = 4,
    /// Keep-alive: ready for next request
    KeepAlive = 5,
    /// Timeout occurred
    Timeout = 6,
    /// Error occurred
    Error = 7,
    /// Close requested
    Close = 8,
}

impl ConnectionEvent {
    /// Number of events (for table sizing).
    pub const COUNT: usize = 9;

    /// Convert to u8.
    #[inline(always)]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

// ============================================================================
// Transition Action
// ============================================================================

/// Action to take during state transition.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionAction {
    /// No action needed
    None = 0,
    /// Start reading request
    StartRead = 1,
    /// Continue reading
    ContinueRead = 2,
    /// Dispatch to handler
    Dispatch = 3,
    /// Start writing response
    StartWrite = 4,
    /// Continue writing
    ContinueWrite = 5,
    /// Reset for keep-alive
    Reset = 6,
    /// Initiate close
    InitiateClose = 7,
    /// Force close (error)
    ForceClose = 8,
    /// Log error
    LogError = 9,
}

// ============================================================================
// Transition Table (Branchless Lookup)
// ============================================================================

/// Pre-computed state transition table.
///
/// Layout: `TRANSITION_TABLE[current_state][event] = (next_state, action)`
///
/// This eliminates branching by using array indexing instead of match statements.
/// The table is const and will be embedded in the binary.
const TRANSITION_TABLE: [[TransitionEntry; ConnectionEvent::COUNT]; ConnectionState::COUNT] = {
    use ConnectionState as S;
    use TransitionAction as A;

    // Helper to create entries
    const fn e(state: S, action: A) -> TransitionEntry {
        TransitionEntry {
            next_state: state.as_u8(),
            action: action as u8,
        }
    }

    // Default: stay in current state, no action
    const NOOP_IDLE: TransitionEntry = e(S::Idle, A::None);
    const NOOP_CONN: TransitionEntry = e(S::Connected, A::None);
    const NOOP_READ: TransitionEntry = e(S::Reading, A::None);
    const NOOP_PROC: TransitionEntry = e(S::Processing, A::None);
    const NOOP_WRIT: TransitionEntry = e(S::Writing, A::None);
    const NOOP_CLOS: TransitionEntry = e(S::Closing, A::None);
    const NOOP_CLSD: TransitionEntry = e(S::Closed, A::None);
    const NOOP_ERR: TransitionEntry = e(S::Error, A::None);

    // Error transitions
    const TO_ERROR: TransitionEntry = e(S::Error, A::ForceClose);
    const TO_CLOSE: TransitionEntry = e(S::Closing, A::InitiateClose);
    const TO_CLOSED: TransitionEntry = e(S::Closed, A::None);

    [
        // Idle state: waiting for connection
        // Events: Accept, DataReady, ReqComplete, RespReady, WriteComplete, KeepAlive, Timeout, Error, Close
        [
            e(S::Connected, A::StartRead), // Accept
            NOOP_IDLE,                     // DataReady (ignore)
            NOOP_IDLE,                     // RequestComplete (ignore)
            NOOP_IDLE,                     // ResponseReady (ignore)
            NOOP_IDLE,                     // WriteComplete (ignore)
            NOOP_IDLE,                     // KeepAlive (ignore)
            NOOP_IDLE,                     // Timeout (ignore)
            NOOP_IDLE,                     // Error (ignore)
            NOOP_IDLE,                     // Close (ignore)
        ],
        // Connected state: connection established
        [
            NOOP_CONN,                      // Accept (ignore)
            e(S::Reading, A::ContinueRead), // DataReady -> Reading
            NOOP_CONN,                      // RequestComplete (ignore)
            NOOP_CONN,                      // ResponseReady (ignore)
            NOOP_CONN,                      // WriteComplete (ignore)
            NOOP_CONN,                      // KeepAlive (ignore)
            TO_CLOSE,                       // Timeout -> Closing
            TO_ERROR,                       // Error -> Error
            TO_CLOSE,                       // Close -> Closing
        ],
        // Reading state: reading request
        [
            NOOP_READ,                      // Accept (ignore)
            e(S::Reading, A::ContinueRead), // DataReady -> continue reading
            e(S::Processing, A::Dispatch),  // RequestComplete -> Processing
            NOOP_READ,                      // ResponseReady (ignore)
            NOOP_READ,                      // WriteComplete (ignore)
            NOOP_READ,                      // KeepAlive (ignore)
            TO_CLOSE,                       // Timeout -> Closing
            TO_ERROR,                       // Error -> Error
            TO_CLOSE,                       // Close -> Closing
        ],
        // Processing state: executing handler
        [
            NOOP_PROC,                    // Accept (ignore)
            NOOP_PROC,                    // DataReady (ignore)
            NOOP_PROC,                    // RequestComplete (ignore)
            e(S::Writing, A::StartWrite), // ResponseReady -> Writing
            NOOP_PROC,                    // WriteComplete (ignore)
            NOOP_PROC,                    // KeepAlive (ignore)
            TO_CLOSE,                     // Timeout -> Closing
            TO_ERROR,                     // Error -> Error
            TO_CLOSE,                     // Close -> Closing
        ],
        // Writing state: writing response
        [
            NOOP_WRIT,                       // Accept (ignore)
            NOOP_WRIT,                       // DataReady (ignore)
            NOOP_WRIT,                       // RequestComplete (ignore)
            NOOP_WRIT,                       // ResponseReady (ignore)
            e(S::Closing, A::InitiateClose), // WriteComplete -> Closing (or KeepAlive)
            e(S::Connected, A::Reset),       // KeepAlive -> Connected
            TO_CLOSE,                        // Timeout -> Closing
            TO_ERROR,                        // Error -> Error
            TO_CLOSE,                        // Close -> Closing
        ],
        // Closing state: graceful shutdown
        [
            NOOP_CLOS, // Accept (ignore)
            NOOP_CLOS, // DataReady (ignore)
            NOOP_CLOS, // RequestComplete (ignore)
            NOOP_CLOS, // ResponseReady (ignore)
            TO_CLOSED, // WriteComplete -> Closed
            NOOP_CLOS, // KeepAlive (ignore)
            TO_CLOSED, // Timeout -> Closed
            TO_CLOSED, // Error -> Closed
            TO_CLOSED, // Close -> Closed
        ],
        // Closed state: terminal
        [
            NOOP_CLSD, NOOP_CLSD, NOOP_CLSD, NOOP_CLSD, NOOP_CLSD, NOOP_CLSD, NOOP_CLSD, NOOP_CLSD,
            NOOP_CLSD,
        ],
        // Error state: terminal
        [
            NOOP_ERR, NOOP_ERR, NOOP_ERR, NOOP_ERR, NOOP_ERR, NOOP_ERR, NOOP_ERR, NOOP_ERR,
            NOOP_ERR,
        ],
    ]
};

/// Packed transition entry (2 bytes).
#[derive(Clone, Copy)]
struct TransitionEntry {
    next_state: u8,
    action: u8,
}

impl TransitionEntry {
    #[inline(always)]
    fn state(self) -> ConnectionState {
        ConnectionState::from_u8(self.next_state)
    }

    #[inline(always)]
    fn action(self) -> TransitionAction {
        // SAFETY: action values are always valid TransitionAction discriminants
        unsafe { std::mem::transmute(self.action) }
    }
}

// ============================================================================
// Connection State Machine
// ============================================================================

/// High-performance connection state machine.
///
/// Optimized for minimal branching and cache efficiency.
#[derive(Debug)]
pub struct Connection {
    /// Current state (1 byte)
    state: ConnectionState,
    /// Keep-alive enabled
    keep_alive: bool,
    /// Request count on this connection
    request_count: u32,
    /// Connection established time
    connected_at: Option<Instant>,
    /// Last activity time
    last_activity: Option<Instant>,
    /// Connection ID (for logging/debugging)
    id: u64,
}

impl Connection {
    /// Create a new connection in idle state.
    #[inline]
    pub fn new() -> Self {
        Self {
            state: ConnectionState::Idle,
            keep_alive: true,
            request_count: 0,
            connected_at: None,
            last_activity: None,
            id: CONNECTION_ID_COUNTER.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Create with specific ID.
    #[inline]
    pub fn with_id(id: u64) -> Self {
        Self {
            state: ConnectionState::Idle,
            keep_alive: true,
            request_count: 0,
            connected_at: None,
            last_activity: None,
            id,
        }
    }

    /// Get current state.
    #[inline(always)]
    pub fn state(&self) -> ConnectionState {
        self.state
    }

    /// Get connection ID.
    #[inline(always)]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Get request count.
    #[inline(always)]
    pub fn request_count(&self) -> u32 {
        self.request_count
    }

    /// Check if keep-alive is enabled.
    #[inline(always)]
    pub fn keep_alive(&self) -> bool {
        self.keep_alive
    }

    /// Set keep-alive.
    #[inline(always)]
    pub fn set_keep_alive(&mut self, enabled: bool) {
        self.keep_alive = enabled;
    }

    /// Get time since connection established.
    #[inline]
    pub fn age(&self) -> Option<Duration> {
        self.connected_at.map(|t| t.elapsed())
    }

    /// Get time since last activity.
    #[inline]
    pub fn idle_time(&self) -> Option<Duration> {
        self.last_activity.map(|t| t.elapsed())
    }

    /// Handle an event with branchless state transition.
    ///
    /// Returns the action to perform.
    #[inline]
    pub fn handle_event(&mut self, event: ConnectionEvent) -> TransitionAction {
        // Branchless table lookup
        let entry = TRANSITION_TABLE[self.state.as_u8() as usize][event.as_u8() as usize];

        // Update state
        let new_state = entry.state();
        let action = entry.action();

        // Track state changes
        if new_state != self.state {
            self.on_state_change(new_state);
        }

        self.state = new_state;
        self.last_activity = Some(Instant::now());

        action
    }

    /// Try transition with validation.
    ///
    /// Returns `Ok(action)` if transition is valid, `Err` otherwise.
    #[inline]
    pub fn try_transition(
        &mut self,
        event: ConnectionEvent,
    ) -> Result<TransitionAction, TransitionError> {
        let entry = TRANSITION_TABLE[self.state.as_u8() as usize][event.as_u8() as usize];
        let new_state = entry.state();
        let action = entry.action();

        // Check if this is a meaningful transition
        if new_state == self.state && action == TransitionAction::None {
            return Err(TransitionError::InvalidTransition {
                from: self.state,
                event,
            });
        }

        if new_state != self.state {
            self.on_state_change(new_state);
        }

        self.state = new_state;
        self.last_activity = Some(Instant::now());

        Ok(action)
    }

    /// Called on state change.
    #[inline]
    fn on_state_change(&mut self, new_state: ConnectionState) {
        match new_state {
            ConnectionState::Connected => {
                self.connected_at = Some(Instant::now());
                CONNECTION_STATS.record_connected();
            }
            ConnectionState::Reading => {
                // New request starting
            }
            ConnectionState::Processing => {
                self.request_count += 1;
            }
            ConnectionState::Closed => {
                CONNECTION_STATS.record_closed();
            }
            ConnectionState::Error => {
                CONNECTION_STATS.record_error();
            }
            _ => {}
        }
    }

    /// Reset connection for reuse (keep-alive).
    #[inline]
    pub fn reset(&mut self) {
        self.state = ConnectionState::Connected;
        self.last_activity = Some(Instant::now());
        CONNECTION_STATS.record_reuse();
    }

    /// Force close the connection.
    #[inline]
    pub fn force_close(&mut self) {
        if !self.state.is_terminal() {
            self.state = ConnectionState::Closed;
            CONNECTION_STATS.record_closed();
        }
    }

    /// Check if connection should be closed (idle timeout).
    #[inline]
    pub fn should_close(&self, idle_timeout: Duration) -> bool {
        if self.state.is_terminal() {
            return true;
        }

        if let Some(idle_time) = self.idle_time()
            && idle_time > idle_timeout
        {
            return true;
        }

        false
    }
}

impl Default for Connection {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Recyclable Trait
// ============================================================================

/// Trait for objects that can be reset and reused.
///
/// Implementing this trait allows objects to be efficiently recycled
/// in a pool, avoiding allocation overhead.
pub trait Recyclable {
    /// Reset the object to its initial state for reuse.
    ///
    /// This should clear all request-specific data while preserving
    /// pooled resources like buffers.
    fn reset(&mut self);

    /// Check if the object is clean and ready for reuse.
    fn is_clean(&self) -> bool;

    /// Get the generation counter (for use-after-recycle detection).
    fn generation(&self) -> u64;

    /// Increment the generation counter.
    fn increment_generation(&mut self);
}

// ============================================================================
// Recyclable Connection
// ============================================================================

/// Extended connection with full recycling support.
#[derive(Debug)]
pub struct RecyclableConnection {
    /// Base connection state machine
    inner: Connection,
    /// Generation counter for use-after-recycle detection
    generation: u64,
    /// Total recycle count for this slot
    recycle_count: u64,
    /// Pre-allocated read buffer capacity
    read_buffer_capacity: usize,
    /// Pre-allocated write buffer capacity
    write_buffer_capacity: usize,
    /// Custom user data slot (type-erased)
    user_data: Option<Box<dyn std::any::Any + Send + Sync>>,
}

impl RecyclableConnection {
    /// Create a new recyclable connection.
    pub fn new() -> Self {
        Self {
            inner: Connection::new(),
            generation: 0,
            recycle_count: 0,
            read_buffer_capacity: 0,
            write_buffer_capacity: 0,
            user_data: None,
        }
    }

    /// Create with specific ID.
    pub fn with_id(id: u64) -> Self {
        Self {
            inner: Connection::with_id(id),
            generation: 0,
            recycle_count: 0,
            read_buffer_capacity: 0,
            write_buffer_capacity: 0,
            user_data: None,
        }
    }

    /// Create with buffer capacity hints.
    pub fn with_capacities(read_capacity: usize, write_capacity: usize) -> Self {
        Self {
            inner: Connection::new(),
            generation: 0,
            recycle_count: 0,
            read_buffer_capacity: read_capacity,
            write_buffer_capacity: write_capacity,
            user_data: None,
        }
    }

    /// Get the inner connection.
    #[inline(always)]
    pub fn inner(&self) -> &Connection {
        &self.inner
    }

    /// Get mutable inner connection.
    #[inline(always)]
    pub fn inner_mut(&mut self) -> &mut Connection {
        &mut self.inner
    }

    /// Get total recycle count.
    #[inline(always)]
    pub fn recycle_count(&self) -> u64 {
        self.recycle_count
    }

    /// Get read buffer capacity hint.
    #[inline(always)]
    pub fn read_buffer_capacity(&self) -> usize {
        self.read_buffer_capacity
    }

    /// Get write buffer capacity hint.
    #[inline(always)]
    pub fn write_buffer_capacity(&self) -> usize {
        self.write_buffer_capacity
    }

    /// Set buffer capacity hints for next use.
    #[inline]
    pub fn set_buffer_capacities(&mut self, read: usize, write: usize) {
        self.read_buffer_capacity = read;
        self.write_buffer_capacity = write;
    }

    /// Set user data.
    pub fn set_user_data<T: std::any::Any + Send + Sync + 'static>(&mut self, data: T) {
        self.user_data = Some(Box::new(data));
    }

    /// Get user data.
    pub fn user_data<T: std::any::Any + Send + Sync + 'static>(&self) -> Option<&T> {
        self.user_data.as_ref()?.downcast_ref::<T>()
    }

    /// Take user data.
    pub fn take_user_data<T: std::any::Any + Send + Sync + 'static>(&mut self) -> Option<T> {
        let boxed = self.user_data.take()?;
        boxed.downcast::<T>().ok().map(|b| *b)
    }

    /// Handle event (delegates to inner).
    #[inline]
    pub fn handle_event(&mut self, event: ConnectionEvent) -> TransitionAction {
        self.inner.handle_event(event)
    }

    /// Get state (delegates to inner).
    #[inline(always)]
    pub fn state(&self) -> ConnectionState {
        self.inner.state()
    }

    /// Get connection ID (delegates to inner).
    #[inline(always)]
    pub fn id(&self) -> u64 {
        self.inner.id()
    }

    /// Prepare for recycling - clean up and return to pool.
    pub fn prepare_for_recycle(&mut self) {
        // Clear user data
        self.user_data = None;
        // Reset inner connection
        self.inner.state = ConnectionState::Idle;
        self.inner.keep_alive = true;
        self.inner.request_count = 0;
        self.inner.connected_at = None;
        self.inner.last_activity = None;
        // Increment counters
        self.generation += 1;
        self.recycle_count += 1;
        // Record stats
        RECYCLE_STATS.record_recycle();
    }
}

impl Default for RecyclableConnection {
    fn default() -> Self {
        Self::new()
    }
}

impl Recyclable for RecyclableConnection {
    fn reset(&mut self) {
        self.prepare_for_recycle();
    }

    fn is_clean(&self) -> bool {
        self.inner.state() == ConnectionState::Idle && self.user_data.is_none()
    }

    fn generation(&self) -> u64 {
        self.generation
    }

    fn increment_generation(&mut self) {
        self.generation += 1;
    }
}

// ============================================================================
// Generic Recycle Pool
// ============================================================================

/// Generic object pool with recycling support.
///
/// This pool maintains a set of pre-allocated objects that can be
/// acquired, used, and returned for reuse.
#[derive(Debug)]
pub struct RecyclePool<T: Recyclable + Default> {
    /// Pooled objects
    objects: Vec<T>,
    /// Free list (indices of available objects)
    free_indices: Vec<usize>,
    /// Pool capacity
    capacity: usize,
    /// High water mark (max concurrent usage)
    high_water_mark: usize,
    /// Configuration
    #[allow(dead_code)] // Reserved for future dynamic pool management
    config: RecyclePoolConfig,
}

impl<T: Recyclable + Default> RecyclePool<T> {
    /// Create a new pool with capacity.
    pub fn new(capacity: usize) -> Self {
        Self::with_config(capacity, RecyclePoolConfig::default())
    }

    /// Create with custom configuration.
    pub fn with_config(capacity: usize, config: RecyclePoolConfig) -> Self {
        let mut objects = Vec::with_capacity(capacity);
        let mut free_indices = Vec::with_capacity(capacity);

        for i in 0..capacity {
            objects.push(T::default());
            free_indices.push(i);
        }

        Self {
            objects,
            free_indices,
            capacity,
            high_water_mark: 0,
            config,
        }
    }

    /// Acquire an object from the pool.
    ///
    /// Returns a handle that automatically returns the object when dropped.
    #[inline]
    pub fn acquire(&mut self) -> Option<PoolHandle<'_, T>> {
        let index = self.free_indices.pop()?;
        let obj = &mut self.objects[index];

        // Ensure object is clean
        if !obj.is_clean() {
            obj.reset();
        }

        // Track high water mark
        let active = self.capacity - self.free_indices.len();
        if active > self.high_water_mark {
            self.high_water_mark = active;
        }

        RECYCLE_STATS.record_acquire();

        Some(PoolHandle {
            pool: self,
            index,
            released: false,
            _marker: std::marker::PhantomData,
        })
    }

    /// Try to acquire without blocking.
    #[inline]
    pub fn try_acquire(&mut self) -> Option<PoolHandle<'_, T>> {
        self.acquire()
    }

    /// Release an object back to the pool.
    #[inline]
    fn release(&mut self, index: usize) {
        if index < self.capacity {
            // Reset object for reuse
            self.objects[index].reset();
            self.free_indices.push(index);
            RECYCLE_STATS.record_release();
        }
    }

    /// Get object by index (for internal use).
    #[inline]
    pub fn get(&self, index: usize) -> Option<&T> {
        self.objects.get(index)
    }

    /// Get mutable object by index.
    #[inline]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.objects.get_mut(index)
    }

    /// Get pool capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get available count.
    #[inline]
    pub fn available(&self) -> usize {
        self.free_indices.len()
    }

    /// Get active count.
    #[inline]
    pub fn active(&self) -> usize {
        self.capacity - self.free_indices.len()
    }

    /// Get high water mark.
    #[inline]
    pub fn high_water_mark(&self) -> usize {
        self.high_water_mark
    }

    /// Reset high water mark.
    #[inline]
    pub fn reset_high_water_mark(&mut self) {
        self.high_water_mark = self.active();
    }

    /// Check if pool is exhausted.
    #[inline]
    pub fn is_exhausted(&self) -> bool {
        self.free_indices.is_empty()
    }

    /// Shrink pool to minimum size.
    pub fn shrink(&mut self, min_capacity: usize) {
        let target = min_capacity.max(self.active());
        if target < self.capacity {
            // Only shrink if we have excess free slots
            while self.capacity > target && !self.free_indices.is_empty() {
                if let Some(index) = self.free_indices.pop() {
                    // Mark as removed (but keep in vec to avoid reindexing)
                    self.objects[index].reset();
                }
                self.capacity -= 1;
            }
        }
    }

    /// Grow pool capacity.
    pub fn grow(&mut self, additional: usize) {
        let new_capacity = self.capacity + additional;
        self.objects.reserve(additional);

        for _ in 0..additional {
            let index = self.objects.len();
            self.objects.push(T::default());
            self.free_indices.push(index);
        }

        self.capacity = new_capacity;
    }
}

/// Pool handle - RAII wrapper for acquired objects.
#[derive(Debug)]
pub struct PoolHandle<'a, T: Recyclable + Default> {
    pool: *mut RecyclePool<T>,
    index: usize,
    released: bool,
    _marker: std::marker::PhantomData<&'a mut T>,
}

impl<'a, T: Recyclable + Default> PoolHandle<'a, T> {
    /// Get reference to the object.
    #[inline]
    pub fn get(&self) -> &T {
        unsafe { &(&(*self.pool).objects)[self.index] }
    }

    /// Get mutable reference.
    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        unsafe { &mut (&mut (*self.pool).objects)[self.index] }
    }

    /// Get the pool index.
    #[inline]
    pub fn index(&self) -> usize {
        self.index
    }

    /// Get the generation of the object.
    #[inline]
    pub fn generation(&self) -> u64 {
        self.get().generation()
    }

    /// Release back to pool explicitly.
    #[inline]
    pub fn release(mut self) {
        if !self.released {
            unsafe { (*self.pool).release(self.index) };
            self.released = true;
        }
    }
}

impl<'a, T: Recyclable + Default> Drop for PoolHandle<'a, T> {
    fn drop(&mut self) {
        if !self.released {
            unsafe { (*self.pool).release(self.index) };
        }
    }
}

impl<'a, T: Recyclable + Default> std::ops::Deref for PoolHandle<'a, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<'a, T: Recyclable + Default> std::ops::DerefMut for PoolHandle<'a, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut()
    }
}

// ============================================================================
// Recycle Pool Configuration
// ============================================================================

/// Configuration for recycle pools.
#[derive(Debug, Clone)]
pub struct RecyclePoolConfig {
    /// Initial capacity
    pub initial_capacity: usize,
    /// Maximum capacity (0 = unlimited)
    pub max_capacity: usize,
    /// Grow by this amount when exhausted
    pub grow_by: usize,
    /// Shrink when usage drops below this fraction
    pub shrink_threshold: f32,
    /// Minimum capacity to maintain
    pub min_capacity: usize,
}

impl Default for RecyclePoolConfig {
    fn default() -> Self {
        Self {
            initial_capacity: 100,
            max_capacity: 10000,
            grow_by: 50,
            shrink_threshold: 0.25,
            min_capacity: 10,
        }
    }
}

impl RecyclePoolConfig {
    /// Create new configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set initial capacity.
    pub fn initial_capacity(mut self, capacity: usize) -> Self {
        self.initial_capacity = capacity;
        self
    }

    /// Set maximum capacity.
    pub fn max_capacity(mut self, max: usize) -> Self {
        self.max_capacity = max;
        self
    }

    /// Set grow-by amount.
    pub fn grow_by(mut self, amount: usize) -> Self {
        self.grow_by = amount;
        self
    }

    /// Set shrink threshold.
    pub fn shrink_threshold(mut self, threshold: f32) -> Self {
        self.shrink_threshold = threshold;
        self
    }

    /// Set minimum capacity.
    pub fn min_capacity(mut self, min: usize) -> Self {
        self.min_capacity = min;
        self
    }
}

// ============================================================================
// Recycling Statistics
// ============================================================================

/// Statistics for object recycling.
#[derive(Debug, Default)]
pub struct RecycleStats {
    /// Total acquires
    acquires: AtomicU64,
    /// Total releases
    releases: AtomicU64,
    /// Total recycles (resets)
    recycles: AtomicU64,
    /// Total allocations (when pool exhausted)
    allocations: AtomicU64,
}

impl RecycleStats {
    /// Create new stats.
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    fn record_acquire(&self) {
        self.acquires.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn record_release(&self) {
        self.releases.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn record_recycle(&self) {
        self.recycles.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    #[allow(dead_code)] // Reserved for future pool allocation tracking
    fn record_allocation(&self) {
        self.allocations.fetch_add(1, Ordering::Relaxed);
    }

    /// Get acquire count.
    pub fn acquires(&self) -> u64 {
        self.acquires.load(Ordering::Relaxed)
    }

    /// Get release count.
    pub fn releases(&self) -> u64 {
        self.releases.load(Ordering::Relaxed)
    }

    /// Get recycle count.
    pub fn recycles(&self) -> u64 {
        self.recycles.load(Ordering::Relaxed)
    }

    /// Get allocation count.
    pub fn allocations(&self) -> u64 {
        self.allocations.load(Ordering::Relaxed)
    }

    /// Get recycle ratio (recycles / acquires).
    pub fn recycle_ratio(&self) -> f64 {
        let acquires = self.acquires() as f64;
        if acquires > 0.0 {
            self.recycles() as f64 / acquires
        } else {
            0.0
        }
    }

    /// Get hit ratio (1 - allocations/acquires).
    pub fn hit_ratio(&self) -> f64 {
        let acquires = self.acquires() as f64;
        if acquires > 0.0 {
            1.0 - (self.allocations() as f64 / acquires)
        } else {
            1.0
        }
    }
}

/// Global recycle statistics.
static RECYCLE_STATS: RecycleStats = RecycleStats {
    acquires: AtomicU64::new(0),
    releases: AtomicU64::new(0),
    recycles: AtomicU64::new(0),
    allocations: AtomicU64::new(0),
};

/// Get global recycle statistics.
pub fn recycle_stats() -> &'static RecycleStats {
    &RECYCLE_STATS
}

// ============================================================================
// Connection Recycler
// ============================================================================

/// Specialized connection recycler with optimizations.
pub struct ConnectionRecycler {
    /// Pool of recyclable connections
    pool: RecyclePool<RecyclableConnection>,
    /// Configuration
    config: ConnectionConfig,
}

impl ConnectionRecycler {
    /// Create a new connection recycler.
    pub fn new(capacity: usize) -> Self {
        Self::with_config(capacity, ConnectionConfig::default())
    }

    /// Create with custom configuration.
    pub fn with_config(capacity: usize, config: ConnectionConfig) -> Self {
        let mut pool: RecyclePool<RecyclableConnection> = RecyclePool::new(capacity);

        // Pre-configure all connections
        for i in 0..capacity {
            if let Some(conn) = pool.get_mut(i) {
                conn.inner_mut().set_keep_alive(config.keep_alive);
            }
        }

        Self { pool, config }
    }

    /// Acquire a connection.
    #[inline]
    pub fn acquire(&mut self) -> Option<PoolHandle<'_, RecyclableConnection>> {
        let handle = self.pool.acquire()?;
        Some(handle)
    }

    /// Get pool statistics.
    pub fn stats(&self) -> RecyclerStats {
        RecyclerStats {
            capacity: self.pool.capacity(),
            available: self.pool.available(),
            active: self.pool.active(),
            high_water_mark: self.pool.high_water_mark(),
        }
    }

    /// Get configuration.
    pub fn config(&self) -> &ConnectionConfig {
        &self.config
    }

    /// Check if a connection should be recycled based on limits.
    pub fn should_recycle(&self, conn: &RecyclableConnection) -> bool {
        // Check max requests
        if conn.inner().request_count() >= self.config.max_requests {
            return false;
        }

        // Check keep-alive
        if !conn.inner().keep_alive() {
            return false;
        }

        // Check age
        if let Some(age) = conn.inner().age()
            && age > self.config.idle_timeout * 10
        {
            return false;
        }

        true
    }
}

/// Connection recycler statistics.
#[derive(Debug, Clone, Copy)]
pub struct RecyclerStats {
    /// Pool capacity
    pub capacity: usize,
    /// Available connections
    pub available: usize,
    /// Active connections
    pub active: usize,
    /// High water mark
    pub high_water_mark: usize,
}

// ============================================================================
// Transition Error
// ============================================================================

/// Error during state transition.
#[derive(Debug, Clone)]
pub enum TransitionError {
    /// Invalid transition for current state
    InvalidTransition {
        from: ConnectionState,
        event: ConnectionEvent,
    },
    /// Connection already closed
    AlreadyClosed,
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTransition { from, event } => {
                write!(f, "Invalid transition: {:?} + {:?}", from, event)
            }
            Self::AlreadyClosed => write!(f, "Connection already closed"),
        }
    }
}

impl std::error::Error for TransitionError {}

// ============================================================================
// Connection Pool
// ============================================================================

/// Pre-allocated connection pool for reduced allocation overhead.
#[derive(Debug)]
pub struct ConnectionPool {
    /// Pooled connections
    connections: Vec<Connection>,
    /// Free list (indices)
    free_indices: Vec<usize>,
    /// Pool capacity
    capacity: usize,
}

impl ConnectionPool {
    /// Create a new pool with capacity.
    pub fn new(capacity: usize) -> Self {
        let mut connections = Vec::with_capacity(capacity);
        let mut free_indices = Vec::with_capacity(capacity);

        for i in 0..capacity {
            connections.push(Connection::with_id(i as u64));
            free_indices.push(i);
        }

        Self {
            connections,
            free_indices,
            capacity,
        }
    }

    /// Acquire a connection from the pool.
    #[inline]
    pub fn acquire(&mut self) -> Option<&mut Connection> {
        let index = self.free_indices.pop()?;
        let conn = &mut self.connections[index];
        conn.state = ConnectionState::Idle;
        CONNECTION_STATS.record_pool_acquire();
        Some(conn)
    }

    /// Release a connection back to the pool.
    #[inline]
    pub fn release(&mut self, id: u64) {
        let index = id as usize;
        if index < self.capacity {
            self.connections[index].force_close();
            self.free_indices.push(index);
            CONNECTION_STATS.record_pool_release();
        }
    }

    /// Get connection by ID.
    #[inline]
    pub fn get(&self, id: u64) -> Option<&Connection> {
        let index = id as usize;
        if index < self.capacity {
            Some(&self.connections[index])
        } else {
            None
        }
    }

    /// Get mutable connection by ID.
    #[inline]
    pub fn get_mut(&mut self, id: u64) -> Option<&mut Connection> {
        let index = id as usize;
        if index < self.capacity {
            Some(&mut self.connections[index])
        } else {
            None
        }
    }

    /// Get pool capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get number of available connections.
    #[inline]
    pub fn available(&self) -> usize {
        self.free_indices.len()
    }

    /// Get number of active connections.
    #[inline]
    pub fn active(&self) -> usize {
        self.capacity - self.free_indices.len()
    }
}

// ============================================================================
// Connection Configuration
// ============================================================================

/// Configuration for connection handling.
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    /// Idle timeout for keep-alive connections
    pub idle_timeout: Duration,
    /// Maximum requests per connection
    pub max_requests: u32,
    /// Keep-alive enabled by default
    pub keep_alive: bool,
    /// Read timeout
    pub read_timeout: Duration,
    /// Write timeout
    pub write_timeout: Duration,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            idle_timeout: Duration::from_secs(60),
            max_requests: 1000,
            keep_alive: true,
            read_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(30),
        }
    }
}

impl ConnectionConfig {
    /// Create new configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set idle timeout.
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Set max requests per connection.
    pub fn max_requests(mut self, max: u32) -> Self {
        self.max_requests = max;
        self
    }

    /// Enable/disable keep-alive.
    pub fn keep_alive(mut self, enabled: bool) -> Self {
        self.keep_alive = enabled;
        self
    }

    /// Set read timeout.
    pub fn read_timeout(mut self, timeout: Duration) -> Self {
        self.read_timeout = timeout;
        self
    }

    /// Set write timeout.
    pub fn write_timeout(mut self, timeout: Duration) -> Self {
        self.write_timeout = timeout;
        self
    }
}

// ============================================================================
// Statistics
// ============================================================================

/// Connection statistics.
#[derive(Debug, Default)]
pub struct ConnectionStats {
    /// Total connections established
    connected: AtomicU64,
    /// Total connections closed
    closed: AtomicU64,
    /// Total connection errors
    errors: AtomicU64,
    /// Keep-alive reuses
    reuses: AtomicU64,
    /// Pool acquires
    pool_acquires: AtomicU64,
    /// Pool releases
    pool_releases: AtomicU64,
    /// State transitions
    #[allow(dead_code)] // Reserved for future state machine telemetry
    transitions: AtomicU64,
}

impl ConnectionStats {
    /// Create new stats.
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    fn record_connected(&self) {
        self.connected.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn record_closed(&self) {
        self.closed.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn record_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn record_reuse(&self) {
        self.reuses.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn record_pool_acquire(&self) {
        self.pool_acquires.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn record_pool_release(&self) {
        self.pool_releases.fetch_add(1, Ordering::Relaxed);
    }

    /// Get total connections.
    pub fn connected(&self) -> u64 {
        self.connected.load(Ordering::Relaxed)
    }

    /// Get closed connections.
    pub fn closed(&self) -> u64 {
        self.closed.load(Ordering::Relaxed)
    }

    /// Get errors.
    pub fn errors(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }

    /// Get keep-alive reuses.
    pub fn reuses(&self) -> u64 {
        self.reuses.load(Ordering::Relaxed)
    }

    /// Get currently active connections.
    pub fn active(&self) -> u64 {
        let connected = self.connected.load(Ordering::Relaxed);
        let closed = self.closed.load(Ordering::Relaxed);
        connected.saturating_sub(closed)
    }

    /// Get pool acquires.
    pub fn pool_acquires(&self) -> u64 {
        self.pool_acquires.load(Ordering::Relaxed)
    }

    /// Get pool releases.
    pub fn pool_releases(&self) -> u64 {
        self.pool_releases.load(Ordering::Relaxed)
    }
}

/// Global connection ID counter.
static CONNECTION_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Global connection statistics.
static CONNECTION_STATS: ConnectionStats = ConnectionStats {
    connected: AtomicU64::new(0),
    closed: AtomicU64::new(0),
    errors: AtomicU64::new(0),
    reuses: AtomicU64::new(0),
    pool_acquires: AtomicU64::new(0),
    pool_releases: AtomicU64::new(0),
    transitions: AtomicU64::new(0),
};

/// Get global connection statistics.
pub fn connection_stats() -> &'static ConnectionStats {
    &CONNECTION_STATS
}

// ============================================================================
// State Machine Executor
// ============================================================================

/// State machine executor with event batching.
///
/// Processes multiple events efficiently with minimal overhead.
pub struct StateMachineExecutor {
    /// Pending events
    events: Vec<(u64, ConnectionEvent)>,
    /// Batch size
    #[allow(dead_code)] // Reserved for batch processing limits
    batch_size: usize,
}

impl StateMachineExecutor {
    /// Create new executor.
    pub fn new(batch_size: usize) -> Self {
        Self {
            events: Vec::with_capacity(batch_size),
            batch_size,
        }
    }

    /// Queue an event for processing.
    #[inline]
    pub fn queue(&mut self, conn_id: u64, event: ConnectionEvent) {
        self.events.push((conn_id, event));
    }

    /// Process all queued events against a connection pool.
    #[inline]
    pub fn process(&mut self, pool: &mut ConnectionPool) -> Vec<(u64, TransitionAction)> {
        let mut results = Vec::with_capacity(self.events.len());

        for (conn_id, event) in self.events.drain(..) {
            if let Some(conn) = pool.get_mut(conn_id) {
                let action = conn.handle_event(event);
                results.push((conn_id, action));
            }
        }

        results
    }

    /// Process events with a callback for each action.
    #[inline]
    pub fn process_with<F>(&mut self, pool: &mut ConnectionPool, mut callback: F)
    where
        F: FnMut(u64, TransitionAction),
    {
        for (conn_id, event) in self.events.drain(..) {
            if let Some(conn) = pool.get_mut(conn_id) {
                let action = conn.handle_event(event);
                callback(conn_id, action);
            }
        }
    }

    /// Check if executor has pending events.
    #[inline]
    pub fn has_pending(&self) -> bool {
        !self.events.is_empty()
    }

    /// Get pending event count.
    #[inline]
    pub fn pending_count(&self) -> usize {
        self.events.len()
    }

    /// Clear pending events.
    #[inline]
    pub fn clear(&mut self) {
        self.events.clear();
    }
}

impl Default for StateMachineExecutor {
    fn default() -> Self {
        Self::new(64)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_state_size() {
        // Verify state is 1 byte
        assert_eq!(std::mem::size_of::<ConnectionState>(), 1);
    }

    #[test]
    fn test_connection_event_size() {
        // Verify event is 1 byte
        assert_eq!(std::mem::size_of::<ConnectionEvent>(), 1);
    }

    #[test]
    fn test_transition_entry_size() {
        // Verify entry is 2 bytes
        assert_eq!(std::mem::size_of::<TransitionEntry>(), 2);
    }

    #[test]
    fn test_basic_state_transitions() {
        let mut conn = Connection::new();
        assert_eq!(conn.state(), ConnectionState::Idle);

        // Accept -> Connected
        let action = conn.handle_event(ConnectionEvent::Accept);
        assert_eq!(conn.state(), ConnectionState::Connected);
        assert_eq!(action, TransitionAction::StartRead);

        // DataReady -> Reading
        let action = conn.handle_event(ConnectionEvent::DataReady);
        assert_eq!(conn.state(), ConnectionState::Reading);
        assert_eq!(action, TransitionAction::ContinueRead);

        // RequestComplete -> Processing
        let action = conn.handle_event(ConnectionEvent::RequestComplete);
        assert_eq!(conn.state(), ConnectionState::Processing);
        assert_eq!(action, TransitionAction::Dispatch);

        // ResponseReady -> Writing
        let action = conn.handle_event(ConnectionEvent::ResponseReady);
        assert_eq!(conn.state(), ConnectionState::Writing);
        assert_eq!(action, TransitionAction::StartWrite);

        // WriteComplete -> Closing
        let action = conn.handle_event(ConnectionEvent::WriteComplete);
        assert_eq!(conn.state(), ConnectionState::Closing);
        assert_eq!(action, TransitionAction::InitiateClose);
    }

    #[test]
    fn test_keep_alive_transition() {
        let mut conn = Connection::new();
        conn.handle_event(ConnectionEvent::Accept);
        conn.handle_event(ConnectionEvent::DataReady);
        conn.handle_event(ConnectionEvent::RequestComplete);
        conn.handle_event(ConnectionEvent::ResponseReady);

        // KeepAlive -> back to Connected
        let action = conn.handle_event(ConnectionEvent::KeepAlive);
        assert_eq!(conn.state(), ConnectionState::Connected);
        assert_eq!(action, TransitionAction::Reset);
    }

    #[test]
    fn test_error_transition() {
        let mut conn = Connection::new();
        conn.handle_event(ConnectionEvent::Accept);

        // Error -> Error state
        let action = conn.handle_event(ConnectionEvent::Error);
        assert_eq!(conn.state(), ConnectionState::Error);
        assert_eq!(action, TransitionAction::ForceClose);
        assert!(conn.state().is_terminal());
    }

    #[test]
    fn test_timeout_transition() {
        let mut conn = Connection::new();
        conn.handle_event(ConnectionEvent::Accept);

        // Timeout -> Closing
        let action = conn.handle_event(ConnectionEvent::Timeout);
        assert_eq!(conn.state(), ConnectionState::Closing);
        assert_eq!(action, TransitionAction::InitiateClose);
    }

    #[test]
    fn test_connection_pool() {
        let mut pool = ConnectionPool::new(10);

        assert_eq!(pool.capacity(), 10);
        assert_eq!(pool.available(), 10);
        assert_eq!(pool.active(), 0);

        // Acquire connections
        let conn1 = pool.acquire().unwrap();
        let id1 = conn1.id();
        assert_eq!(pool.available(), 9);
        assert_eq!(pool.active(), 1);

        let conn2 = pool.acquire().unwrap();
        let id2 = conn2.id();
        assert_eq!(pool.available(), 8);

        // Release
        pool.release(id1);
        assert_eq!(pool.available(), 9);

        pool.release(id2);
        assert_eq!(pool.available(), 10);
    }

    #[test]
    fn test_connection_config() {
        let config = ConnectionConfig::new()
            .idle_timeout(Duration::from_secs(120))
            .max_requests(500)
            .keep_alive(false)
            .read_timeout(Duration::from_secs(10))
            .write_timeout(Duration::from_secs(10));

        assert_eq!(config.idle_timeout, Duration::from_secs(120));
        assert_eq!(config.max_requests, 500);
        assert!(!config.keep_alive);
    }

    #[test]
    fn test_state_machine_executor() {
        let mut pool = ConnectionPool::new(5);
        let mut executor = StateMachineExecutor::new(10);

        // Acquire a connection
        let conn = pool.acquire().unwrap();
        let conn_id = conn.id();

        // Queue events
        executor.queue(conn_id, ConnectionEvent::Accept);
        executor.queue(conn_id, ConnectionEvent::DataReady);
        executor.queue(conn_id, ConnectionEvent::RequestComplete);

        assert!(executor.has_pending());
        assert_eq!(executor.pending_count(), 3);

        // Process
        let results = executor.process(&mut pool);
        assert_eq!(results.len(), 3);
        assert!(!executor.has_pending());

        // Verify final state
        let conn = pool.get(conn_id).unwrap();
        assert_eq!(conn.state(), ConnectionState::Processing);
    }

    #[test]
    fn test_request_count() {
        let mut conn = Connection::new();
        assert_eq!(conn.request_count(), 0);

        // Process a request
        conn.handle_event(ConnectionEvent::Accept);
        conn.handle_event(ConnectionEvent::DataReady);
        conn.handle_event(ConnectionEvent::RequestComplete); // Increments here
        assert_eq!(conn.request_count(), 1);

        conn.handle_event(ConnectionEvent::ResponseReady);
        conn.handle_event(ConnectionEvent::KeepAlive);

        // Second request
        conn.handle_event(ConnectionEvent::DataReady);
        conn.handle_event(ConnectionEvent::RequestComplete);
        assert_eq!(conn.request_count(), 2);
    }

    #[test]
    fn test_try_transition_valid() {
        let mut conn = Connection::new();
        let result = conn.try_transition(ConnectionEvent::Accept);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), TransitionAction::StartRead);
    }

    #[test]
    fn test_try_transition_invalid() {
        let mut conn = Connection::new();
        // DataReady in Idle state is a no-op
        let result = conn.try_transition(ConnectionEvent::DataReady);
        assert!(result.is_err());
    }

    #[test]
    fn test_connection_stats() {
        let stats = connection_stats();
        let _ = stats.connected();
        let _ = stats.closed();
        let _ = stats.errors();
        let _ = stats.active();
        let _ = stats.reuses();
    }

    #[test]
    fn test_state_is_terminal() {
        assert!(ConnectionState::Closed.is_terminal());
        assert!(ConnectionState::Error.is_terminal());
        assert!(!ConnectionState::Connected.is_terminal());
        assert!(!ConnectionState::Processing.is_terminal());
    }

    #[test]
    fn test_state_is_active() {
        assert!(ConnectionState::Reading.is_active());
        assert!(ConnectionState::Processing.is_active());
        assert!(ConnectionState::Writing.is_active());
        assert!(!ConnectionState::Idle.is_active());
        assert!(!ConnectionState::Connected.is_active());
    }

    // Recycling tests

    #[test]
    fn test_recyclable_connection_basic() {
        let mut conn = RecyclableConnection::new();
        assert_eq!(conn.generation(), 0);
        assert_eq!(conn.recycle_count(), 0);
        assert!(conn.is_clean());

        // Use the connection
        conn.handle_event(ConnectionEvent::Accept);
        conn.handle_event(ConnectionEvent::DataReady);

        // Prepare for recycle
        conn.prepare_for_recycle();
        assert_eq!(conn.generation(), 1);
        assert_eq!(conn.recycle_count(), 1);
        assert!(conn.is_clean());
        assert_eq!(conn.state(), ConnectionState::Idle);
    }

    #[test]
    fn test_recyclable_connection_user_data() {
        let mut conn = RecyclableConnection::new();

        // Set user data
        conn.set_user_data(42u32);
        assert_eq!(conn.user_data::<u32>(), Some(&42));

        // Take user data
        let data = conn.take_user_data::<u32>();
        assert_eq!(data, Some(42));
        assert!(conn.user_data::<u32>().is_none());
    }

    #[test]
    fn test_recyclable_connection_buffer_capacities() {
        let conn = RecyclableConnection::with_capacities(4096, 8192);
        assert_eq!(conn.read_buffer_capacity(), 4096);
        assert_eq!(conn.write_buffer_capacity(), 8192);
    }

    #[test]
    fn test_recycle_pool_basic() {
        let mut pool: RecyclePool<RecyclableConnection> = RecyclePool::new(5);

        assert_eq!(pool.capacity(), 5);
        assert_eq!(pool.available(), 5);
        assert_eq!(pool.active(), 0);
        assert!(!pool.is_exhausted());

        // Acquire and use
        {
            let mut handle = pool.acquire().unwrap();
            handle.handle_event(ConnectionEvent::Accept);
            assert_eq!(handle.state(), ConnectionState::Connected);
            // Note: cannot check pool state while handle is borrowed
        }

        // Auto-released when handle dropped
        assert_eq!(pool.available(), 5);
        assert_eq!(pool.active(), 0);
    }

    #[test]
    fn test_recycle_pool_exhaustion() {
        let mut pool: RecyclePool<RecyclableConnection> = RecyclePool::new(2);

        // Initial state
        assert_eq!(pool.capacity(), 2);
        assert_eq!(pool.available(), 2);
        assert!(!pool.is_exhausted());

        // Acquire and release one
        {
            let h = pool.acquire().unwrap();
            assert_eq!(h.index(), 1); // Last index pushed
        }
        assert_eq!(pool.available(), 2);

        // Acquire again - should get the same slot back (LIFO)
        {
            let h = pool.acquire().unwrap();
            assert_eq!(h.index(), 1);
        }

        // Pool should still have all slots available
        assert!(!pool.is_exhausted());
    }

    #[test]
    fn test_recycle_pool_generation_tracking() {
        let mut pool: RecyclePool<RecyclableConnection> = RecyclePool::new(1);

        let gen1 = {
            let handle = pool.acquire().unwrap();
            handle.generation()
        };

        let gen2 = {
            let handle = pool.acquire().unwrap();
            handle.generation()
        };

        // Generation should increment on recycle
        assert!(gen2 > gen1);
    }

    #[test]
    fn test_recycle_pool_high_water_mark() {
        let mut pool: RecyclePool<RecyclableConnection> = RecyclePool::new(5);

        // Acquire and release 3 connections one at a time to build up high water mark
        {
            let _ = pool.acquire().unwrap();
        }
        assert!(pool.high_water_mark() >= 1);

        // Check high water mark is tracked
        let hwm_before = pool.high_water_mark();

        // Reset
        pool.reset_high_water_mark();
        assert!(pool.high_water_mark() <= hwm_before);
    }

    #[test]
    fn test_recycle_pool_grow() {
        let mut pool: RecyclePool<RecyclableConnection> = RecyclePool::new(2);

        assert_eq!(pool.capacity(), 2);

        pool.grow(3);

        assert_eq!(pool.capacity(), 5);
        assert_eq!(pool.available(), 5);
    }

    #[test]
    fn test_recycle_pool_config() {
        let config = RecyclePoolConfig::new()
            .initial_capacity(50)
            .max_capacity(500)
            .grow_by(25)
            .shrink_threshold(0.1)
            .min_capacity(5);

        assert_eq!(config.initial_capacity, 50);
        assert_eq!(config.max_capacity, 500);
        assert_eq!(config.grow_by, 25);
        assert!((config.shrink_threshold - 0.1).abs() < 0.001);
        assert_eq!(config.min_capacity, 5);
    }

    #[test]
    fn test_connection_recycler() {
        let mut recycler = ConnectionRecycler::new(10);

        let stats = recycler.stats();
        assert_eq!(stats.capacity, 10);
        assert_eq!(stats.available, 10);
        assert_eq!(stats.active, 0);

        // Acquire and use
        {
            let mut handle = recycler.acquire().unwrap();
            handle.handle_event(ConnectionEvent::Accept);
            // Note: can't check stats while handle is borrowed
        }

        // Released
        let stats = recycler.stats();
        assert_eq!(stats.active, 0);
        assert_eq!(stats.available, 10);
    }

    #[test]
    fn test_recycle_stats() {
        let stats = recycle_stats();
        let _ = stats.acquires();
        let _ = stats.releases();
        let _ = stats.recycles();
        let _ = stats.allocations();
        let _ = stats.recycle_ratio();
        let _ = stats.hit_ratio();
    }

    #[test]
    fn test_recyclable_trait() {
        let mut conn = RecyclableConnection::new();

        // Implement Recyclable
        assert!(conn.is_clean());
        assert_eq!(conn.generation(), 0);

        conn.handle_event(ConnectionEvent::Accept);
        conn.increment_generation();
        assert_eq!(conn.generation(), 1);

        conn.reset();
        assert!(conn.is_clean());
        assert_eq!(conn.generation(), 2); // reset increments generation
    }
}
