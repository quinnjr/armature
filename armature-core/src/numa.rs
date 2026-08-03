//! NUMA-Aware Memory Allocation
//!
//! This module provides NUMA (Non-Uniform Memory Access) awareness for
//! optimal memory allocation on multi-socket systems. By allocating memory
//! on the same NUMA node as the executing worker, we minimize memory access
//! latencies and maximize throughput.
//!
//! ## What is NUMA?
//!
//! On multi-socket systems, each CPU socket has its own local memory.
//! Accessing local memory is fast (~100ns), while accessing remote memory
//! on another socket is slower (~300ns). NUMA-aware allocation ensures
//! data stays close to the CPU that uses it.
//!
//! ```text
//! ┌─────────────┐     ┌─────────────┐
//! │   Socket 0  │     │   Socket 1  │
//! │  ┌───────┐  │     │  ┌───────┐  │
//! │  │ CPUs  │  │     │  │ CPUs  │  │
//! │  │ 0-7   │  │     │  │ 8-15  │  │
//! │  └───────┘  │     │  └───────┘  │
//! │      │      │     │      │      │
//! │  ┌───────┐  │     │  ┌───────┐  │
//! │  │ RAM   │◄─┼─────┼──│ RAM   │  │
//! │  │ Node 0│  │     │  │ Node 1│  │
//! │  └───────┘  │     │  └───────┘  │
//! └─────────────┘     └─────────────┘
//!       Local           Remote
//!      ~100ns           ~300ns
//! ```
//!
//! ## Usage
//!
//! ```rust,ignore
//! use armature_core::numa::{NumaConfig, NumaNode, numa_available};
//!
//! // Check if NUMA is available
//! if numa_available() {
//!     let config = NumaConfig::detect();
//!     println!("NUMA nodes: {}", config.num_nodes());
//!
//!     // Get current node for this thread
//!     let node = NumaNode::current();
//!
//!     // Allocate on specific node
//!     let buffer = node.allocate(4096);
//! }
//! ```
//!
//! ## Performance Impact
//!
//! On NUMA systems with proper allocation:
//! - Memory access latency: ~100ns (local) vs ~300ns (remote)
//! - Memory bandwidth: 2-3x higher for local access
//! - Cache coherency traffic reduced
//!
//! Expected throughput improvement: 5-15% on multi-socket systems.
//!
//! ## Implementation Notes
//!
//! On Linux, thread binding is implemented with the `set_mempolicy(2)`
//! syscall and buffer placement with `mbind(2)` (both invoked via
//! `libc::syscall`, no libnuma dependency). Two rules govern behavior:
//!
//! - **Thread-scoped policy**: `set_mempolicy` only affects the *calling*
//!   thread. [`bind_to_node`] / [`bind_to_local_node`] must therefore be
//!   called on the worker thread whose allocations they should restrict
//!   (this is how [`init_worker_numa`] is intended to be used).
//! - **Graceful degradation for allocations**: [`NumaAllocator`] always
//!   returns usable memory. If `mbind` fails (e.g. NUMA syscalls not
//!   compiled into the kernel), the allocation is kept without NUMA
//!   placement, a `tracing` warning is emitted, and the miss is recorded
//!   in [`NumaAllocStats::mbind_misses`]. Binding functions, in contrast,
//!   report failures as errors and never pretend success — on non-Linux
//!   platforms they return [`NumaError::NotSupported`].

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

// ============================================================================
// NUMA Availability Detection
// ============================================================================

/// Check if NUMA is available on this system.
///
/// On Linux, this is a lightweight heuristic: it returns `true` only if
/// `/sys/devices/system/node/node1` exists, i.e. the kernel exposes a
/// *second* NUMA node under sysfs. It does **not** use libnuma, does not
/// attempt to call `set_mempolicy`/`mbind`, and does not verify that NUMA
/// syscalls actually work (e.g. under seccomp or a container sandbox that
/// blocks them) — the binding functions ([`bind_to_node`],
/// [`bind_to_local_node`]) are the ones that surface those failures. They
/// distinguish the two causes: [`NumaError::NotSupported`] when the kernel
/// does not implement the syscall at all (`ENOSYS`), and
/// [`NumaError::BindFailed`] when the syscall exists but is rejected — which
/// is what a seccomp filter or container sandbox typically produces
/// (`EPERM`). On non-Linux platforms this always returns `false`.
///
/// In short: `true` means "sysfs reports more than one NUMA node," not
/// "NUMA placement is guaranteed to work."
#[inline]
pub fn numa_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        is_numa_available_linux()
    }

    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[cfg(target_os = "linux")]
fn is_numa_available_linux() -> bool {
    // Heuristic only: a second `nodeN` entry under sysfs implies a NUMA
    // topology, but doesn't confirm set_mempolicy/mbind will succeed.
    std::path::Path::new("/sys/devices/system/node/node1").exists()
}

/// Get the number of NUMA nodes.
#[inline]
pub fn num_numa_nodes() -> usize {
    #[cfg(target_os = "linux")]
    {
        num_numa_nodes_linux()
    }

    #[cfg(not(target_os = "linux"))]
    {
        1
    }
}

#[cfg(target_os = "linux")]
fn num_numa_nodes_linux() -> usize {
    // Count directories matching /sys/devices/system/node/node*
    let node_path = std::path::Path::new("/sys/devices/system/node");
    if let Ok(entries) = std::fs::read_dir(node_path) {
        entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .map(|n| n.starts_with("node"))
                    .unwrap_or(false)
            })
            .count()
    } else {
        1
    }
}

// ============================================================================
// NUMA Node
// ============================================================================

/// Represents a NUMA node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NumaNode {
    /// Node ID (0-based)
    id: usize,
}

impl NumaNode {
    /// Create a NUMA node reference.
    #[inline]
    pub const fn new(id: usize) -> Self {
        Self { id }
    }

    /// Get the node ID.
    #[inline]
    pub const fn id(&self) -> usize {
        self.id
    }

    /// Get the NUMA node for the current CPU.
    #[inline]
    pub fn current() -> Self {
        Self::new(current_numa_node())
    }

    /// Get all available NUMA nodes.
    pub fn all() -> Vec<Self> {
        (0..num_numa_nodes()).map(Self::new).collect()
    }

    /// Get CPUs belonging to this node.
    pub fn cpus(&self) -> Vec<usize> {
        #[cfg(target_os = "linux")]
        {
            self.cpus_linux()
        }

        #[cfg(not(target_os = "linux"))]
        {
            // On non-NUMA systems, return all CPUs
            (0..std::thread::available_parallelism()
                .map(|p| p.get())
                .unwrap_or(1))
                .collect()
        }
    }

    #[cfg(target_os = "linux")]
    fn cpus_linux(&self) -> Vec<usize> {
        let path = format!("/sys/devices/system/node/node{}/cpulist", self.id);
        if let Ok(content) = std::fs::read_to_string(&path) {
            parse_cpu_list(&content)
        } else {
            Vec::new()
        }
    }

    /// Get total memory on this node (bytes).
    pub fn total_memory(&self) -> u64 {
        #[cfg(target_os = "linux")]
        {
            self.total_memory_linux()
        }

        #[cfg(not(target_os = "linux"))]
        {
            0
        }
    }

    #[cfg(target_os = "linux")]
    fn total_memory_linux(&self) -> u64 {
        let path = format!("/sys/devices/system/node/node{}/meminfo", self.id);
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                if line.contains("MemTotal:")
                    && let Some(kb_str) = line.split_whitespace().nth(3)
                    && let Ok(kb) = kb_str.parse::<u64>()
                {
                    return kb * 1024;
                }
            }
        }
        0
    }

    /// Get free memory on this node (bytes).
    pub fn free_memory(&self) -> u64 {
        #[cfg(target_os = "linux")]
        {
            self.free_memory_linux()
        }

        #[cfg(not(target_os = "linux"))]
        {
            0
        }
    }

    #[cfg(target_os = "linux")]
    fn free_memory_linux(&self) -> u64 {
        let path = format!("/sys/devices/system/node/node{}/meminfo", self.id);
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                if line.contains("MemFree:")
                    && let Some(kb_str) = line.split_whitespace().nth(3)
                    && let Ok(kb) = kb_str.parse::<u64>()
                {
                    return kb * 1024;
                }
            }
        }
        0
    }

    /// Get distance to another node (lower is better).
    ///
    /// Returns 10 for local, higher for remote nodes.
    pub fn distance_to(&self, other: &NumaNode) -> u32 {
        #[cfg(target_os = "linux")]
        {
            self.distance_to_linux(other)
        }

        #[cfg(not(target_os = "linux"))]
        {
            let _ = other;
            10 // Local distance
        }
    }

    #[cfg(target_os = "linux")]
    fn distance_to_linux(&self, other: &NumaNode) -> u32 {
        let path = format!("/sys/devices/system/node/node{}/distance", self.id);
        if let Ok(content) = std::fs::read_to_string(&path)
            && let Some(dist_str) = content.split_whitespace().nth(other.id)
            && let Ok(dist) = dist_str.parse::<u32>()
        {
            return dist;
        }
        if self.id == other.id { 10 } else { 20 }
    }
}

/// Get the NUMA node for the current CPU.
#[inline]
pub fn current_numa_node() -> usize {
    #[cfg(target_os = "linux")]
    {
        current_numa_node_linux()
    }

    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

#[cfg(target_os = "linux")]
fn current_numa_node_linux() -> usize {
    // Read from /proc/self/numa_maps or use getcpu
    // Simplified: read CPU and map to node
    if let Ok(content) = std::fs::read_to_string("/proc/self/stat") {
        // Field 39 (0-indexed 38) is the processor number
        let fields: Vec<&str> = content.split_whitespace().collect();
        if fields.len() > 38
            && let Ok(cpu) = fields[38].parse::<usize>()
        {
            return cpu_to_numa_node(cpu);
        }
    }
    0
}

#[cfg(target_os = "linux")]
fn cpu_to_numa_node(cpu: usize) -> usize {
    // Check which node the CPU belongs to
    for node_id in 0..num_numa_nodes() {
        let path = format!("/sys/devices/system/node/node{}/cpulist", node_id);
        if let Ok(content) = std::fs::read_to_string(&path) {
            let cpus = parse_cpu_list(&content);
            if cpus.contains(&cpu) {
                return node_id;
            }
        }
    }
    0
}

/// Parse a CPU list like "0-7,16-23"
#[cfg(target_os = "linux")]
fn parse_cpu_list(s: &str) -> Vec<usize> {
    let mut cpus = Vec::new();
    for part in s.trim().split(',') {
        if let Some((start, end)) = part.split_once('-') {
            if let (Ok(s), Ok(e)) = (start.parse::<usize>(), end.parse::<usize>()) {
                cpus.extend(s..=e);
            }
        } else if let Ok(cpu) = part.parse::<usize>() {
            cpus.push(cpu);
        }
    }
    cpus
}

// ============================================================================
// NUMA Configuration
// ============================================================================

/// NUMA system configuration.
#[derive(Debug, Clone)]
pub struct NumaConfig {
    /// Number of NUMA nodes
    num_nodes: usize,
    /// NUMA nodes
    nodes: Vec<NumaNode>,
    /// Total system memory
    total_memory: u64,
    /// Memory allocation policy
    policy: NumaPolicy,
}

impl NumaConfig {
    /// Detect NUMA configuration from the system.
    pub fn detect() -> Self {
        let num_nodes = num_numa_nodes();
        let nodes: Vec<NumaNode> = (0..num_nodes).map(NumaNode::new).collect();
        let total_memory: u64 = nodes.iter().map(|n| n.total_memory()).sum();

        Self {
            num_nodes,
            nodes,
            total_memory,
            policy: NumaPolicy::Local,
        }
    }

    /// Get number of NUMA nodes.
    #[inline]
    pub fn num_nodes(&self) -> usize {
        self.num_nodes
    }

    /// Get all NUMA nodes.
    #[inline]
    pub fn nodes(&self) -> &[NumaNode] {
        &self.nodes
    }

    /// Get total system memory.
    #[inline]
    pub fn total_memory(&self) -> u64 {
        self.total_memory
    }

    /// Get current NUMA policy.
    #[inline]
    pub fn policy(&self) -> NumaPolicy {
        self.policy
    }

    /// Set NUMA policy.
    #[inline]
    pub fn set_policy(&mut self, policy: NumaPolicy) {
        self.policy = policy;
    }

    /// Check if this is a NUMA system.
    #[inline]
    pub fn is_numa(&self) -> bool {
        self.num_nodes > 1
    }

    /// Get optimal node for a worker ID.
    ///
    /// Distributes workers across nodes round-robin.
    #[inline]
    pub fn node_for_worker(&self, worker_id: usize) -> NumaNode {
        if self.num_nodes > 0 {
            self.nodes[worker_id % self.num_nodes]
        } else {
            NumaNode::new(0)
        }
    }

    /// Get the node with most free memory.
    pub fn node_with_most_free_memory(&self) -> NumaNode {
        self.nodes
            .iter()
            .max_by_key(|n| n.free_memory())
            .copied()
            .unwrap_or(NumaNode::new(0))
    }
}

impl Default for NumaConfig {
    fn default() -> Self {
        Self::detect()
    }
}

/// NUMA memory allocation policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NumaPolicy {
    /// Allocate on local node (best for latency)
    #[default]
    Local,
    /// Interleave across all nodes (best for bandwidth)
    Interleave,
    /// Prefer specific node but allow fallback
    Preferred(usize),
    /// Bind to specific node strictly
    Bind(usize),
}

// ============================================================================
// Linux mempolicy syscall support
// ============================================================================

/// `MPOL_*` constants for `set_mempolicy(2)` / `mbind(2)`.
///
/// Defined locally because the `libc` crate does not export them for all
/// supported targets. Values are stable kernel ABI (see
/// `include/uapi/linux/mempolicy.h`).
#[cfg(target_os = "linux")]
mod mempolicy {
    #![allow(dead_code)]

    /// Restore the default (local allocation) policy.
    pub const MPOL_DEFAULT: libc::c_int = 0;
    /// Prefer the given node, fall back to others under pressure.
    pub const MPOL_PREFERRED: libc::c_int = 1;
    /// Strictly restrict allocation to the given node set.
    pub const MPOL_BIND: libc::c_int = 2;
    /// Interleave allocations across the given node set.
    pub const MPOL_INTERLEAVE: libc::c_int = 3;
    /// Allocate on the node of the CPU that triggers the fault (kernel >= 3.8).
    pub const MPOL_LOCAL: libc::c_int = 4;

    /// `mbind` flag: migrate already-faulted pages so they conform to the
    /// policy (needed because `alloc_zeroed` may touch pages before `mbind`).
    pub const MPOL_MF_MOVE: libc::c_uint = 1 << 1;

    /// Number of bits in the nodemask we pass to the kernel (a single `u64`).
    /// Node ids >= this cannot be expressed in the mask.
    pub const MAX_NODES: usize = 64;
}

/// Raw `set_mempolicy(2)` wrapper for the calling thread.
///
/// Pass `nodemask: None` for modes that take no mask (`MPOL_DEFAULT`,
/// `MPOL_LOCAL`). Returns the OS error on failure.
#[cfg(target_os = "linux")]
fn set_mempolicy(
    mode: libc::c_int,
    nodemask: Option<&u64>,
    maxnode: usize,
) -> Result<(), std::io::Error> {
    let mask_ptr = nodemask.map_or(std::ptr::null(), |m| m as *const u64);
    // SAFETY: mask_ptr is either null (with maxnode 0) or points to a valid
    // u64 that outlives the call; maxnode never exceeds the mask's bit width.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_set_mempolicy,
            mode,
            mask_ptr,
            maxnode as libc::c_ulong,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Allocation alignment used by [`NumaAllocator`].
///
/// On Linux this is the system page size so allocations are valid arguments
/// to `mbind(2)` (which requires a page-aligned address).
#[cfg(target_os = "linux")]
fn numa_alloc_align() -> usize {
    static ALIGN: OnceLock<usize> = OnceLock::new();
    *ALIGN.get_or_init(|| {
        // SAFETY: sysconf(_SC_PAGESIZE) has no memory-safety preconditions.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page > 0 { page as usize } else { 4096 }
    })
}

#[cfg(not(target_os = "linux"))]
fn numa_alloc_align() -> usize {
    64
}

// ============================================================================
// NUMA-Aware Allocator
// ============================================================================

/// NUMA-aware memory allocation.
///
/// This provides low-level NUMA allocation primitives. For most use cases,
/// prefer `NumaBuffer` or the worker state system.
pub struct NumaAllocator {
    /// Target NUMA node
    node: NumaNode,
    /// Allocation statistics
    stats: NumaAllocStats,
}

impl NumaAllocator {
    /// Create a NUMA allocator for a specific node.
    pub fn new(node: NumaNode) -> Self {
        Self {
            node,
            stats: NumaAllocStats::new(),
        }
    }

    /// Create a NUMA allocator for the local node.
    pub fn local() -> Self {
        Self::new(NumaNode::current())
    }

    /// Allocate zeroed memory placed on this node.
    ///
    /// Returns a pointer to the allocated memory, or None if allocation fails.
    ///
    /// On Linux the region is page-aligned and bound to the allocator's node
    /// with `mbind(2)` (`MPOL_BIND` + `MPOL_MF_MOVE`), so its pages actually
    /// land on that node. If `mbind` fails (e.g. NUMA syscalls unavailable),
    /// the allocation is still returned — placement degrades gracefully to
    /// the kernel default, a warning is logged, and the miss is counted in
    /// [`NumaAllocStats::mbind_misses`]. On non-Linux platforms no placement
    /// is attempted (every allocation counts as an `mbind` miss).
    ///
    /// # Safety
    ///
    /// The caller must ensure proper deallocation using `deallocate`.
    #[inline]
    pub fn allocate(&self, size: usize) -> Option<*mut u8> {
        if size == 0 {
            return None;
        }

        #[cfg(target_os = "linux")]
        {
            self.allocate_linux(size)
        }

        #[cfg(not(target_os = "linux"))]
        {
            self.allocate_fallback(size)
        }
    }

    #[cfg(target_os = "linux")]
    fn allocate_linux(&self, size: usize) -> Option<*mut u8> {
        use std::alloc::{Layout, alloc_zeroed};

        // Page-aligned so the region is a valid mbind(2) argument.
        // Zeroed so NumaBuffer::as_slice never exposes uninitialized memory.
        let layout = Layout::from_size_align(size, numa_alloc_align()).ok()?;
        let ptr = unsafe { alloc_zeroed(layout) };

        if ptr.is_null() {
            self.stats.record_failure();
            return None;
        }

        // Bind the region's pages to the target node. MPOL_MF_MOVE migrates
        // any pages alloc_zeroed already faulted in. Failure is non-fatal:
        // keep the allocation without NUMA placement (graceful degradation).
        if self.node.id() < mempolicy::MAX_NODES {
            let nodemask: u64 = 1u64 << self.node.id();
            // SAFETY: ptr..ptr+size is a live allocation we own; nodemask
            // outlives the call and maxnode matches its bit width.
            let rc = unsafe {
                libc::syscall(
                    libc::SYS_mbind,
                    ptr as *mut libc::c_void,
                    size as libc::c_ulong,
                    mempolicy::MPOL_BIND,
                    &nodemask as *const u64,
                    mempolicy::MAX_NODES as libc::c_ulong,
                    mempolicy::MPOL_MF_MOVE,
                )
            };
            if rc != 0 {
                let err = std::io::Error::last_os_error();
                tracing::warn!(
                    node = self.node.id(),
                    size,
                    error = %err,
                    "mbind failed; keeping allocation without NUMA placement"
                );
                self.stats.record_mbind_miss();
            }
        } else {
            tracing::warn!(
                node = self.node.id(),
                "NUMA node id exceeds nodemask width; keeping allocation without NUMA placement"
            );
            self.stats.record_mbind_miss();
        }

        self.stats.record_allocation(size, self.node.id());
        Some(ptr)
    }

    #[cfg(not(target_os = "linux"))]
    fn allocate_fallback(&self, size: usize) -> Option<*mut u8> {
        use std::alloc::{Layout, alloc_zeroed};

        // Zeroed so NumaBuffer::as_slice never exposes uninitialized memory
        let layout = Layout::from_size_align(size, numa_alloc_align()).ok()?;
        let ptr = unsafe { alloc_zeroed(layout) };

        if ptr.is_null() {
            self.stats.record_failure();
            None
        } else {
            // No NUMA placement possible on this platform.
            self.stats.record_mbind_miss();
            self.stats.record_allocation(size, 0);
            Some(ptr)
        }
    }

    /// Deallocate memory.
    ///
    /// # Safety
    ///
    /// `ptr` must have been allocated by this allocator with the given `size`.
    #[inline]
    pub unsafe fn deallocate(&self, ptr: *mut u8, size: usize) {
        use std::alloc::{Layout, dealloc};

        if let Ok(layout) = Layout::from_size_align(size, numa_alloc_align()) {
            // SAFETY: caller guarantees ptr was allocated with this size/alignment
            unsafe { dealloc(ptr, layout) };
            self.stats.record_deallocation(size);
        } else {
            // Layout::from_size_align only fails if `size`, rounded up to
            // `numa_alloc_align()`, would overflow `isize`. Reaching this
            // means the caller already violated this function's documented
            // `unsafe` precondition (ptr/size must match a prior `allocate`
            // call, whose size was validated against the same alignment).
            // There's nothing safe to free `ptr` with, so we deliberately
            // leak rather than guess at a layout — surface it so it's at
            // least observable in debug builds instead of vanishing silently.
            debug_assert!(
                false,
                "NumaAllocator::deallocate: invalid Layout for size={size}, align={}; \
                 caller violated documented safety preconditions, leaking ptr={ptr:p}",
                numa_alloc_align()
            );
            tracing::warn!(
                size,
                align = numa_alloc_align(),
                ?ptr,
                "NumaAllocator::deallocate: invalid Layout, leaking memory \
                 (caller violated documented unsafe preconditions)"
            );
        }
    }

    /// Get the target node.
    #[inline]
    pub fn node(&self) -> NumaNode {
        self.node
    }

    /// Get allocation statistics.
    #[inline]
    pub fn stats(&self) -> &NumaAllocStats {
        &self.stats
    }
}

// ============================================================================
// NUMA Buffer
// ============================================================================

/// A buffer allocated on a specific NUMA node.
#[derive(Debug)]
pub struct NumaBuffer {
    /// Pointer to data
    ptr: *mut u8,
    /// Buffer size
    size: usize,
    /// NUMA node
    node: NumaNode,
}

impl NumaBuffer {
    /// Allocate a buffer on the local NUMA node.
    pub fn new(size: usize) -> Option<Self> {
        Self::on_node(NumaNode::current(), size)
    }

    /// Allocate a buffer on a specific NUMA node.
    pub fn on_node(node: NumaNode, size: usize) -> Option<Self> {
        let allocator = NumaAllocator::new(node);
        let ptr = allocator.allocate(size)?;

        Some(Self { ptr, size, node })
    }

    /// Get the buffer size.
    #[inline]
    pub fn size(&self) -> usize {
        self.size
    }

    /// Get the NUMA node.
    #[inline]
    pub fn node(&self) -> NumaNode {
        self.node
    }

    /// Get a slice view of the buffer.
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.size) }
    }

    /// Get a mutable slice view.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.size) }
    }

    /// Get raw pointer.
    #[inline]
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    /// Get mutable raw pointer.
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }
}

impl Drop for NumaBuffer {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            let allocator = NumaAllocator::new(self.node);
            unsafe {
                allocator.deallocate(self.ptr, self.size);
            }
        }
    }
}

// SAFETY: NumaBuffer owns its memory and can be sent across threads
unsafe impl Send for NumaBuffer {}
unsafe impl Sync for NumaBuffer {}

// ============================================================================
// Worker NUMA Binding
// ============================================================================

/// Bind the calling thread's memory policy to a NUMA node.
///
/// On Linux this calls `set_mempolicy(MPOL_BIND, ...)`, strictly restricting
/// the **calling thread's** future memory allocations to the given node.
/// Because the policy is per-thread, this must be called on the worker
/// thread it should affect (see [`init_worker_numa`]).
///
/// # Errors
///
/// - [`NumaError::InvalidNode`] if the node id is out of range for this
///   system (or exceeds the supported 64-node mask width).
/// - [`NumaError::NotSupported`] on non-Linux platforms, or if the kernel
///   lacks the NUMA syscalls (`ENOSYS`).
/// - [`NumaError::BindFailed`] if `set_mempolicy` fails for any other
///   reason (the OS error is included in the message).
#[inline]
pub fn bind_to_node(node: NumaNode) -> Result<(), NumaError> {
    #[cfg(target_os = "linux")]
    {
        bind_to_node_linux(node)
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = node;
        NUMA_STATS.record_bind(false);
        Err(NumaError::NotSupported)
    }
}

#[cfg(target_os = "linux")]
fn bind_to_node_linux(node: NumaNode) -> Result<(), NumaError> {
    let num_nodes = num_numa_nodes();
    if node.id() >= num_nodes || node.id() >= mempolicy::MAX_NODES {
        NUMA_STATS.record_bind(false);
        return Err(NumaError::InvalidNode {
            node: node.id(),
            max: num_nodes.saturating_sub(1),
        });
    }

    let nodemask: u64 = 1u64 << node.id();
    match set_mempolicy(mempolicy::MPOL_BIND, Some(&nodemask), mempolicy::MAX_NODES) {
        Ok(()) => {
            NUMA_STATS.record_bind(true);
            Ok(())
        }
        Err(err) => {
            NUMA_STATS.record_bind(false);
            if err.raw_os_error() == Some(libc::ENOSYS) {
                Err(NumaError::NotSupported)
            } else {
                Err(NumaError::BindFailed {
                    reason: format!(
                        "set_mempolicy(MPOL_BIND, node {}) failed: {}",
                        node.id(),
                        err
                    ),
                })
            }
        }
    }
}

/// Bind the calling thread's memory policy to its local NUMA node.
///
/// On Linux this calls `set_mempolicy(MPOL_LOCAL)` (kernel >= 3.8), which
/// allocates pages on the node of the CPU that faults them in — following
/// the thread if it migrates. On kernels that predate `MPOL_LOCAL`
/// (`EINVAL`), it falls back to an explicit [`bind_to_node`] on the current
/// node. Like [`bind_to_node`], the policy only affects the calling thread.
///
/// # Errors
///
/// Returns [`NumaError::NotSupported`] on non-Linux platforms or when the
/// kernel lacks NUMA syscalls (`ENOSYS`), and [`NumaError::BindFailed`] for
/// other syscall failures.
#[inline]
pub fn bind_to_local_node() -> Result<(), NumaError> {
    #[cfg(target_os = "linux")]
    {
        match set_mempolicy(mempolicy::MPOL_LOCAL, None, 0) {
            Ok(()) => {
                NUMA_STATS.record_bind(true);
                Ok(())
            }
            // MPOL_LOCAL requires kernel >= 3.8; older kernels report EINVAL.
            // Fall back to a strict bind to the current node.
            Err(err) if err.raw_os_error() == Some(libc::EINVAL) => {
                bind_to_node(NumaNode::current())
            }
            Err(err) => {
                NUMA_STATS.record_bind(false);
                if err.raw_os_error() == Some(libc::ENOSYS) {
                    Err(NumaError::NotSupported)
                } else {
                    Err(NumaError::BindFailed {
                        reason: format!("set_mempolicy(MPOL_LOCAL) failed: {}", err),
                    })
                }
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        NUMA_STATS.record_bind(false);
        Err(NumaError::NotSupported)
    }
}

/// Initialize NUMA for a worker.
///
/// This binds the calling thread's memory policy to the node assigned to
/// `worker_id` (round-robin across nodes). Must be called **on the worker
/// thread itself**, since `set_mempolicy` only affects the calling thread.
/// On single-node or non-NUMA systems no binding is attempted and the
/// assigned node is returned as-is.
///
/// Binding failures that reflect the environment rather than a caller
/// mistake — the NUMA syscalls being unavailable ([`NumaError::NotSupported`],
/// e.g. seccomp/container sandbox or a non-Linux platform) or the bind
/// syscall being rejected ([`NumaError::BindFailed`]) — are treated as a
/// non-fatal degradation: a warning is logged and the assigned node is
/// returned as if binding had succeeded, mirroring the allocation path which
/// records an `mbind` miss instead of failing. Genuinely fatal
/// misconfiguration (e.g. [`NumaError::InvalidNode`]) is still returned as an
/// error.
pub fn init_worker_numa(worker_id: usize, config: &NumaConfig) -> Result<NumaNode, NumaError> {
    let node = config.node_for_worker(worker_id);

    if config.is_numa() {
        match bind_to_node(node) {
            Ok(()) => {}
            // Environment can't honor the bind: degrade gracefully instead of
            // failing worker init. The worker still runs; allocations simply
            // fall back to the kernel default placement.
            Err(err @ (NumaError::NotSupported | NumaError::BindFailed { .. })) => {
                tracing::warn!(
                    worker_id,
                    node = node.id(),
                    error = %err,
                    "NUMA bind unavailable for worker; continuing without NUMA placement"
                );
            }
            // Fatal misconfiguration (e.g. an invalid node id): still an error.
            Err(err) => return Err(err),
        }
    }

    NUMA_STATS.record_init();
    Ok(node)
}

// ============================================================================
// Error Types
// ============================================================================

/// NUMA operation error.
#[derive(Debug, Clone)]
pub enum NumaError {
    /// Invalid NUMA node
    InvalidNode { node: usize, max: usize },
    /// NUMA not available
    NotAvailable,
    /// NUMA binding is not supported on this platform or kernel
    NotSupported,
    /// Binding failed
    BindFailed { reason: String },
    /// Allocation failed
    AllocationFailed { size: usize },
}

impl std::fmt::Display for NumaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidNode { node, max } => {
                write!(f, "Invalid NUMA node {}, max is {}", node, max)
            }
            Self::NotAvailable => write!(f, "NUMA not available on this system"),
            Self::NotSupported => {
                write!(f, "NUMA binding not supported on this platform or kernel")
            }
            Self::BindFailed { reason } => write!(f, "NUMA binding failed: {}", reason),
            Self::AllocationFailed { size } => {
                write!(f, "NUMA allocation failed for {} bytes", size)
            }
        }
    }
}

impl std::error::Error for NumaError {}

// ============================================================================
// Statistics
// ============================================================================

/// NUMA allocation statistics.
#[derive(Debug, Default)]
pub struct NumaAllocStats {
    /// Total allocations
    allocations: AtomicU64,
    /// Total bytes allocated
    bytes_allocated: AtomicU64,
    /// Total deallocations
    deallocations: AtomicU64,
    /// Total bytes deallocated
    bytes_deallocated: AtomicU64,
    /// Failed allocations
    failures: AtomicU64,
    /// Allocations kept without NUMA placement (mbind failed or unavailable)
    mbind_misses: AtomicU64,
    /// Allocations per node
    per_node: [AtomicU64; 8], // Support up to 8 NUMA nodes
}

impl NumaAllocStats {
    /// Create new stats.
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    fn record_allocation(&self, size: usize, node: usize) {
        self.allocations.fetch_add(1, Ordering::Relaxed);
        self.bytes_allocated
            .fetch_add(size as u64, Ordering::Relaxed);
        if node < 8 {
            self.per_node[node].fetch_add(1, Ordering::Relaxed);
        }
    }

    #[inline]
    fn record_deallocation(&self, size: usize) {
        self.deallocations.fetch_add(1, Ordering::Relaxed);
        self.bytes_deallocated
            .fetch_add(size as u64, Ordering::Relaxed);
    }

    #[inline]
    fn record_failure(&self) {
        self.failures.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn record_mbind_miss(&self) {
        self.mbind_misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Get total allocations.
    pub fn allocations(&self) -> u64 {
        self.allocations.load(Ordering::Relaxed)
    }

    /// Get bytes allocated.
    pub fn bytes_allocated(&self) -> u64 {
        self.bytes_allocated.load(Ordering::Relaxed)
    }

    /// Get deallocations.
    pub fn deallocations(&self) -> u64 {
        self.deallocations.load(Ordering::Relaxed)
    }

    /// Get failures.
    pub fn failures(&self) -> u64 {
        self.failures.load(Ordering::Relaxed)
    }

    /// Get allocations that were kept without NUMA placement because
    /// `mbind` failed or is unavailable on this platform.
    pub fn mbind_misses(&self) -> u64 {
        self.mbind_misses.load(Ordering::Relaxed)
    }

    /// Get allocations per node.
    pub fn allocations_on_node(&self, node: usize) -> u64 {
        if node < 8 {
            self.per_node[node].load(Ordering::Relaxed)
        } else {
            0
        }
    }
}

/// Global NUMA statistics.
#[derive(Debug, Default)]
pub struct GlobalNumaStats {
    /// Worker NUMA initializations
    inits: AtomicU64,
    /// NUMA bindings (successful)
    binds_success: AtomicU64,
    /// NUMA bindings (failed)
    binds_failed: AtomicU64,
}

impl GlobalNumaStats {
    /// Create new stats.
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    fn record_init(&self) {
        self.inits.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn record_bind(&self, success: bool) {
        if success {
            self.binds_success.fetch_add(1, Ordering::Relaxed);
        } else {
            self.binds_failed.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Get initialization count.
    pub fn inits(&self) -> u64 {
        self.inits.load(Ordering::Relaxed)
    }

    /// Get successful binds.
    pub fn binds_success(&self) -> u64 {
        self.binds_success.load(Ordering::Relaxed)
    }

    /// Get failed binds.
    pub fn binds_failed(&self) -> u64 {
        self.binds_failed.load(Ordering::Relaxed)
    }
}

/// Global NUMA statistics.
static NUMA_STATS: GlobalNumaStats = GlobalNumaStats {
    inits: AtomicU64::new(0),
    binds_success: AtomicU64::new(0),
    binds_failed: AtomicU64::new(0),
};

/// Get global NUMA statistics.
pub fn numa_stats() -> &'static GlobalNumaStats {
    &NUMA_STATS
}

// ============================================================================
// Cached NUMA Configuration
// ============================================================================

static NUMA_CONFIG: OnceLock<NumaConfig> = OnceLock::new();

/// Get cached NUMA configuration.
///
/// This detects the system's NUMA topology once and caches it.
pub fn cached_numa_config() -> &'static NumaConfig {
    NUMA_CONFIG.get_or_init(NumaConfig::detect)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_numa_available() {
        // The Linux implementation promises exactly one thing: sysfs exposes a
        // second NUMA node. Pin that relationship rather than the raw value,
        // which varies with the host, so a stubbed-out detector can't pass.
        #[cfg(target_os = "linux")]
        {
            let has_node1 = std::path::Path::new("/sys/devices/system/node/node1").exists();
            assert_eq!(numa_available(), has_node1);
        }

        // Non-Linux has no NUMA support at all, so the answer is unconditional.
        #[cfg(not(target_os = "linux"))]
        {
            assert!(!numa_available());
        }
    }

    #[test]
    fn test_num_numa_nodes() {
        let nodes = num_numa_nodes();
        assert!(nodes >= 1);
    }

    #[test]
    fn test_numa_node_basic() {
        let node = NumaNode::new(0);
        assert_eq!(node.id(), 0);
    }

    #[test]
    fn test_numa_node_current() {
        let node = NumaNode::current();
        // Current node should be valid
        assert!(node.id() < num_numa_nodes() || num_numa_nodes() == 1);
    }

    #[test]
    fn test_numa_node_all() {
        let nodes = NumaNode::all();
        assert_eq!(nodes.len(), num_numa_nodes());
    }

    #[test]
    fn test_numa_config_detect() {
        let config = NumaConfig::detect();
        assert!(config.num_nodes() >= 1);
        assert_eq!(config.nodes().len(), config.num_nodes());
    }

    #[test]
    fn test_numa_config_node_for_worker() {
        let config = NumaConfig::detect();

        // Workers should be distributed across nodes
        let node0 = config.node_for_worker(0);
        let node1 = config.node_for_worker(config.num_nodes());

        // Worker 0 and worker N should map to same node (round-robin)
        assert_eq!(node0.id(), node1.id());
    }

    #[test]
    fn test_numa_policy_default() {
        let policy = NumaPolicy::default();
        assert_eq!(policy, NumaPolicy::Local);
    }

    #[test]
    fn test_numa_buffer_allocation() {
        if let Some(buffer) = NumaBuffer::new(1024) {
            assert_eq!(buffer.size(), 1024);
            assert!(buffer.node().id() < num_numa_nodes() || num_numa_nodes() == 1);
        }
        // May fail on systems without NUMA or memory pressure
    }

    #[test]
    fn test_numa_buffer_zero_initialized() {
        // Fresh buffers must be zeroed: as_slice() would otherwise expose
        // uninitialized memory (UB).
        if let Some(buffer) = NumaBuffer::new(4096) {
            assert!(buffer.as_slice().iter().all(|&b| b == 0));
        }
    }

    #[test]
    fn test_numa_buffer_read_write() {
        if let Some(mut buffer) = NumaBuffer::new(64) {
            let slice = buffer.as_mut_slice();
            slice[0] = 42;
            slice[63] = 99;

            let read_slice = buffer.as_slice();
            assert_eq!(read_slice[0], 42);
            assert_eq!(read_slice[63], 99);
        }
    }

    /// Reset the calling thread's memory policy to the kernel default so a
    /// test doesn't leave its thread bound.
    #[cfg(target_os = "linux")]
    fn reset_mempolicy() {
        let _ = set_mempolicy(mempolicy::MPOL_DEFAULT, None, 0);
    }

    #[test]
    fn test_bind_to_local_node() {
        match bind_to_local_node() {
            Ok(()) => {
                #[cfg(target_os = "linux")]
                reset_mempolicy();
            }
            Err(NumaError::NotSupported) => {
                // Non-Linux platform, or kernel without NUMA syscalls (ENOSYS).
                eprintln!("skipping: NUMA binding not supported here");
            }
            Err(e) => panic!("bind_to_local_node failed unexpectedly: {e}"),
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_bind_to_node_zero() {
        // Binding to node 0 with MPOL_BIND should succeed on any Linux with
        // NUMA syscalls compiled in, even single-node machines.
        match bind_to_node(NumaNode::new(0)) {
            Ok(()) => reset_mempolicy(),
            Err(NumaError::NotSupported) => {
                eprintln!("skipping: set_mempolicy returned ENOSYS");
            }
            Err(e) => panic!("bind_to_node(0) failed unexpectedly: {e}"),
        }
    }

    #[test]
    fn test_bind_to_invalid_node() {
        let result = bind_to_node(NumaNode::new(usize::MAX));
        assert!(result.is_err());
        #[cfg(target_os = "linux")]
        assert!(matches!(result, Err(NumaError::InvalidNode { .. })));
        #[cfg(not(target_os = "linux"))]
        assert!(matches!(result, Err(NumaError::NotSupported)));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn test_bind_not_supported_off_linux() {
        // Binding must not pretend success on non-Linux platforms.
        assert!(matches!(
            bind_to_node(NumaNode::new(0)),
            Err(NumaError::NotSupported)
        ));
        assert!(matches!(bind_to_local_node(), Err(NumaError::NotSupported)));
    }

    #[test]
    fn test_numa_allocator_placement() {
        let allocator = NumaAllocator::new(NumaNode::new(0));
        let size = 8192;
        if let Some(ptr) = allocator.allocate(size) {
            // On Linux the allocation must be page-aligned (mbind requirement).
            assert_eq!(ptr as usize % numa_alloc_align(), 0);
            assert_eq!(allocator.stats().allocations(), 1);
            assert_eq!(allocator.stats().bytes_allocated(), size as u64);
            // mbind either succeeded (0 misses) or degraded gracefully (1).
            assert!(allocator.stats().mbind_misses() <= 1);
            unsafe { allocator.deallocate(ptr, size) };
            assert_eq!(allocator.stats().deallocations(), 1);
        }
    }

    #[test]
    fn test_init_worker_numa_degrades_when_bind_unsupported() {
        // A config that reports as NUMA (num_nodes > 1) but whose nodes all
        // point at node 0, so bind_to_node never trips InvalidNode. Regardless
        // of whether this host actually permits the bind syscall,
        // init_worker_numa must return Ok: on a multi-node host where the
        // syscall is blocked (seccomp/container) it degrades gracefully with a
        // warning instead of failing worker init.
        let config = NumaConfig {
            num_nodes: 2,
            nodes: vec![NumaNode::new(0), NumaNode::new(0)],
            total_memory: 0,
            policy: NumaPolicy::Local,
        };
        assert!(config.is_numa());

        let result = init_worker_numa(0, &config);
        assert!(
            result.is_ok(),
            "init_worker_numa must not error when NUMA binding is unsupported: {result:?}"
        );
        assert_eq!(result.unwrap().id(), 0);

        // If a bind actually took effect, don't leave this thread bound.
        #[cfg(target_os = "linux")]
        reset_mempolicy();
    }

    #[test]
    fn test_numa_error_display() {
        let err1 = NumaError::InvalidNode { node: 10, max: 3 };
        assert!(err1.to_string().contains("10"));

        let err2 = NumaError::NotAvailable;
        assert!(err2.to_string().contains("not available"));

        let err3 = NumaError::NotSupported;
        assert!(err3.to_string().contains("not supported"));
    }

    #[test]
    fn test_numa_stats() {
        let stats = numa_stats();
        let _ = stats.inits();
        let _ = stats.binds_success();
        let _ = stats.binds_failed();
    }

    #[test]
    fn test_cached_numa_config() {
        let config1 = cached_numa_config();
        let config2 = cached_numa_config();
        // Should return same reference
        assert_eq!(config1.num_nodes(), config2.num_nodes());
    }

    #[test]
    fn test_numa_alloc_stats() {
        let stats = NumaAllocStats::new();
        stats.record_allocation(1024, 0);
        assert_eq!(stats.allocations(), 1);
        assert_eq!(stats.bytes_allocated(), 1024);
        assert_eq!(stats.allocations_on_node(0), 1);
    }
}
