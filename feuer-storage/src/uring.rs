//! Separate single-owner rings for reads and writes. Only this module hands buffer pointers to the kernel.
//! Requests must be nonempty, with offsets and lengths aligned to DIRECT_IO_ALIGNMENT_BYTES.

use std::{
    alloc::{Layout, alloc_zeroed, dealloc},
    collections::{BTreeMap, VecDeque},
    fs::File,
    io,
    ops::Range,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    ptr::NonNull,
    sync::{Arc, LazyLock, Mutex, Weak, mpsc},
    thread::{self, JoinHandle},
};

use bytes::Bytes;
use io_uring::{IoUring, opcode, squeue, types};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};

use crate::{
    IoMetrics, IoOperation,
    allocation::{DiskRegion, DiskRegionReadGuard},
    metrics::IoBufferPoolMetrics,
};

// Buffer addresses, physical offsets, and I/O lengths use this alignment.
// Opening verifies that the filesystem's direct-I/O requirements divide it.
pub(crate) const DIRECT_IO_ALIGNMENT_BYTES: usize = 4096;
// Maximum physical bytes per chunk, including alignment padding. DataFile
// reduces the logical chunk size when its starting offset is unaligned.
pub(crate) const MAX_IO_CHUNK_BYTES: usize = 1024 * 1024;
// Maximum number of admitted read requests.
const MAX_IN_FLIGHT_READS: usize = 64;
// Local-SSD benchmarks saturated 1-MiB writes at QD8. QD64 added no write-only
// throughput, but raised write p99 from 4.6 to 47 ms and worsened small-read latency.
// See benchmarks/ssd/uring-20260928/REPORT.md.
const MAX_IN_FLIGHT_WRITES: usize = 8;
// Per-queue I/O buffer memory budget: 64 full-size aligned buffers.
// Caller inputs, caller-provided read destinations, and completed results are
// outside this budget. A read into an existing buffer charges only its I/O slice.
const MAX_IO_BUFFER_BYTES: usize = 64 * 1024 * 1024;
// Per-queue idle allocations, separate from active I/O and caller-owned results.
// Read once on first use; zero disables retention for that size class.
static IDLE_IO_BUFFER_CAPACITIES: LazyLock<[usize; 3]> = LazyLock::new(|| {
    [
        ("FEUER_SMALL_IO_BUFFER_POOL_BYTES", 128 * 1024 * 1024),
        ("FEUER_MEDIUM_IO_BUFFER_POOL_BYTES", 256 * 1024 * 1024),
        ("FEUER_LARGE_IO_BUFFER_POOL_BYTES", 1024 * 1024 * 1024),
    ]
    .map(|(name, default)| {
        feuer_types::config::read_env_number(name, default, 0).unwrap_or_else(|error| panic!("{error}"))
    })
});

#[cfg(test)]
mod tests;

/// A handle that submits I/O requests and owns the queue thread's lifetime.
pub(crate) struct IoQueueHandle {
    // Every request on this queue has this direction.
    operation: IoOperation,
    // Sends admitted requests; taken on drop to signal shutdown before joining.
    sender: Option<mpsc::SyncSender<IoRequest>>,
    // Shared eventfd wakes the queue for new work or shutdown.
    wake_fd: Arc<OwnedFd>,
    // Queue thread; taken and joined on drop so submitted I/O drains first.
    thread: Option<JoinHandle<()>>,
    // Queue-local budgets, enforced before allocating or queueing work.
    admission: Arc<IoAdmissionBudgets>,
}

struct IoAdmissionBudgets {
    // Limit covering preparing, queued, and active requests.
    request_slots: Arc<Semaphore>,
    // Aligned I/O buffer memory budget: each permit covers 4 KiB.
    // Acquired before allocating the buffer.
    buffer_memory: Arc<Semaphore>,
    idle_buffers: [Arc<Mutex<IdleIoBuffers>>; 3],
}

impl IoAdmissionBudgets {
    fn new(max_in_flight: usize, operation: IoOperation, metrics: &IoMetrics) -> Self {
        Self {
            request_slots: Arc::new(Semaphore::new(max_in_flight)),
            buffer_memory: Arc::new(Semaphore::new(MAX_IO_BUFFER_BYTES / DIRECT_IO_ALIGNMENT_BYTES)),
            idle_buffers: std::array::from_fn(|pool_index| {
                Arc::new(Mutex::new(IdleIoBuffers::new(
                    IDLE_IO_BUFFER_CAPACITIES[pool_index],
                    metrics.buffer_pools(operation)[pool_index].clone(),
                )))
            }),
        }
    }

    fn buffer_pool(&self, length: usize) -> &Arc<Mutex<IdleIoBuffers>> {
        let pool_index = if length <= MAX_IO_CHUNK_BYTES {
            0
        } else if length < 10 * 1024 * 1024 {
            1
        } else {
            2
        };
        &self.idle_buffers[pool_index]
    }
}

impl IoQueueHandle {
    pub(crate) fn new(
        file: Arc<File>,
        directory_lock: Arc<File>,
        operation: IoOperation,
        metrics: &IoMetrics,
    ) -> io::Result<Self> {
        let (thread_name, max_in_flight) = match operation {
            IoOperation::Read => ("feuer-read-io", MAX_IN_FLIGHT_READS),
            IoOperation::Write => ("feuer-write-io", MAX_IN_FLIGHT_WRITES),
            _ => unreachable!("queue only supports reads and writes"),
        };
        let ring = IoUring::new(max_in_flight as u32)?;
        // SAFETY: eventfd has no pointer arguments and returns a new owned fd.
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fd was just created and has no other owner.
        let wake_fd = Arc::new(unsafe { OwnedFd::from_raw_fd(fd) });
        let (sender, receiver) = mpsc::sync_channel(max_in_flight);
        let admission = Arc::new(IoAdmissionBudgets::new(max_in_flight, operation, metrics));
        let mut queue = IoQueue {
            admission: admission.clone(),
            ring,
            file: Some(file),
            directory_lock: Some(directory_lock),
            wake_fd: wake_fd.clone(),
            receiver,
            pending: VecDeque::new(),
            active: (0..max_in_flight).map(|_| None).collect(),
        };
        let thread = thread::Builder::new().name(thread_name.into()).spawn(move || {
            if let Err(error) = queue.process_requests_until_disconnected() {
                tracing::error!(target: "feuer::storage::io", operation = operation.as_str(), %error, "io_uring queue stopped");
            }
        })?;
        Ok(Self {
            operation,
            sender: Some(sender),
            wake_fd,
            thread: Some(thread),
            admission,
        })
    }

    /// Acquires a request slot and buffer-memory permits before preparing or queueing I/O.
    async fn acquire_request_and_buffer_permits(
        &self,
        length: usize,
    ) -> io::Result<(OwnedSemaphorePermit, OwnedSemaphorePermit)> {
        let request_permit = self
            .admission
            .request_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| queue_stopped_error())?;
        let buffer_memory_permit = self
            .admission
            .buffer_memory
            .clone()
            .acquire_many_owned((length / DIRECT_IO_ALIGNMENT_BYTES) as u32)
            .await
            .map_err(|_| queue_stopped_error())?;
        Ok((request_permit, buffer_memory_permit))
    }

    pub(crate) fn allocate_buffer(
        &self,
        length: usize,
        read_guards: Vec<DiskRegionReadGuard>,
    ) -> io::Result<AlignedIoBuffer> {
        AlignedIoBuffer::new(length, read_guards, self.admission.buffer_pool(length))
    }

    pub(crate) async fn read(&self, offset: u64, length: usize) -> io::Result<Bytes> {
        assert_eq!(self.operation, IoOperation::Read);
        let permits = self.acquire_request_and_buffer_permits(length).await?;
        let buffer = self.allocate_buffer(length, Vec::new())?;
        Ok(self
            .submit_and_wait(offset, IoBuffers::Read(buffer), 0..length, permits)
            .await?
            .into_read()
            .into_bytes())
    }

    /// Writes parts at aligned offsets, starting at zero; gaps become zero.
    /// Aligned payload slices are retained directly; only other bytes need a copy.
    pub(crate) async fn write_parts(
        &self,
        offset: u64,
        length: usize,
        parts: &[(usize, Bytes)],
        region: Option<DiskRegion>,
    ) -> io::Result<()> {
        assert_eq!(self.operation, IoOperation::Write);
        assert!(length > 0 && length <= MAX_IO_CHUNK_BYTES);
        assert!(length.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES));
        let permits = self.acquire_request_and_buffer_permits(length).await?;
        let buffers = IoBuffers::from_write_parts(length, parts, region)?;
        self.submit_and_wait(offset, buffers, 0..length, permits).await?;
        Ok(())
    }

    /// Transfers exclusive buffer ownership to the queue until this slice completes.
    pub(crate) async fn read_into(
        &self,
        offset: u64,
        buffer: AlignedIoBuffer,
        destination: Range<usize>,
    ) -> io::Result<AlignedIoBuffer> {
        assert_eq!(self.operation, IoOperation::Read);
        let permits = self.acquire_request_and_buffer_permits(destination.len()).await?;
        Ok(self
            .submit_and_wait(offset, IoBuffers::Read(buffer), destination, permits)
            .await?
            .into_read())
    }

    /// Submits the admitted request to the queue and waits for its buffers or an I/O error.
    async fn submit_and_wait(
        &self,
        offset: u64,
        buffers: IoBuffers,
        destination: Range<usize>,
        permits: (OwnedSemaphorePermit, OwnedSemaphorePermit),
    ) -> io::Result<IoBuffers> {
        let (result_sender, result_receiver) = oneshot::channel();
        let request = IoRequest::new(offset, buffers, destination, result_sender, permits);
        // All queued + active requests hold a permit, so this cannot wait for space.
        self.sender
            .as_ref()
            .unwrap()
            .try_send(request)
            .map_err(|_| queue_stopped_error())?;
        wake_queue(&self.wake_fd);
        result_receiver.await.map_err(|_| queue_stopped_error())?
    }
}

impl Drop for IoQueueHandle {
    fn drop(&mut self) {
        self.sender.take();
        wake_queue(&self.wake_fd);
        // The queue drains submitted I/O before releasing buffers and the directory lock.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn queue_stopped_error() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "io_uring queue stopped")
}

/// Idle aligned buffers indexed by allocation size, with a separate byte budget per pool.
struct IdleIoBuffers {
    by_length: BTreeMap<usize, Vec<AlignedIoBuffer>>,
    bytes: usize,
    capacity: usize,
    metrics: Arc<IoBufferPoolMetrics>,
}

impl IdleIoBuffers {
    fn new(capacity: usize, metrics: Arc<IoBufferPoolMetrics>) -> Self {
        metrics.capacity_bytes.increase(capacity as u64);
        Self {
            by_length: BTreeMap::new(),
            bytes: 0,
            capacity,
            metrics,
        }
    }
}

impl Drop for IdleIoBuffers {
    fn drop(&mut self) {
        self.metrics.idle_bytes.decrease(self.bytes as u64);
        self.metrics.capacity_bytes.decrease(self.capacity as u64);
    }
}

pub(crate) struct AlignedIoBuffer {
    // Owned, aligned allocation whose address stays stable while the kernel uses it.
    ptr: NonNull<u8>,
    // Allocation size/alignment for slice bounds and matching deallocation.
    layout: Layout,
    // Whole-entry reads retain disk ownership even if the waiting caller is canceled.
    read_guards: Vec<DiskRegionReadGuard>,
    // Results do not keep the read pool alive. Idle buffers and write scratch have an empty Weak.
    idle_buffers: Weak<Mutex<IdleIoBuffers>>,
}

impl AlignedIoBuffer {
    fn new(
        length: usize,
        read_guards: Vec<DiskRegionReadGuard>,
        idle_buffers: &Arc<Mutex<IdleIoBuffers>>,
    ) -> io::Result<Self> {
        let reused_buffer = {
            let mut idle = idle_buffers.lock().unwrap();
            let buffer = idle.by_length.get_mut(&length).and_then(Vec::pop);
            if buffer.is_some() {
                idle.bytes -= length;
                idle.metrics.idle_bytes.decrease(length as u64);
                if idle.by_length[&length].is_empty() {
                    idle.by_length.remove(&length);
                }
            }
            buffer
        };
        // Reused memory is initialized but not zeroed; reads overwrite it before exposure.
        let mut buffer = match reused_buffer {
            Some(buffer) => buffer,
            None => Self::allocate_zeroed(length)?,
        };
        buffer.read_guards = read_guards;
        buffer.idle_buffers = Arc::downgrade(idle_buffers);
        Ok(buffer)
    }

    /// Allocates fresh, zeroed aligned memory, freed rather than pooled when its owner is dropped.
    fn allocate_zeroed(length: usize) -> io::Result<Self> {
        assert!(length > 0);
        let layout = Layout::from_size_align(length, DIRECT_IO_ALIGNMENT_BYTES).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "I/O buffer length exceeds allocation limit",
            )
        })?;
        // SAFETY: layout is non-zero with a valid power-of-two alignment.
        let ptr = NonNull::new(unsafe { alloc_zeroed(layout) })
            .ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "direct I/O buffer allocation failed"))?;
        Ok(Self {
            ptr,
            layout,
            read_guards: Vec::new(),
            idle_buffers: Weak::new(),
        })
    }

    pub(crate) fn into_bytes(mut self) -> Bytes {
        // No I/O owns the buffer now; returned bytes must not retain disk regions.
        self.read_guards.clear();
        Bytes::from_owner(self)
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: this allocation is initialized and exclusively accessed by its owner.
        // Called only before submission or after the corresponding CQE has been consumed.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.layout.size()) }
    }
}

impl AsRef<[u8]> for AlignedIoBuffer {
    fn as_ref(&self) -> &[u8] {
        // SAFETY: the allocation is initialized and remains owned by self.
        // Called only after I/O completes, when the kernel no longer accesses it.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.layout.size()) }
    }
}

// SAFETY: AlignedIoBuffer uniquely owns its allocation; moving it does not move the allocation.
unsafe impl Send for AlignedIoBuffer {}

impl Drop for AlignedIoBuffer {
    fn drop(&mut self) {
        // Dropping an abandoned I/O result must release disk ownership even when memory is pooled.
        self.read_guards.clear();
        if let Some(idle_buffers) = self.idle_buffers.upgrade() {
            let mut idle = idle_buffers.lock().unwrap();
            let length = self.layout.size();
            if length <= idle.capacity - idle.bytes {
                // Transfer allocation ownership to the pool. Its empty Weak ensures that
                // destroying the pool frees this allocation instead of returning it again.
                idle.by_length.entry(length).or_default().push(Self {
                    ptr: self.ptr,
                    layout: self.layout,
                    read_guards: Vec::new(),
                    idle_buffers: Weak::new(),
                });
                idle.bytes += length;
                idle.metrics.idle_bytes.increase(length as u64);
                idle.metrics.retained.increase(1);
                return;
            }
            idle.metrics.discarded.increase(1);
        }
        // SAFETY: the matching allocation remains owned, and no kernel operation references it.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) };
    }
}

/// Memory retained by a read or vectored write until completion.
enum IoBuffers {
    Read(AlignedIoBuffer),
    Write {
        bytes: Vec<Bytes>,
        vectors: Vec<libc::iovec>,
        // Retained through completion, even if the caller drops its future.
        _region: Option<DiskRegion>,
    },
}

impl IoBuffers {
    /// Builds buffers from write parts, retaining aligned slices and copying the rest with zero padding.
    /// This prepares memory only; it does not issue a disk write.
    fn from_write_parts(length: usize, parts: &[(usize, Bytes)], region: Option<DiskRegion>) -> io::Result<Self> {
        assert_eq!(parts.first().map(|part| part.0), Some(0));
        let mut buffers = Vec::new();
        for (index, (offset, bytes)) in parts.iter().enumerate() {
            let end = parts.get(index + 1).map_or(length, |part| part.0);
            assert!(*offset <= end && end <= length && end.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES));
            assert!(bytes.len() <= end - offset);
            let aligned_length = if (bytes.as_ptr() as usize).is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES) {
                bytes.len() / DIRECT_IO_ALIGNMENT_BYTES * DIRECT_IO_ALIGNMENT_BYTES
            } else {
                0
            };
            if aligned_length > 0 {
                buffers.push(bytes.slice(..aligned_length));
            }
            let copy_length = end - offset - aligned_length;
            if copy_length > 0 {
                let mut buffer = AlignedIoBuffer::allocate_zeroed(copy_length)?;
                buffer.as_mut_slice()[..bytes.len() - aligned_length].copy_from_slice(&bytes[aligned_length..]);
                buffers.push(buffer.into_bytes());
            }
        }
        Ok(Self::Write {
            vectors: Vec::with_capacity(buffers.len()),
            bytes: buffers,
            _region: region,
        })
    }

    fn submission_entry(&mut self, fd: types::Fd, offset: u64, range: Range<usize>) -> squeue::Entry {
        match self {
            Self::Read(buffer) => {
                // SAFETY: the destination is in bounds and exclusively owned by the
                // active request until its completion has been consumed.
                let ptr = unsafe { buffer.ptr.as_ptr().add(range.start) };
                opcode::Read::new(fd, ptr, range.len() as u32).offset(offset).build()
            }
            Self::Write { bytes, vectors, .. } => {
                vectors.clear();
                let mut bytes_to_skip = range.start;
                for bytes in bytes {
                    if bytes_to_skip >= bytes.len() {
                        bytes_to_skip -= bytes.len();
                        continue;
                    }
                    let remaining_bytes = &bytes[bytes_to_skip..];
                    vectors.push(libc::iovec {
                        iov_base: remaining_bytes.as_ptr().cast_mut().cast(),
                        iov_len: remaining_bytes.len(),
                    });
                    bytes_to_skip = 0;
                }
                // Each slice covers at least 4 KiB: at most 256 descriptors per
                // 1-MiB request, below Linux's IOV_MAX. The request owns them all.
                opcode::Writev::new(fd, vectors.as_ptr(), vectors.len() as u32)
                    .offset(offset)
                    .build()
            }
        }
    }

    fn into_read(self) -> AlignedIoBuffer {
        match self {
            Self::Read(buffer) => buffer,
            Self::Write { .. } => unreachable!("write result cannot be used as a read buffer"),
        }
    }
}

// SAFETY: read buffers own their allocations; write descriptors point only into owned
// immutable Bytes. Moving either does not move payload memory. Only the queue submits them.
unsafe impl Send for IoBuffers {}

/// One admitted I/O request, owning its buffers and permits through completion.
struct IoRequest {
    // Aligned physical start, used for kernel offsets.
    offset: u64,
    // Aligned memory for disk reads/writes; kept alive until I/O completes.
    buffers: IoBuffers,
    // Read destination slice, or the complete logical range of a vectored write.
    destination: Range<usize>,
    // Bytes completed, allowing aligned short-I/O continuations.
    completed_bytes: usize,
    // Caller result channel; taken on finish/failure, also detects cancellation.
    reply: Option<oneshot::Sender<io::Result<IoBuffers>>>,
    // Holds request admission until this request is dropped.
    _request_permit: OwnedSemaphorePermit,
    // Holds the I/O buffer memory charge until this request is dropped.
    _buffer_memory_permit: OwnedSemaphorePermit,
}

impl IoRequest {
    fn new(
        offset: u64,
        buffers: IoBuffers,
        destination: Range<usize>,
        reply: oneshot::Sender<io::Result<IoBuffers>>,
        permits: (OwnedSemaphorePermit, OwnedSemaphorePermit),
    ) -> Self {
        assert!(offset.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES as u64));
        assert!(destination.start.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES));
        let length = destination.len();
        assert!(length.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES));
        match &buffers {
            IoBuffers::Read(buffer) => assert!(destination.end <= buffer.layout.size()),
            IoBuffers::Write { bytes, .. } => assert_eq!(destination, 0..bytes.iter().map(Bytes::len).sum()),
        }
        assert!(length > 0 && length <= MAX_IO_CHUNK_BYTES);
        Self {
            offset,
            buffers,
            destination,
            completed_bytes: 0,
            reply: Some(reply),
            _request_permit: permits.0,
            _buffer_memory_permit: permits.1,
        }
    }

    fn submission_entry(&mut self, fd: i32, slot: usize) -> squeue::Entry {
        self.buffers
            .submission_entry(
                types::Fd(fd),
                self.offset + self.completed_bytes as u64,
                self.destination.start + self.completed_bytes..self.destination.end,
            )
            .user_data(slot as u64)
    }

    /// Applies a kernel completion result to the completed-byte count.
    /// Returns true when the request needs a retry or its aligned remainder submitted.
    fn apply_completion_result(&mut self, result: i32) -> io::Result<bool> {
        if result == -libc::EINTR {
            return Ok(true);
        }
        if result < 0 {
            return Err(io::Error::from_raw_os_error(-result));
        }
        let completion_bytes = result as usize;
        if completion_bytes == 0 || completion_bytes > self.destination.len() - self.completed_bytes {
            return Err(self.incomplete_io_error());
        }
        self.completed_bytes += completion_bytes;
        if self.completed_bytes != self.destination.len() {
            // An unaligned remainder cannot be resubmitted with O_DIRECT.
            if !self.completed_bytes.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES) {
                return Err(self.incomplete_io_error());
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn incomplete_io_error(&self) -> io::Error {
        io::Error::new(
            if matches!(self.buffers, IoBuffers::Read(_)) {
                io::ErrorKind::UnexpectedEof
            } else {
                io::ErrorKind::WriteZero
            },
            "incomplete aligned direct I/O request",
        )
    }

    /// Sends the buffers or error as the request result, releasing its admission permits.
    fn send_result(mut self, result: io::Result<()>) {
        let result = result.map(|()| self.buffers);
        let _ = self.reply.take().unwrap().send(result);
    }
}

struct IoQueue {
    // Closes budgets on exit to release admission waiters.
    admission: Arc<IoAdmissionBudgets>,
    // Thread-owned kernel submission/completion queues; no cross-thread ring access.
    ring: IoUring,
    // Direct-I/O payload file; taken to retain ownership on abnormal exit.
    file: Option<Arc<File>>,
    // Shared directory lock; both queues must drain before it can be released.
    directory_lock: Option<Arc<File>>,
    // Eventfd polled alongside the ring so new work need not wait for completion.
    wake_fd: Arc<OwnedFd>,
    // Incoming admitted requests; disconnection starts draining shutdown.
    receiver: mpsc::Receiver<IoRequest>,
    // Unsubmitted requests in arrival order; callers prevent conflicting I/O.
    pending: VecDeque<IoRequest>,
    // Owns in-flight requests through completion; CQEs identify their slot indices.
    active: Vec<Option<IoRequest>>,
}

impl IoQueue {
    /// Processes requests until the sender disconnects and all queued and active I/O drains.
    fn process_requests_until_disconnected(&mut self) -> io::Result<()> {
        let mut disconnected = false;
        let mut completions = Vec::with_capacity(self.active.len());
        loop {
            completions.extend(self.ring.completion().map(|cqe| (cqe.user_data(), cqe.result())));
            for (slot, result) in completions.drain(..) {
                let slot = slot as usize;
                let request = self.active[slot].as_mut().expect("completion for inactive slot");
                match request.apply_completion_result(result) {
                    Ok(true) => self.queue_active_request(slot),
                    result => self.active[slot].take().unwrap().send_result(result.map(|_| ())),
                }
            }
            loop {
                match self.receiver.try_recv() {
                    Ok(request) => self.pending.push_back(request),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
            self.queue_pending_requests();
            let has_active_requests = self.active.iter().any(Option::is_some);
            if disconnected && !has_active_requests && self.pending.is_empty() {
                return Ok(());
            }
            // submit() never waits for a completion. poll watches BOTH completions and
            // new requests, so an outstanding slow read cannot stall fresh submissions.
            match self.ring.submit() {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
            // A short submission must be retried before sleeping; queued SQEs might
            // otherwise have no completion capable of waking us.
            if !self.ring.submission().is_empty() {
                continue;
            }
            self.wait_for_completion_or_wakeup()?;
        }
    }

    /// Queues pending, uncanceled requests in free active slots and the ring's submission queue.
    /// The caller submits them to the kernel separately.
    fn queue_pending_requests(&mut self) {
        self.pending
            .retain(|request| !request.reply.as_ref().unwrap().is_closed());
        // Callers own conflict prevention and disk-region lifetime through completion.
        // Cancellation discards pending requests, but never releases active requests.
        for slot in 0..self.active.len() {
            if self.active[slot].is_some() {
                continue;
            }
            let Some(request) = self.pending.pop_front() else {
                break;
            };
            self.active[slot] = Some(request);
            self.queue_active_request(slot);
        }
    }

    /// Queues the active request's remaining I/O in the ring without submitting it to the kernel yet.
    fn queue_active_request(&mut self, slot: usize) {
        let entry = self.active[slot]
            .as_mut()
            .unwrap()
            .submission_entry(self.file.as_ref().unwrap().as_raw_fd(), slot);
        // SAFETY: all referenced resources are owned by the active slot through completion.
        // The ring is sized for all active slots, each with at most one submitted or queued SQE.
        unsafe {
            self.ring
                .submission()
                .push(&entry)
                .expect("ring sized for all active slots")
        };
    }

    /// Waits for a ring completion or an eventfd wakeup for new requests or shutdown.
    fn wait_for_completion_or_wakeup(&self) -> io::Result<()> {
        let mut fds = [
            libc::pollfd {
                fd: self.ring.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: self.wake_fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: fds describes two valid pollfd values, and both fds remain owned.
        let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
        if result < 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::Interrupted {
                Ok(())
            } else {
                Err(error)
            };
        }
        if fds
            .iter()
            .any(|fd| fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0)
        {
            return Err(queue_stopped_error());
        }
        if fds[1].revents & libc::POLLIN != 0 {
            let mut wake_count = 0u64;
            // SAFETY: wake_count is writable for the required eight bytes; eventfd is nonblocking.
            unsafe { libc::read(self.wake_fd.as_raw_fd(), (&mut wake_count as *mut u64).cast(), 8) };
        }
        Ok(())
    }
}

impl Drop for IoQueue {
    fn drop(&mut self) {
        self.admission.request_slots.close();
        self.admission.buffer_memory.close();
        if self.active.iter().any(Option::is_some) {
            // An abnormal queue exit cannot prove the kernel has stopped using pointers.
            // Closing a ring may tear it down asynchronously. Leak only the bounded active
            // set and file/lock owners rather than risking use-after-free or early reuse.
            // Normal shutdown drains all completions and never takes this path.
            tracing::error!(target: "feuer::storage::io", "retaining active I/O resources after queue failure");
            for mut request in self.active.iter_mut().filter_map(Option::take) {
                if let Some(reply) = request.reply.take() {
                    let _ = reply.send(Err(queue_stopped_error()));
                }
                std::mem::forget(request);
            }
            std::mem::forget(self.file.take());
            std::mem::forget(self.directory_lock.take());
        }
    }
}

/// Wakes the queue thread through its eventfd after enqueueing work or disconnecting the sender.
fn wake_queue(wake_fd: &OwnedFd) {
    let wake_count = 1u64;
    loop {
        // SAFETY: wake_count is readable for eight bytes and wake_fd is an owned eventfd.
        let result = unsafe { libc::write(wake_fd.as_raw_fd(), (&wake_count as *const u64).cast(), 8) };
        if result >= 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            // EAGAIN means a notification is already pending (eventfd counter full).
            return;
        }
    }
}
