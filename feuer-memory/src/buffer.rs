//! Aligned allocations and idle buffers sharing the memory cache's byte budget.

use std::{
    alloc::{Layout, alloc_zeroed, dealloc},
    io,
    mem::ManuallyDrop,
    num::NonZeroUsize,
    ptr::NonNull,
    slice,
    sync::{Arc, Weak},
};

#[cfg(not(target_os = "linux"))]
use std::alloc::realloc;
#[cfg(target_os = "linux")]
use std::ptr::null_mut;

use bytes::Bytes;
#[cfg(target_os = "linux")]
use libc::{
    _SC_PAGESIZE, MAP_ANONYMOUS, MAP_FAILED, MAP_PRIVATE, MREMAP_MAYMOVE, PROT_READ, PROT_WRITE, mmap, mremap, munmap,
    sysconf,
};
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
    BUFFER_SIZES.partition_point(|&size| size < length)
}

/// Idle aligned buffers and allocation bytes charged to cached objects share one cache's budget.
/// Checked-out buffers are charged when admitted to the cache.
pub struct BufferPool {
    pub(crate) capacity: u64,
    inner: Mutex<BufferPoolInner>,
    pub(crate) metrics: Arc<MemoryMetrics>,
}

/// Cached-byte accounting and idle buffers, updated under one lock.
#[derive(Default)]
struct BufferPoolInner {
    /// Allocation bytes charged to cached objects.
    used_bytes: u64,
    idle_bytes: u64,
    idle_buffers: [Vec<AlignedBuffer>; BUCKET_COUNT],
}

impl BufferPool {
    pub(crate) fn new(capacity: u64, metrics: Arc<MemoryMetrics>) -> Arc<Self> {
        metrics.capacity_bytes.increase(capacity);
        Arc::new(Self {
            capacity,
            inner: Mutex::default(),
            metrics,
        })
    }

    /// A storage reader without a memory cache retains no idle buffers.
    pub fn unpooled() -> Arc<Self> {
        Self::new(0, MemoryMetrics::noop())
    }

    /// Fresh backing capacity for a length. Reuse may retain less than 10% spare capacity.
    pub fn allocation_capacity(length: usize) -> usize {
        BUFFER_SIZES.get(bucket_index(length)).copied().unwrap_or(length)
    }

    /// Takes or allocates a buffer. Above 64 MiB, reuse grows to fit and shrinks only
    /// when the idle buffer's capacity is at least 10% larger than the requested length.
    /// Reused bytes are initialized, but must be overwritten before returning a read result.
    pub fn allocate(self: &Arc<Self>, length: usize) -> io::Result<AlignedBuffer> {
        let index = bucket_index(length);
        let capacity = Self::allocation_capacity(length);
        let idle_buffer = self.take_idle_buffer(&mut self.inner.lock(), index);
        let mut buffer = match idle_buffer {
            Some(mut buffer) => {
                if let BufferMemory::Large(memory) = &mut *buffer.memory {
                    memory.resize_for_reuse(NonZeroUsize::new(capacity).unwrap())?;
                }
                buffer
            }
            None => AlignedBuffer::allocate_zeroed(capacity)?,
        };
        buffer.length = length;
        buffer.pool = Some((Arc::downgrade(self), self.metrics.clone()));
        self.metrics.used_buffer_bytes[index].increase(buffer.capacity() as u64);
        Ok(buffer)
    }

    /// Allocation bytes charged to cached objects plus idle buffer capacity.
    pub fn used_bytes(&self) -> u64 {
        let inner = self.inner.lock();
        inner.used_bytes + inner.idle_bytes
    }

    /// Allocation bytes available for reuse.
    pub fn idle_bytes(&self) -> u64 {
        self.inner.lock().idle_bytes
    }

    /// Adds allocation bytes charged to cached objects, freeing idle buffers if needed.
    pub(crate) fn add_used_bytes(&self, bytes: u64) {
        let mut inner = self.inner.lock();
        inner.used_bytes += bytes;
        self.metrics.used_bytes.increase(bytes);
        let remaining_capacity = self.capacity.saturating_sub(inner.used_bytes);
        // Cached objects take precedence over idle buffers. Free larger buffers first.
        for index in (0..BUCKET_COUNT).rev() {
            while inner.idle_bytes > remaining_capacity
                && let Some(buffer) = self.take_idle_buffer(&mut inner, index)
            {
                // Idle buffers have no pool reference. Dropping them frees their allocations.
                drop(buffer);
            }
        }
    }

    /// Subtracts allocation bytes charged to cached objects.
    pub(crate) fn remove_used_bytes(&self, bytes: u64) {
        self.inner.lock().used_bytes -= bytes;
        self.metrics.used_bytes.decrease(bytes);
    }

    /// Takes an idle buffer and subtracts its bytes from accounting and metrics under the pool lock.
    fn take_idle_buffer(&self, inner: &mut BufferPoolInner, index: usize) -> Option<AlignedBuffer> {
        let buffer = inner.idle_buffers[index].pop()?;
        let capacity = buffer.capacity() as u64;
        inner.idle_bytes -= capacity;
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
            let buffers = &self.inner.get_mut().idle_buffers[index];
            let bytes = buffers.iter().map(|buffer| buffer.capacity() as u64).sum();
            self.decrease_idle_buffer_metrics(index, bytes);
        }
        self.metrics.capacity_bytes.decrease(self.capacity);
    }
}

/// An owned, initialized, 4-KiB-aligned allocation with a separate exposed length.
/// After conversion to `Bytes`, it returns to its pool only when the last owner drops it.
/// Fixed buffers keep their capacity. Large buffers can resize and use anonymous mappings on Linux.
pub struct AlignedBuffer {
    memory: ManuallyDrop<BufferMemory>,
    length: usize,
    // Checked-out pooled buffers retain metrics, but only a weak reference to their pool.
    pool: Option<(Weak<BufferPool>, Arc<MemoryMetrics>)>,
}

/// Backing memory and the information required to release it.
enum BufferMemory {
    Fixed(FixedBuffer),
    Large(LargeBuffer),
}

impl BufferMemory {
    fn ptr(&self) -> NonNull<u8> {
        match self {
            Self::Fixed(buffer) => buffer.ptr,
            Self::Large(buffer) => buffer.ptr,
        }
    }

    fn capacity(&self) -> usize {
        match self {
            Self::Fixed(buffer) => buffer.layout.size(),
            Self::Large(buffer) => buffer.capacity(),
        }
    }
}

/// Allocator-owned backing memory whose capacity stays unchanged.
struct FixedBuffer {
    ptr: NonNull<u8>,
    layout: Layout,
}

impl Drop for FixedBuffer {
    fn drop(&mut self) {
        // SAFETY: this owner holds the allocation made with exactly this layout.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) };
    }
}

/// Resizable backing memory for buffers above the largest fixed bucket.
/// Linux uses an anonymous mapping. Other platforms use the allocator.
struct LargeBuffer {
    ptr: NonNull<u8>,
    #[cfg(target_os = "linux")]
    length: NonZeroUsize,
    #[cfg(not(target_os = "linux"))]
    layout: Layout,
}

impl LargeBuffer {
    fn allocate_zeroed(length: NonZeroUsize) -> io::Result<Self> {
        let layout = AlignedBuffer::layout(length.get())?;
        #[cfg(target_os = "linux")]
        {
            // SAFETY: length is nonzero and valid. Anonymous private pages are initialized to zero.
            let ptr = unsafe {
                mmap(
                    null_mut(),
                    layout.size(),
                    PROT_READ | PROT_WRITE,
                    MAP_PRIVATE | MAP_ANONYMOUS,
                    -1,
                    0,
                )
            };
            if ptr == MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                ptr: NonNull::new(ptr.cast()).expect("mmap returned a null address"),
                length,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            // SAFETY: layout is nonzero with a valid power-of-two alignment.
            let ptr = NonNull::new(unsafe { alloc_zeroed(layout) })
                .ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "aligned buffer allocation failed"))?;
            Ok(Self { ptr, layout })
        }
    }

    fn capacity(&self) -> usize {
        #[cfg(target_os = "linux")]
        {
            self.length.get()
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.layout.size()
        }
    }

    /// Preserves initialized bytes and zeroes added bytes. Shrinks only when the old capacity
    /// is at least 10% larger than the requested capacity. Failure leaves the memory unchanged.
    fn resize_for_reuse(&mut self, capacity: NonZeroUsize) -> io::Result<()> {
        let old_capacity = self.capacity();
        let new_capacity = capacity.get();
        if new_capacity <= old_capacity {
            let spare_bytes = old_capacity - new_capacity;
            // Shrink only when spare bytes reach 10% of the requested capacity.
            let minimum_shrink_bytes = new_capacity.div_ceil(10);
            if spare_bytes < minimum_shrink_bytes {
                return Ok(());
            }
        }
        let layout = AlignedBuffer::layout(new_capacity)?;
        #[cfg(target_os = "linux")]
        {
            // SAFETY: this buffer owns an anonymous mapping. Both sizes are nonzero and valid.
            let resized = unsafe {
                mremap(
                    self.ptr.as_ptr().cast(),
                    self.length.get(),
                    layout.size(),
                    MREMAP_MAYMOVE,
                )
            };
            if resized == MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            self.ptr = NonNull::new(resized.cast()).expect("mremap returned a null address");
            self.length = capacity;
            if new_capacity > old_capacity {
                // New pages are zeroed by the kernel. Only the retained last page can contain
                // old bytes beyond the previous capacity, after a shrink and subsequent growth.
                // SAFETY: sysconf queries the process's page size.
                let page_size = unsafe { sysconf(_SC_PAGESIZE) } as usize;
                let end = old_capacity.next_multiple_of(page_size).min(new_capacity);
                // SAFETY: these bytes belong to the resized mapping.
                unsafe { self.ptr.add(old_capacity).write_bytes(0, end - old_capacity) };
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            // SAFETY: ptr was allocated with self.layout. The new size is nonzero and valid.
            let resized = NonNull::new(unsafe { realloc(self.ptr.as_ptr(), self.layout, new_capacity) })
                .ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "aligned buffer resize failed"))?;
            self.ptr = resized;
            self.layout = layout;
            if new_capacity > old_capacity {
                // SAFETY: bytes past the old capacity belong to the new allocation.
                unsafe { self.ptr.add(old_capacity).write_bytes(0, new_capacity - old_capacity) };
            }
        }
        Ok(())
    }
}

impl Drop for LargeBuffer {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        // SAFETY: this owner holds the mapping, which the kernel rounds to whole pages.
        unsafe {
            munmap(self.ptr.as_ptr().cast(), self.length.get())
        };
        #[cfg(not(target_os = "linux"))]
        // SAFETY: this owner holds the allocation made with exactly this layout.
        unsafe {
            dealloc(self.ptr.as_ptr(), self.layout)
        };
    }
}

impl AlignedBuffer {
    /// Allocates zeroed memory that is freed on release (used for write scratch).
    pub fn allocate_zeroed(length: usize) -> io::Result<Self> {
        assert!(length > 0);
        if length > MAX_FIXED_BUFFER_BYTES {
            return Ok(Self {
                memory: ManuallyDrop::new(BufferMemory::Large(LargeBuffer::allocate_zeroed(
                    NonZeroUsize::new(length).unwrap(),
                )?)),
                length,
                pool: None,
            });
        }
        let layout = Self::layout(length)?;
        // SAFETY: layout is nonzero with a valid power-of-two alignment.
        let ptr = NonNull::new(unsafe { alloc_zeroed(layout) })
            .ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "aligned buffer allocation failed"))?;
        Ok(Self {
            memory: ManuallyDrop::new(BufferMemory::Fixed(FixedBuffer { ptr, layout })),
            length,
            pool: None,
        })
    }

    fn layout(capacity: usize) -> io::Result<Layout> {
        Layout::from_size_align(capacity, BUFFER_ALIGNMENT)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "buffer length exceeds allocation limit"))
    }

    /// Backing allocation bytes, including spare capacity.
    pub fn capacity(&self) -> usize {
        self.memory.capacity()
    }

    /// Transfers this allocation to reference-counted bytes without copying.
    pub fn into_bytes(self) -> Bytes {
        Bytes::from_owner(self)
    }

    /// Exclusively accesses the requested bytes, never spare capacity.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: this initialized allocation is exclusively owned by self.
        unsafe { slice::from_raw_parts_mut(self.memory.ptr().as_ptr(), self.length) }
    }
}

impl AsRef<[u8]> for AlignedBuffer {
    fn as_ref(&self) -> &[u8] {
        // SAFETY: self owns initialized bytes for the exposed length.
        unsafe { slice::from_raw_parts(self.memory.ptr().as_ptr(), self.length) }
    }
}

// SAFETY: moving the unique owner does not move or expose its allocation.
unsafe impl Send for AlignedBuffer {}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        // SAFETY: Drop takes the backing memory exactly once and never accesses self.memory again.
        let memory = unsafe { ManuallyDrop::take(&mut self.memory) };
        if let Some((pool, metrics)) = self.pool.take() {
            let index = bucket_index(memory.capacity());
            let capacity = memory.capacity() as u64;
            metrics.used_buffer_bytes[index].decrease(capacity);
            if let Some(pool) = pool.upgrade() {
                let mut inner = pool.inner.lock();
                let idle_limit = pool.capacity.saturating_sub(inner.used_bytes);
                if inner.idle_bytes + capacity <= idle_limit {
                    inner.idle_buffers[index].push(Self {
                        memory: ManuallyDrop::new(memory),
                        length: self.length,
                        pool: None,
                    });
                    inner.idle_bytes += capacity;
                    pool.metrics.idle_buffer_bytes[index].increase(capacity);
                    pool.metrics.used_bytes.increase(capacity);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
