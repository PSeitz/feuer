//! Separate single-owner rings for reads and writes. Only this module hands buffer pointers to the kernel.
//! Requests must be nonempty, with offsets and lengths aligned to DIRECT_IO_ALIGNMENT_BYTES.

use std::{
    alloc::{Layout, alloc_zeroed, dealloc},
    collections::VecDeque,
    fs::File,
    io,
    ops::Range,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    ptr::NonNull,
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
};

use bytes::Bytes;
use io_uring::{IoUring, opcode, squeue, types};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};

use crate::{IoOperation, allocation::DiskRegionReadGuard};

// Buffer addresses, physical offsets, and I/O lengths use this alignment.
// Opening verifies that the filesystem's direct-I/O requirements divide it.
pub(crate) const DIRECT_IO_ALIGNMENT_BYTES: usize = 4096;
// Maximum physical bytes per chunk, including alignment padding. DataFile
// reduces the logical chunk size when its starting offset is unaligned.
pub(crate) const MAX_IO_CHUNK_BYTES: usize = 1024 * 1024;
// Maximum ring capacity; reads admit 64 requests, writes admit fewer below.
const MAX_IN_FLIGHT_IO: usize = 64;
// Local-SSD benchmarks saturated 1-MiB writes at QD8. QD64 added no write-only
// throughput, but raised write p99 from 4.6 to 47 ms and worsened small-read latency.
// See benchmarks/ssd/uring-20260928/REPORT.md.
const MAX_IN_FLIGHT_WRITES: usize = 8;
// Per-queue I/O buffer memory budget: MAX_IN_FLIGHT_IO full-size aligned buffers.
// Caller inputs, caller-provided read destinations, and completed results are
// outside this budget. A read into an existing buffer charges only its I/O slice.
const MAX_IO_BUFFER_BYTES: usize = 64 * 1024 * 1024;

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
}

impl IoAdmissionBudgets {
    fn new(max_in_flight: usize) -> Self {
        Self {
            request_slots: Arc::new(Semaphore::new(max_in_flight)),
            buffer_memory: Arc::new(Semaphore::new(MAX_IO_BUFFER_BYTES / DIRECT_IO_ALIGNMENT_BYTES)),
        }
    }
}

impl IoQueueHandle {
    pub(crate) fn new(file: Arc<File>, directory_lock: Arc<File>, operation: IoOperation) -> io::Result<Self> {
        let (thread_name, max_in_flight) = match operation {
            IoOperation::Read => ("feuer-read-io", MAX_IN_FLIGHT_IO),
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
        let admission = Arc::new(IoAdmissionBudgets::new(max_in_flight));
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
            if let Err(error) = queue.run() {
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

    async fn acquire(&self, length: usize) -> io::Result<(OwnedSemaphorePermit, OwnedSemaphorePermit)> {
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

    pub(crate) async fn execute(&self, offset: u64, length: usize, payload: &[u8]) -> io::Result<Bytes> {
        let permits = self.acquire(length).await?;
        let mut buffer = AlignedIoBuffer::new(length, Vec::new())?;
        if self.operation == IoOperation::Write {
            buffer.as_mut_slice().copy_from_slice(payload);
        }
        let buffer = self.execute_buffer(offset, buffer, 0..length, permits).await?;
        Ok(if self.operation == IoOperation::Read {
            buffer.into_bytes()
        } else {
            Bytes::new()
        })
    }

    /// Transfers exclusive buffer ownership to the queue until this slice completes.
    pub(crate) async fn read_into(
        &self,
        offset: u64,
        buffer: AlignedIoBuffer,
        destination: Range<usize>,
    ) -> io::Result<AlignedIoBuffer> {
        assert_eq!(self.operation, IoOperation::Read);
        let permits = self.acquire(destination.len()).await?;
        self.execute_buffer(offset, buffer, destination, permits).await
    }

    async fn execute_buffer(
        &self,
        offset: u64,
        buffer: AlignedIoBuffer,
        destination: Range<usize>,
        permits: (OwnedSemaphorePermit, OwnedSemaphorePermit),
    ) -> io::Result<AlignedIoBuffer> {
        let (reply, receive) = oneshot::channel();
        let request = IoRequest::new(self.operation, offset, buffer, destination, reply, permits);
        // All queued + active requests hold a permit, so this cannot wait for space.
        self.sender
            .as_ref()
            .unwrap()
            .try_send(request)
            .map_err(|_| queue_stopped_error())?;
        notify(&self.wake_fd);
        receive.await.map_err(|_| queue_stopped_error())?
    }
}

impl Drop for IoQueueHandle {
    fn drop(&mut self) {
        self.sender.take();
        notify(&self.wake_fd);
        // The queue drains submitted I/O before releasing buffers and the directory lock.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn queue_stopped_error() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "io_uring queue stopped")
}

pub(crate) struct AlignedIoBuffer {
    // Owned, aligned allocation whose address stays stable while the kernel uses it.
    ptr: NonNull<u8>,
    // Allocation size/alignment for slice bounds and matching deallocation.
    layout: Layout,
    // Whole-entry reads retain disk ownership even if the waiting caller is canceled.
    read_guards: Vec<DiskRegionReadGuard>,
}

impl AlignedIoBuffer {
    pub(crate) fn new(length: usize, read_guards: Vec<DiskRegionReadGuard>) -> io::Result<Self> {
        assert!(length > 0);
        let layout = Layout::from_size_align(length, DIRECT_IO_ALIGNMENT_BYTES).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "read buffer length exceeds allocation limit",
            )
        })?;
        // SAFETY: layout is non-zero with a valid power-of-two alignment.
        let ptr = NonNull::new(unsafe { alloc_zeroed(layout) })
            .ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "direct I/O buffer allocation failed"))?;
        Ok(Self {
            ptr,
            layout,
            read_guards,
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
        // SAFETY: the matching allocation remains owned, and no kernel operation references it.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) };
    }
}

/// One admitted I/O request, owning its buffers and permits through completion.
struct IoRequest {
    // I/O direction, unchanged through short-I/O continuations.
    operation: IoOperation,
    // Aligned physical start, used for kernel offsets.
    offset: u64,
    // Aligned memory for disk reads/writes; kept alive until I/O completes.
    io_buffer: AlignedIoBuffer,
    // Disjoint destination slice within the exclusively owned allocation.
    destination: Range<usize>,
    // Bytes completed, allowing aligned short-I/O continuations.
    completed_bytes: usize,
    // Caller result channel; taken on finish/failure, also detects cancellation.
    reply: Option<oneshot::Sender<io::Result<AlignedIoBuffer>>>,
    // Holds request admission until this request is dropped.
    _request_permit: OwnedSemaphorePermit,
    // Holds the I/O buffer memory charge until this request is dropped.
    _buffer_memory_permit: OwnedSemaphorePermit,
}

impl IoRequest {
    fn new(
        operation: IoOperation,
        offset: u64,
        io_buffer: AlignedIoBuffer,
        destination: Range<usize>,
        reply: oneshot::Sender<io::Result<AlignedIoBuffer>>,
        permits: (OwnedSemaphorePermit, OwnedSemaphorePermit),
    ) -> Self {
        assert!(offset.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES as u64));
        assert!(destination.start.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES));
        let length = destination.len();
        assert!(length.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES));
        assert!(!destination.is_empty() && destination.end <= io_buffer.layout.size());
        assert!(destination.len() <= MAX_IO_CHUNK_BYTES);
        Self {
            operation,
            offset,
            io_buffer,
            destination,
            completed_bytes: 0,
            reply: Some(reply),
            _request_permit: permits.0,
            _buffer_memory_permit: permits.1,
        }
    }

    fn submission_entry(&mut self, fd: i32, slot: usize) -> squeue::Entry {
        let fd = types::Fd(fd);
        // SAFETY: completed_bytes is within io_buffer. The request owns this memory
        // in its active slot until the completion is consumed.
        let ptr = unsafe {
            self.io_buffer
                .ptr
                .as_ptr()
                .add(self.destination.start + self.completed_bytes)
        };
        let length = (self.destination.len() - self.completed_bytes) as u32;
        let offset = self.offset + self.completed_bytes as u64;
        let entry = if self.operation == IoOperation::Read {
            opcode::Read::new(fd, ptr, length).offset(offset).build()
        } else {
            opcode::Write::new(fd, ptr, length).offset(offset).build()
        };
        entry.user_data(slot as u64)
    }

    // true means the operation needs to be retried or its remainder submitted.
    fn complete(&mut self, result: i32) -> io::Result<bool> {
        if result == -libc::EINTR {
            return Ok(true);
        }
        if result < 0 {
            return Err(io::Error::from_raw_os_error(-result));
        }
        let count = result as usize;
        if count == 0 || count > self.destination.len() - self.completed_bytes {
            return Err(self.incomplete_io_error());
        }
        self.completed_bytes += count;
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
            if self.operation == IoOperation::Read {
                io::ErrorKind::UnexpectedEof
            } else {
                io::ErrorKind::WriteZero
            },
            "incomplete aligned direct I/O request",
        )
    }

    fn finish(mut self, result: io::Result<()>) {
        let result = result.map(|()| self.io_buffer);
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
    fn run(&mut self) -> io::Result<()> {
        let mut disconnected = false;
        let mut completions = Vec::with_capacity(MAX_IN_FLIGHT_IO);
        loop {
            completions.extend(self.ring.completion().map(|cqe| (cqe.user_data(), cqe.result())));
            for (slot, result) in completions.drain(..) {
                let slot = slot as usize;
                let request = self.active[slot].as_mut().expect("completion for inactive slot");
                match request.complete(result) {
                    Ok(true) => self.submit_slot(slot),
                    result => self.active[slot].take().unwrap().finish(result.map(|_| ())),
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
            self.schedule();
            let active = self.active.iter().any(Option::is_some);
            if disconnected && !active && self.pending.is_empty() {
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
            self.wait()?;
        }
    }

    fn schedule(&mut self) {
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
            self.submit_slot(slot);
        }
    }

    fn submit_slot(&mut self, slot: usize) {
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

    fn wait(&self) -> io::Result<()> {
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
            let mut value = 0u64;
            // SAFETY: value is writable for the required eight bytes; eventfd is nonblocking.
            unsafe { libc::read(self.wake_fd.as_raw_fd(), (&mut value as *mut u64).cast(), 8) };
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

fn notify(wake_fd: &OwnedFd) {
    let value = 1u64;
    loop {
        // SAFETY: value is readable for eight bytes and wake is an owned eventfd.
        let result = unsafe { libc::write(wake_fd.as_raw_fd(), (&value as *const u64).cast(), 8) };
        if result >= 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            // EAGAIN means a notification is already pending (eventfd counter full).
            return;
        }
    }
}
