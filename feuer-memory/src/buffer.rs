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
pub(crate) const BUFFER_SIZES: [usize; 13] = [
    32 * 1024,
    256 * 1024,
    512 * 1024,
    1024 * 1024,
    2 * 1024 * 1024,
    3 * 1024 * 1024,
    4 * 1024 * 1024,
    8 * 1024 * 1024,
    12 * 1024 * 1024,
    16 * 1024 * 1024,
    24 * 1024 * 1024,
    32 * 1024 * 1024,
    64 * 1024 * 1024,
];

/// Maximum share of memory capacity occupied by idle buffers across all buckets.
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
    pub(crate) capacity: u64,
    idle_limit: u64,
    state: Mutex<EntryAllocationBytesAndIdleBuffers>,
    pub(crate) metrics: Arc<MemoryMetrics>,
}

/// Entry allocation byte counts and idle buffers grouped by size, protected by one lock.
#[derive(Default)]
struct EntryAllocationBytesAndIdleBuffers {
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
            state: Mutex::default(),
            metrics,
        })
    }

    /// A standalone storage reader without a memory cache retains no idle buffers.
    pub fn unpooled() -> Arc<Self> {
        Self::new(0, MemoryMetrics::noop())
    }

    /// Backing capacity for a length, without allocating or taking an idle buffer.
    pub fn allocation_capacity(length: usize) -> usize {
        BUFFER_SIZES.into_iter().find(|&size| length <= size).unwrap_or(length)
    }

    /// Takes or allocates a buffer. Larger than 64 MiB is exact-size and unpooled.
    /// Reused bytes are initialized, but must be overwritten before returning a read result.
    pub fn allocate(self: &Arc<Self>, length: usize) -> io::Result<AlignedBuffer> {
        let bucket = BUFFER_SIZES.iter().position(|&size| length <= size);
        let capacity = bucket.map(|index| BUFFER_SIZES[index]).unwrap_or(length);
        let mut buffer = bucket
            .and_then(|index| self.take_idle_buffer(&mut self.state.lock(), index))
            .map_or_else(|| AlignedBuffer::allocate_zeroed(capacity), Ok)?;
        buffer.length = length;
        if let Some(index) = bucket {
            buffer.pool = Some((Arc::downgrade(self), self.metrics.clone()));
            self.metrics.used_buffer_bytes[index].increase(capacity as u64);
        }
        Ok(buffer)
    }

    /// Entry allocation charges plus idle buffer capacity.
    pub fn used_bytes(&self) -> u64 {
        let state = self.state.lock();
        state.cached_bytes + state.idle_bytes
    }

    /// Allocation bytes currently available for reuse.
    pub fn idle_bytes(&self) -> u64 {
        self.state.lock().idle_bytes
    }

    /// Adds entry-allocation bytes to accounting and metrics, freeing idle buffers if needed.
    pub(crate) fn add_entry_bytes(&self, bytes: u64) {
        let mut state = self.state.lock();
        state.cached_bytes += bytes;
        self.metrics.used_bytes.increase(bytes);
        let idle_limit = self.capacity.saturating_sub(state.cached_bytes);
        // Cache entries take precedence over idle buffers. Free larger buffers first.
        for index in (0..BUFFER_SIZES.len()).rev() {
            while state.idle_bytes > idle_limit
                && let Some(buffer) = self.take_idle_buffer(&mut state, index)
            {
                // Idle buffers have no pool reference; dropping them frees their allocations.
                drop(buffer);
            }
        }
    }

    /// Subtracts entry-allocation bytes from accounting and metrics.
    pub(crate) fn remove_entry_bytes(&self, bytes: u64) {
        self.state.lock().cached_bytes -= bytes;
        self.metrics.used_bytes.decrease(bytes);
    }

    /// Takes one idle buffer and subtracts its bytes from accounting and metrics under the pool lock.
    fn take_idle_buffer(&self, state: &mut EntryAllocationBytesAndIdleBuffers, index: usize) -> Option<AlignedBuffer> {
        let buffer = state.by_size[index].pop()?;
        let capacity = buffer.capacity() as u64;
        state.idle_bytes -= capacity;
        self.decrease_idle_buffer_metrics(index, capacity);
        Some(buffer)
    }

    /// Decreases metrics for idle-buffer bytes.
    fn decrease_idle_buffer_metrics(&self, index: usize, bytes: u64) {
        self.metrics.idle_buffer_bytes[index].decrease(bytes);
        self.metrics.used_bytes.decrease(bytes);
    }
}

impl Drop for BufferPool {
    fn drop(&mut self) {
        for (index, &size) in BUFFER_SIZES.iter().enumerate() {
            let bytes = self.state.get_mut().by_size[index].len() as u64 * size as u64;
            self.decrease_idle_buffer_metrics(index, bytes);
        }
        self.metrics.capacity_bytes.decrease(self.capacity);
    }
}

/// One owned, initialized, 4-KiB-aligned allocation with a separate exposed length.
/// It returns to its originating pool only after the last Bytes owner drops it.
///
/// This buffer is strongly aligned with my values (performance)
pub struct AlignedBuffer {
    ptr: NonNull<u8>,
    layout: Layout,
    length: usize,
    // Checked-out pooled buffers retain metrics, but only a weak reference to their pool.
    pool: Option<(Weak<BufferPool>, Arc<MemoryMetrics>)>,
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
            pool: None,
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
        if let Some((pool, metrics)) = self.pool.take() {
            let index = BUFFER_SIZES.iter().position(|&size| size == self.capacity()).unwrap();
            let capacity = self.capacity() as u64;
            metrics.used_buffer_bytes[index].decrease(capacity);
            if let Some(pool) = pool.upgrade() {
                let mut state = pool.state.lock();
                let idle_limit = pool.idle_limit.min(pool.capacity.saturating_sub(state.cached_bytes));
                if state.idle_bytes + capacity <= idle_limit {
                    state.by_size[index].push(Self {
                        ptr: self.ptr,
                        layout: self.layout,
                        length: self.length,
                        pool: None,
                    });
                    state.idle_bytes += capacity;
                    pool.metrics.idle_buffer_bytes[index].increase(capacity);
                    pool.metrics.used_bytes.increase(capacity);
                    return;
                }
            }
        }
        // SAFETY: this owner holds the allocation made with exactly this layout.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) };
    }
}

#[cfg(test)]
mod tests;
