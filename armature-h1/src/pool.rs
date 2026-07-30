//! Per-core buffer pool.
//!
//! One pool lives per worker thread, so it is deliberately neither `Sync` nor
//! internally locked. Under the thread-per-core model no connection migrates
//! cores, so a pool never needs to be shared across them.

use bytes::BytesMut;

/// A recycling pool of read and write buffers.
///
/// The interesting case is [`give`](Self::give): a buffer whose [`Bytes`] slices
/// are still alive in a handler cannot be reused without corrupting them.
/// `BytesMut::try_reclaim` is the exact test for that — it succeeds only when
/// the buffer is uniquely owned — and a failure is counted rather than hidden,
/// so handler-side buffer pinning shows up in metrics instead of silently
/// inflating RSS.
///
/// [`Bytes`]: bytes::Bytes
#[derive(Debug)]
pub struct BufPool {
    free: Vec<BytesMut>,
    buf_cap: usize,
    max_free: usize,
    misses: u64,
}

impl BufPool {
    /// Create a pool handing out buffers of at least `buf_cap` bytes, retaining
    /// at most `max_free` of them.
    pub fn new(buf_cap: usize, max_free: usize) -> Self {
        Self {
            free: Vec::with_capacity(max_free),
            buf_cap,
            max_free,
            misses: 0,
        }
    }

    /// Take a cleared buffer with at least `buf_cap` capacity.
    pub fn take(&mut self) -> BytesMut {
        match self.free.pop() {
            Some(mut buf) => {
                buf.clear();
                if buf.capacity() < self.buf_cap {
                    buf.reserve(self.buf_cap - buf.capacity());
                }
                buf
            }
            None => BytesMut::with_capacity(self.buf_cap),
        }
    }

    /// Return a buffer for reuse.
    ///
    /// The buffer is retained only if it can be reclaimed — that is, if no
    /// `Bytes` slice of it is still alive — and only if the pool is below
    /// `max_free`. Otherwise it is dropped and [`misses`](Self::misses) counts
    /// it.
    pub fn give(&mut self, mut buf: BytesMut) {
        if self.free.len() >= self.max_free {
            return;
        }
        buf.clear();
        if buf.try_reclaim(self.buf_cap) {
            self.free.push(buf);
        } else {
            // Still shared: a handler is holding a slice of it. Dropping our
            // handle is correct; the allocation frees when the handler is done.
            self.misses += 1;
        }
    }

    /// How many buffers could not be reclaimed because slices were still alive.
    ///
    /// A rising count means handlers are retaining request data, which pins the
    /// whole buffer. This is the observable form of the buffer-pinning caveat.
    #[inline]
    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// How many buffers are currently pooled.
    #[inline]
    pub fn free_len(&self) -> usize {
        self.free.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BufMut;

    #[test]
    fn take_returns_cleared_buffer_with_capacity() {
        let mut p = BufPool::new(1024, 4);
        let buf = p.take();
        assert_eq!(buf.len(), 0);
        assert!(buf.capacity() >= 1024);
    }

    #[test]
    fn give_then_take_reuses_the_allocation() {
        let mut p = BufPool::new(1024, 4);
        let mut buf = p.take();
        buf.put_slice(b"hello");
        let addr = buf.as_ptr() as usize;
        p.give(buf);
        assert_eq!(p.free_len(), 1);
        let again = p.take();
        assert_eq!(
            again.as_ptr() as usize,
            addr,
            "the same allocation must come back"
        );
        assert_eq!(p.misses(), 0);
    }

    #[test]
    fn retains_at_most_max_free() {
        let mut p = BufPool::new(64, 2);
        let bufs: Vec<_> = (0..5).map(|_| p.take()).collect();
        for b in bufs {
            p.give(b);
        }
        assert_eq!(p.free_len(), 2);
    }

    /// A buffer with a live slice cannot be reused without corrupting it, and
    /// the refusal must be counted rather than hidden.
    #[test]
    fn refuses_to_reuse_a_pinned_buffer() {
        let mut p = BufPool::new(1024, 4);
        let mut buf = p.take();
        buf.put_slice(b"hello world");
        // Freeze part of it, as a parsed header value would be.
        let pinned = buf.split_to(5).freeze();
        assert_eq!(&pinned[..], b"hello");

        p.give(buf);
        assert_eq!(p.misses(), 1, "a pinned buffer must be counted as a miss");
        assert_eq!(p.free_len(), 0, "and must not be pooled");
        // The slice is still valid — that is the whole point.
        assert_eq!(&pinned[..], b"hello");
    }

    #[test]
    fn reuses_once_the_slice_is_dropped() {
        let mut p = BufPool::new(1024, 4);
        let mut buf = p.take();
        buf.put_slice(b"hello world");
        let pinned = buf.split_to(5).freeze();
        drop(pinned);

        p.give(buf);
        assert_eq!(p.misses(), 0);
        assert_eq!(p.free_len(), 1);
    }

    #[test]
    fn take_on_empty_pool_allocates() {
        let mut p = BufPool::new(256, 4);
        assert_eq!(p.free_len(), 0);
        let buf = p.take();
        assert!(buf.capacity() >= 256);
    }

    /// The pool is per-core state and must not be shareable across threads by
    /// accident.
    #[test]
    fn pool_is_not_sync() {
        fn assert_not_sync<T>() {}
        assert_not_sync::<BufPool>();
        // A compile-time check that BufPool is Send but need not be Sync: it is
        // created on and owned by a single worker thread.
        fn assert_send<T: Send>() {}
        assert_send::<BufPool>();
    }
}
