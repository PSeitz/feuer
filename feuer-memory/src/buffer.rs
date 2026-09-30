//! Aligned allocations and idle buffers sharing the memory cache's byte budget.

use std::{
    alloc::{Layout, alloc_zeroed, dealloc},
    io,
    ptr::NonNull,
    sync::{Arc, LazyLock, Weak},
};

use bytes::Bytes;
use feuer_types::config::read_env_number;
use parking_lot::Mutex;

use crate::MemoryMetrics;

/// Alignment of buffers used for direct disk I/O.
pub const BUFFER_ALIGNMENT: usize = 4096;
pub(crate) const BUFFER_SIZES: [usize; 6] = [
    32 * 1024,
    256 * 1024,
    4 * 1024 * 1024,
    16 * 1024 * 1024,
    32 * 1024 * 1024,
    64 * 1024 * 1024,
];

/// Maximum share of memory capacity retained as idle buffers across all buckets.
static IDLE_BUFFER_POOL_PERCENT: LazyLock<u64> = LazyLock::new(|| {
    let percent = read_env_number("FEUER_IDLE_BUFFER_POOL_PERCENT", 7, 0).unwrap_or_else(|error| panic!("{error}"));
    assert!(
        percent <= 100,
        "FEUER_IDLE_BUFFER_POOL_PERCENT must be between 0 and 100"
    );
    percent
});

/// Idle aligned buffers and the cached-allocation charges sharing one cache's budget.
/// Active allocations and caller-only results are not charged to this budget.
pub struct BufferPool {
    capacity: u64,
    idle_limit: u64,
    state: Mutex<RetainedBuffers>,
    metrics: Arc<MemoryMetrics>,
}

/// Cached allocation bytes and idle allocations, protected by the same lock.
struct RetainedBuffers {
    cached_bytes: u64,
    idle_bytes: u64,
    by_size: [Vec<AlignedBuffer>; BUFFER_SIZES.len()],
}

impl BufferPool {
    pub(crate) fn new(capacity: u64, metrics: Arc<MemoryMetrics>) -> Arc<Self> {
        let idle_limit = (u128::from(capacity) * u128::from(*IDLE_BUFFER_POOL_PERCENT) / 100) as u64;
        metrics.capacity_bytes.increase(capacity);
        Arc::new(Self {
            capacity,
            idle_limit,
            state: Mutex::new(RetainedBuffers {
                cached_bytes: 0,
                idle_bytes: 0,
                by_size: std::array::from_fn(|_| Vec::new()),
            }),
            metrics,
        })
    }

    /// A standalone storage reader without a memory cache retains no idle buffers.
    pub fn unpooled() -> Arc<Self> {
        Self::new(0, MemoryMetrics::noop())
    }

    /// Takes or allocates a buffer. Larger than 64 MiB is exact-size and unpooled.
    /// Reused bytes are initialized, but must be overwritten before returning a read result.
    pub fn allocate(self: &Arc<Self>, length: usize) -> io::Result<AlignedBuffer> {
        assert!(length > 0);
        let bucket = BUFFER_SIZES.iter().position(|&size| length <= size);
        let capacity = bucket.map_or(length, |index| BUFFER_SIZES[index]);
        let reused = bucket.and_then(|index| {
            let mut state = self.state.lock();
            let buffer = state.by_size[index].pop();
            if buffer.is_some() {
                state.idle_bytes -= capacity as u64;
                self.remove_idle_usage(index, capacity as u64);
            }
            buffer
        });
        let mut buffer = match reused {
            Some(buffer) => buffer,
            None => AlignedBuffer::allocate_zeroed(capacity)?,
        };
        buffer.length = length;
        if bucket.is_some() {
            buffer.pool = Arc::downgrade(self);
        }
        Ok(buffer)
    }

    /// Allocation bytes retained as cached entries or idle buffers.
    pub fn used_bytes(&self) -> u64 {
        let state = self.state.lock();
        state.cached_bytes + state.idle_bytes
    }

    /// Allocation bytes currently available for reuse.
    pub fn idle_bytes(&self) -> u64 {
        self.state.lock().idle_bytes
    }

    pub(crate) fn add_cached(&self, bytes: u64) {
        let mut state = self.state.lock();
        state.cached_bytes += bytes;
        let idle_limit = self.capacity.saturating_sub(state.cached_bytes);
        // Cache entries take precedence over idle buffers. Free larger buffers first.
        for index in (0..BUFFER_SIZES.len()).rev() {
            while state.idle_bytes > idle_limit {
                let Some(buffer) = state.by_size[index].pop() else {
                    break;
                };
                let capacity = buffer.capacity() as u64;
                state.idle_bytes -= capacity;
                self.remove_idle_usage(index, capacity);
                // Idle buffers have no pool reference; dropping them frees their allocations.
                drop(buffer);
            }
        }
    }

    pub(crate) fn remove_cached(&self, bytes: u64) {
        self.state.lock().cached_bytes -= bytes;
    }

    fn remove_idle_usage(&self, index: usize, bytes: u64) {
        self.metrics.idle_buffer_bytes[index].decrease(bytes);
        self.metrics.decrease_usage(bytes, 0);
    }
}

impl Drop for BufferPool {
    fn drop(&mut self) {
        for (index, &size) in BUFFER_SIZES.iter().enumerate() {
            let bytes = self.state.get_mut().by_size[index].len() as u64 * size as u64;
            self.remove_idle_usage(index, bytes);
        }
        self.metrics.capacity_bytes.decrease(self.capacity);
    }
}

/// One owned, initialized, 4-KiB-aligned allocation with a separate exposed length.
/// It returns to its originating pool only after the last Bytes owner releases it.
pub struct AlignedBuffer {
    ptr: NonNull<u8>,
    layout: Layout,
    length: usize,
    pool: Weak<BufferPool>,
}

impl AlignedBuffer {
    /// Allocates zeroed memory that is freed, not pooled, on release (used for write scratch).
    pub fn allocate_zeroed(length: usize) -> io::Result<Self> {
        assert!(length > 0);
        let layout = Layout::from_size_align(length, BUFFER_ALIGNMENT)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "buffer length exceeds allocation limit"))?;
        // SAFETY: layout is nonzero with a valid power-of-two alignment.
        let ptr = NonNull::new(unsafe { alloc_zeroed(layout) })
            .ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "aligned buffer allocation failed"))?;
        Ok(Self {
            ptr,
            layout,
            length,
            pool: Weak::new(),
        })
    }

    /// Actual backing allocation bytes, including spare capacity.
    pub fn capacity(&self) -> usize {
        self.layout.size()
    }

    /// Transfers this allocation to reference-counted bytes without copying.
    pub fn into_bytes(self) -> Bytes {
        Bytes::from_owner(self)
    }

    /// Exclusively accesses the requested bytes, never spare capacity.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: this initialized allocation is exclusively owned by self.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.length) }
    }
}

impl AsRef<[u8]> for AlignedBuffer {
    fn as_ref(&self) -> &[u8] {
        // SAFETY: self owns initialized bytes for the exposed length.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.length) }
    }
}

// SAFETY: moving the unique owner does not move or expose its allocation.
unsafe impl Send for AlignedBuffer {}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.upgrade() {
            let mut state = pool.state.lock();
            let capacity = self.capacity() as u64;
            let index = BUFFER_SIZES.iter().position(|&size| size == self.capacity()).unwrap();
            let available = pool.capacity.saturating_sub(state.cached_bytes + state.idle_bytes);
            if capacity <= available && state.idle_bytes + capacity <= pool.idle_limit {
                state.by_size[index].push(Self {
                    ptr: self.ptr,
                    layout: self.layout,
                    length: self.length,
                    pool: Weak::new(),
                });
                state.idle_bytes += capacity;
                pool.metrics.idle_buffer_bytes[index].increase(capacity);
                pool.metrics.increase_usage(capacity, 0);
                pool.metrics.returned_buffers[index].increase(1);
                return;
            }
            pool.metrics.dropped_buffers[index].increase(1);
        }
        // SAFETY: this owner holds the allocation made with exactly this layout.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) };
    }
}

#[cfg(test)]
mod tests;
