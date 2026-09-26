//! A single-owner ring. Only this module hands buffer pointers to the kernel.
//! Requests must be nonempty, with offsets and lengths aligned to DIRECT_IO_ALIGNMENT_BYTES.

use std::{
    alloc::{Layout, alloc_zeroed, dealloc},
    collections::VecDeque,
    fs::File,
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    ptr::NonNull,
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
};

use bytes::Bytes;
use io_uring::{IoUring, opcode, squeue, types};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};

use crate::IoOperation;

// Buffer addresses, physical offsets, and I/O lengths use this alignment.
// Opening verifies that the filesystem's direct-I/O requirements divide it.
pub(crate) const DIRECT_IO_ALIGNMENT_BYTES: usize = 4096;
// Maximum physical bytes per chunk, including alignment padding. DataFile
// reduces the logical chunk size when its starting offset is unaligned.
pub(crate) const MAX_IO_CHUNK_BYTES: usize = 1024 * 1024;
// Maximum outstanding ring operations, shared by reads and writes.
const MAX_IN_FLIGHT_IO: usize = 64;
// Separate admission pools: MAX_IN_FLIGHT_IO reads and MAX_IN_FLIGHT_IO writes.
// Writes cannot consume read permits; each class can independently fill the ring.
const MAX_ADMITTED_REQUESTS: usize = 2 * MAX_IN_FLIGHT_IO;
// Staging budget split equally between reads and writes. Each half can
// hold MAX_IN_FLIGHT_IO full-size aligned buffers. Caller inputs and read-result
// allocations are outside this budget.
const MAX_STAGING_BUFFER_BYTES: usize = 128 * 1024 * 1024;

#[cfg(test)]
mod tests;

/// A handle that submits I/O requests and owns the queue thread's lifetime.
pub(crate) struct IoQueueHandle {
    // Sends admitted requests; taken on drop to signal shutdown before joining.
    sender: Option<mpsc::SyncSender<IoRequest>>,
    // Shared eventfd wakes the queue for new work or shutdown.
    wake_fd: Arc<OwnedFd>,
    // Queue thread; taken and joined on drop so submitted I/O drains first.
    thread: Option<JoinHandle<()>>,
    // Shared budgets, enforced before allocating or queueing work.
    admission: Arc<IoAdmissionBudgets>,
}

// Index 0 is reads; index 1 is writes. Separate pools keep writes from
// consuming read admission or staging capacity.
struct IoAdmissionBudgets {
    // Per-class limits covering preparing, queued, and active requests.
    request_slots: [Arc<Semaphore>; 2],
    // Per-class staging budgets in alignment-sized units; acquired before allocation.
    staging_pages: [Arc<Semaphore>; 2],
}

impl IoAdmissionBudgets {
    fn new() -> Self {
        Self {
            request_slots: std::array::from_fn(|_| Arc::new(Semaphore::new(MAX_IN_FLIGHT_IO))),
            staging_pages: std::array::from_fn(|_| {
                Arc::new(Semaphore::new(MAX_STAGING_BUFFER_BYTES / 2 / DIRECT_IO_ALIGNMENT_BYTES))
            }),
        }
    }
}

impl IoQueueHandle {
    pub(crate) fn new(file: File, directory_lock: File) -> io::Result<Self> {
        let ring = IoUring::new(MAX_IN_FLIGHT_IO as u32)?;
        // SAFETY: eventfd has no pointer arguments and returns a new owned fd.
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fd was just created and has no other owner.
        let wake_fd = Arc::new(unsafe { OwnedFd::from_raw_fd(fd) });
        let (sender, receiver) = mpsc::sync_channel(MAX_ADMITTED_REQUESTS);
        let admission = Arc::new(IoAdmissionBudgets::new());
        let mut queue = IoQueue {
            admission: admission.clone(),
            ring,
            file: Some(file),
            directory_lock: Some(directory_lock),
            wake_fd: wake_fd.clone(),
            receiver,
            pending: VecDeque::new(),
            active: (0..MAX_IN_FLIGHT_IO).map(|_| None).collect(),
        };
        let thread = thread::Builder::new().name("feuer-io".into()).spawn(move || {
            if let Err(error) = queue.run() {
                tracing::error!(target: "feuer::storage::io", %error, "io_uring queue stopped");
            }
        })?;
        Ok(Self {
            sender: Some(sender),
            wake_fd,
            thread: Some(thread),
            admission,
        })
    }

    pub(crate) async fn execute(
        &self,
        operation: IoOperation,
        offset: u64,
        length: usize,
        payload: &[u8],
    ) -> io::Result<Bytes> {
        let class = usize::from(operation != IoOperation::Read);
        let request_permit = self.admission.request_slots[class]
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| queue_stopped_error())?;
        let staging_pages_permit = self.admission.staging_pages[class]
            .clone()
            .acquire_many_owned((length / DIRECT_IO_ALIGNMENT_BYTES) as u32)
            .await
            .map_err(|_| queue_stopped_error())?;
        let (reply, receive) = oneshot::channel();
        let request = IoRequest::new(
            operation,
            offset,
            length,
            payload,
            reply,
            (request_permit, staging_pages_permit),
        )?;
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

struct AlignedIoBuffer {
    // Owned, aligned allocation whose address stays stable while the kernel uses it.
    ptr: NonNull<u8>,
    // Allocation size/alignment for slice bounds and matching deallocation.
    layout: Layout,
}

impl AlignedIoBuffer {
    fn new(length: usize) -> io::Result<Self> {
        let layout = Layout::from_size_align(length, DIRECT_IO_ALIGNMENT_BYTES).unwrap();
        // SAFETY: layout is non-zero with a valid power-of-two alignment.
        let ptr = NonNull::new(unsafe { alloc_zeroed(layout) })
            .ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "direct I/O buffer allocation failed"))?;
        Ok(Self { ptr, layout })
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: this allocation is initialized and exclusively accessed by its owner.
        // Called only before submission or after the corresponding CQE has been consumed.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.layout.size()) }
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
    // Bytes completed, allowing aligned short-I/O continuations.
    completed_bytes: usize,
    // Caller result channel; taken on finish/failure, also detects cancellation.
    reply: Option<oneshot::Sender<io::Result<Bytes>>>,
    // Holds request admission until this request is dropped.
    _request_permit: OwnedSemaphorePermit,
    // Holds the staging-buffer charge until this request is dropped.
    _staging_pages_permit: OwnedSemaphorePermit,
}

impl IoRequest {
    fn new(
        operation: IoOperation,
        offset: u64,
        length: usize,
        payload: &[u8],
        reply: oneshot::Sender<io::Result<Bytes>>,
        permits: (OwnedSemaphorePermit, OwnedSemaphorePermit),
    ) -> io::Result<Self> {
        assert!(offset.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES as u64));
        assert!(length.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES));
        assert!(length > 0);
        let mut io_buffer = AlignedIoBuffer::new(length)?;
        if operation == IoOperation::Write {
            io_buffer.as_mut_slice().copy_from_slice(payload);
        }
        Ok(Self {
            operation,
            offset,
            io_buffer,
            completed_bytes: 0,
            reply: Some(reply),
            _request_permit: permits.0,
            _staging_pages_permit: permits.1,
        })
    }

    fn submission_entry(&mut self, fd: i32, slot: usize) -> squeue::Entry {
        let fd = types::Fd(fd);
        // SAFETY: completed_bytes is within io_buffer. The request owns this memory
        // in its active slot until the completion is consumed.
        let ptr = unsafe { self.io_buffer.ptr.as_ptr().add(self.completed_bytes) };
        let length = (self.io_buffer.layout.size() - self.completed_bytes) as u32;
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
        if count == 0 || count > self.io_buffer.layout.size() - self.completed_bytes {
            return Err(self.incomplete_io_error());
        }
        self.completed_bytes += count;
        if self.completed_bytes != self.io_buffer.layout.size() {
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
        let result = result.map(|()| {
            if self.operation == IoOperation::Read {
                Bytes::copy_from_slice(self.io_buffer.as_mut_slice())
            } else {
                Bytes::new()
            }
        });
        let _ = self.reply.take().unwrap().send(result);
    }
}

struct IoQueue {
    // Closes budgets on exit to release admission waiters.
    admission: Arc<IoAdmissionBudgets>,
    // Thread-owned kernel submission/completion queues; no cross-thread ring access.
    ring: IoUring,
    // Direct-I/O payload file; taken to retain ownership on abnormal exit.
    file: Option<File>,
    // Exclusive directory lock, retained if kernel I/O may still be active.
    directory_lock: Option<File>,
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
        loop {
            let completions: Vec<_> = self
                .ring
                .completion()
                .map(|cqe| (cqe.user_data(), cqe.result()))
                .collect();
            for (slot, result) in completions {
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
        // SSD benchmarks (benchmarks/ssd/ssd-concurrent-read-write.md) suggest a
        // future write throttle for small-read-heavy workloads. Mixed-size reads
        // tolerate moderate concurrent writes, so leave writes unthrottled for now.
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
        // At most MAX_IN_FLIGHT_IO slots exist, each with at most one submitted or queued SQE.
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
        for semaphore in self.admission.request_slots.iter().chain(&self.admission.staging_pages) {
            semaphore.close();
        }
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
