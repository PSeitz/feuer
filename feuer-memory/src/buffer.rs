//! Aligned allocations and idle buffers sharing the memory cache's byte budget.

use std::{
    alloc::{Layout, alloc_zeroed, dealloc},
    io,
    ptr::NonNull,
    sync::{Arc, LazyLock, Weak},
};

#[cfg(not(target_os = "linux"))]
use std::alloc::realloc;

use bytes::Bytes;
use feuer_types::config::read_env_number;
use parking_lot::Mutex;

use crate::MemoryMetrics;

/// Alignment of buffers used for direct disk I/O.
pub const BUFFER_ALIGNMENT: usize = 4096;
/// Largest fixed capacity in the buffer pool.
const MAX_FIXED_BUFFER_BYTES: usize = 64 * 1024 * 1024;
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
    MAX_FIXED_BUFFER_BYTES,
];
/// Fixed sizes plus one bucket for allocations above the largest fixed size.
pub(crate) const BUCKET_COUNT: usize = BUFFER_SIZES.len() + 1;

/// Bucket of the smallest fixed size that fits `length`, or the bucket above the largest size.
pub(crate) fn bucket_index(length: usize) -> usize {
    BUFFER_SIZES
        .iter()
        .position(|&size| length <= size)
        .unwrap_or(BUFFER_SIZES.len())
}

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
    by_size: [Vec<AlignedBuffer>; BUCKET_COUNT],
}

impl BufferPool {
    pub(crate) fn new(capacity: u64, metrics: Arc<MemoryMetrics>) -> Arc<Self> {
        Self::with_idle_buffer_pool_percent(capacity, metrics, *IDLE_BUFFER_POOL_PERCENT)
    }

    pub(crate) fn with_idle_buffer_pool_percent(capacity: u64, metrics: Arc<MemoryMetrics>, percent: u64) -> Arc<Self> {
        assert!(percent <= 100, "idle buffer pool percent must be between 0 and 100");
        let idle_limit = (u128::from(capacity) * u128::from(percent) / 100) as u64;
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

    /// Fresh backing capacity for a length. Reuse may retain less than 10% spare capacity.
    pub fn allocation_capacity(length: usize) -> usize {
        BUFFER_SIZES.into_iter().find(|&size| length <= size).unwrap_or(length)
    }

    /// Takes or allocates a buffer. Above 64 MiB, reuse grows to fit and shrinks only
    /// when the idle buffer's capacity is at least 10% larger than the requested length.
    /// Reused bytes are initialized, but must be overwritten before returning a read result.
    pub fn allocate(self: &Arc<Self>, length: usize) -> io::Result<AlignedBuffer> {
        let index = bucket_index(length);
        let capacity = BUFFER_SIZES.get(index).copied().unwrap_or(length);
        let idle_buffer = self.take_idle_buffer(&mut self.state.lock(), index);
        let mut buffer = match idle_buffer {
            Some(buffer) => buffer.resize(capacity)?,
            None => AlignedBuffer::allocate_zeroed(capacity)?,
        };
        buffer.length = length;
        buffer.pool = Some((Arc::downgrade(self), self.metrics.clone()));
        self.metrics.used_buffer_bytes[index].increase(buffer.capacity() as u64);
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
        for index in (0..BUCKET_COUNT).rev() {
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
        for index in 0..BUCKET_COUNT {
            let buffers = &self.state.get_mut().by_size[index];
            let bytes = buffers.iter().map(|buffer| buffer.capacity() as u64).sum();
            self.decrease_idle_buffer_metrics(index, bytes);
        }
        self.metrics.capacity_bytes.decrease(self.capacity);
    }
}

/// One owned, initialized, 4-KiB-aligned allocation with a separate exposed length.
/// It returns to its originating pool only after the last Bytes owner drops it.
/// On Linux, capacities above 64 MiB use anonymous mappings. Resizing stays in that
/// bucket, so capacity also determines how the allocation is released.
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
        let layout = Self::layout(length)?;
        #[cfg(target_os = "linux")]
        if length > MAX_FIXED_BUFFER_BYTES {
            // SAFETY: length is nonzero and valid; anonymous private pages are initialized to zero.
            let ptr = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    length,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                    -1,
                    0,
                )
            };
            if ptr == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            return Ok(Self {
                ptr: NonNull::new(ptr.cast()).expect("mmap returned a null address"),
                layout,
                length,
                pool: None,
            });
        }
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

    /// Resizes a buffer in the above-64-MiB bucket, preserving initialized bytes and zeroing added bytes.
    /// Both capacities exceed 64 MiB. Shrinks only if the old capacity is at least 10% larger.
    /// On failure, the original allocation is freed when the returned error drops `self`.
    fn resize(mut self, capacity: usize) -> io::Result<Self> {
        let old_capacity = self.capacity();
        if capacity <= old_capacity && old_capacity - capacity < capacity.div_ceil(10) {
            return Ok(self);
        }
        let layout = Self::layout(capacity)?;
        #[cfg(target_os = "linux")]
        let ptr = {
            // SAFETY: this buffer owns an anonymous mapping; both sizes are nonzero and valid.
            let ptr = unsafe { libc::mremap(self.ptr.as_ptr().cast(), old_capacity, capacity, libc::MREMAP_MAYMOVE) };
            if ptr == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            let ptr = NonNull::<u8>::new(ptr.cast()).expect("mremap returned a null address");
            if capacity > old_capacity {
                // New pages are zeroed by the kernel. Only the retained last page can contain
                // old bytes beyond the previous capacity, after a shrink and subsequent growth.
                // SAFETY: sysconf queries the process's page size.
                let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
                let end = old_capacity.next_multiple_of(page_size).min(capacity);
                // SAFETY: these bytes belong to the resized mapping.
                unsafe { ptr.add(old_capacity).write_bytes(0, end - old_capacity) };
            }
            ptr
        };
        #[cfg(not(target_os = "linux"))]
        let ptr = {
            // SAFETY: ptr was allocated with self.layout; the new size is valid for its alignment.
            let ptr = NonNull::new(unsafe { realloc(self.ptr.as_ptr(), self.layout, capacity) })
                .ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "aligned buffer resize failed"))?;
            if capacity > old_capacity {
                // SAFETY: bytes past the old capacity belong to the new allocation.
                unsafe { ptr.add(old_capacity).write_bytes(0, capacity - old_capacity) };
            }
            ptr
        };
        self.ptr = ptr;
        self.layout = layout;
        Ok(self)
    }

    fn layout(length: usize) -> io::Result<Layout> {
        Layout::from_size_align(length, BUFFER_ALIGNMENT)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "buffer length exceeds allocation limit"))
    }

    /// Actual backing allocation bytes, including spare capacity.
    pub fn capacity(&self) -> usize {
        self.layout.size()
    }

    /// Transfers this allocation to reference-counted bytes without copying.
    pub fn into_bytes(self) -> Bytes {
        Bytes::from_owner(self)
    }

    /// Transfers this allocation into a download without copying, preserving its capacity charge.
    pub fn into_download(self, start: u64) -> Result<feuer_types::Download, feuer_types::DownloadError> {
        let capacity = self.capacity();
        Ok(feuer_types::Download::new(start, self.into_bytes())?.with_allocation_charge(capacity))
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
            let index = bucket_index(self.capacity());
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
        #[cfg(target_os = "linux")]
        if self.capacity() > MAX_FIXED_BUFFER_BYTES {
            // SAFETY: this owner holds the anonymous mapping, which the kernel rounds to whole pages.
            unsafe { libc::munmap(self.ptr.as_ptr().cast(), self.capacity()) };
            return;
        }
        // SAFETY: this owner holds the allocation made with exactly this layout.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) };
    }
}

#[cfg(test)]
mod tests;
