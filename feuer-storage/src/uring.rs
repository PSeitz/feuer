//! A single-owner ring. Only this module hands buffer pointers to the kernel.

use std::{
    alloc::{Layout, alloc_zeroed, dealloc},
    collections::VecDeque,
    fs::File,
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    ptr::NonNull,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};

use bytes::Bytes;
use io_uring::{IoUring, opcode, squeue, types};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};

use crate::IoOperation;

// Buffer addresses, physical offsets, and I/O lengths use this alignment.
// Opening verifies that the filesystem's direct-I/O requirements divide it.
pub(crate) const ALIGN: usize = 4096;
// Maximum physical bytes per chunk, including alignment padding. DataFile
// reduces the logical chunk size when its starting offset is unaligned.
pub(crate) const MAX_IO_CHUNK_BYTES: usize = 1024 * 1024;
// Maximum outstanding ring operations, shared by reads and writes.
const MAX_IN_FLIGHT_IO: usize = 64;
// Under read demand, allow only this many outstanding writes (including RMW).
// With no reads, writes may use all MAX_IN_FLIGHT_IO slots. Initial policy, not an optimum.
const WRITES_WITH_READS: usize = 4;
// Separate admission pools: MAX_IN_FLIGHT_IO reads and MAX_IN_FLIGHT_IO writes.
// Writes cannot consume read permits; each class can independently fill the ring.
const REQUESTS: usize = 2 * MAX_IN_FLIGHT_IO;
// Staging budget split equally between reads and writes. Each half can
// hold MAX_IN_FLIGHT_IO full-size aligned buffers. RMW also charges its staging payload;
// caller inputs and read-result allocations are outside this budget.
const BUFFER_BYTES: usize = 128 * 1024 * 1024;

#[cfg(test)]
mod tests;

pub(crate) struct Handle {
    // Sends admitted requests; taken on drop to signal shutdown before joining.
    sender: Option<mpsc::SyncSender<Request>>,
    // Shared eventfd wakes the driver for new work, demand changes, or shutdown.
    wake: Arc<OwnedFd>,
    // Worker ownership; taken and joined on drop so submitted I/O drains first.
    thread: Option<JoinHandle<()>>,
    // Shared budgets and read demand, enforced before allocating or queueing work.
    admission: Arc<Admission>,
}

// Index 0 is demand reads; index 1 is writes. Separate pools avoid
// priority inversion in semaphore wait queues before requests reach the driver.
struct Admission {
    // Per-class limits covering preparing, queued, and active requests.
    requests: [Arc<Semaphore>; 2],
    // Per-class staging budgets in ALIGN-sized units; acquired before allocation.
    buffers: [Arc<Semaphore>; 2],
    // Live read futures, including admission waiters, so writes yield early.
    reads: AtomicUsize,
}

impl Admission {
    fn new() -> Self {
        Self {
            requests: std::array::from_fn(|_| Arc::new(Semaphore::new(MAX_IN_FLIGHT_IO))),
            buffers: std::array::from_fn(|_| Arc::new(Semaphore::new(BUFFER_BYTES / 2 / ALIGN))),
            reads: AtomicUsize::new(0),
        }
    }
}

// Count reads BEFORE admission, not just after buffer allocation. Cancellation
// removes their demand, waking an otherwise idle driver when full-speed writes
// can resume. Submitted reads remain visible through the driver's active slots.
struct ReadDemandGuard {
    // Owns one read-demand count, removed on completion or cancellation.
    admission: Arc<Admission>,
    // Wakes the driver when demand starts or ends to update the write allowance.
    wake: Arc<OwnedFd>,
}

impl ReadDemandGuard {
    fn new(admission: &Arc<Admission>, wake: &Arc<OwnedFd>) -> Self {
        if admission.reads.fetch_add(1, Ordering::AcqRel) == 0 {
            notify(wake);
        }
        Self {
            admission: admission.clone(),
            wake: wake.clone(),
        }
    }
}

impl Drop for ReadDemandGuard {
    fn drop(&mut self) {
        if self.admission.reads.fetch_sub(1, Ordering::AcqRel) == 1 {
            notify(&self.wake);
        }
    }
}

fn buffer_charge(operation: IoOperation, offset: u64, length: usize) -> u32 {
    let data_offset_in_buffer = offset as usize % ALIGN;
    let aligned_length = (data_offset_in_buffer + length).next_multiple_of(ALIGN);
    let rmw = operation == IoOperation::Write && (data_offset_in_buffer != 0 || length != aligned_length);
    (aligned_length.max(ALIGN) / ALIGN * if rmw { 2 } else { 1 }) as u32
}

impl Handle {
    pub(crate) fn new(file: File, lock: File) -> io::Result<Self> {
        let ring = IoUring::new(MAX_IN_FLIGHT_IO as u32)?;
        // SAFETY: eventfd has no pointer arguments and returns a new owned fd.
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fd was just created and has no other owner.
        let wake = Arc::new(unsafe { OwnedFd::from_raw_fd(fd) });
        let (sender, receiver) = mpsc::sync_channel(REQUESTS);
        let admission = Arc::new(Admission::new());
        let mut driver = Driver {
            admission: admission.clone(),
            ring,
            file: Some(file),
            lock: Some(lock),
            wake: wake.clone(),
            receiver,
            pending: VecDeque::new(),
            active: (0..MAX_IN_FLIGHT_IO).map(|_| None).collect(),
        };
        let thread = thread::Builder::new().name("feuer-io".into()).spawn(move || {
            if let Err(error) = driver.run() {
                tracing::error!(target: "feuer::storage::io", %error, "io_uring driver stopped");
            }
        })?;
        Ok(Self {
            sender: Some(sender),
            wake,
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
        let _read = (operation == IoOperation::Read).then(|| ReadDemandGuard::new(&self.admission, &self.wake));
        let class = usize::from(operation != IoOperation::Read);
        let slot = self.admission.requests[class]
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| stopped())?;
        let bytes = self.admission.buffers[class]
            .clone()
            .acquire_many_owned(buffer_charge(operation, offset, length))
            .await
            .map_err(|_| stopped())?;
        let (reply, receive) = oneshot::channel();
        let request = Request::new(operation, offset, length, payload, reply, (slot, bytes))?;
        // All queued + active requests hold a permit, so this cannot wait for space.
        self.sender.as_ref().unwrap().try_send(request).map_err(|_| stopped())?;
        notify(&self.wake);
        receive.await.map_err(|_| stopped())?
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.sender.take();
        notify(&self.wake);
        // The worker drains submitted I/O before releasing buffers and the directory lock.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn stopped() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "io_uring driver stopped")
}

struct AlignedBuffer {
    // Owned, aligned allocation whose address stays stable while the kernel uses it.
    ptr: NonNull<u8>,
    // Allocation size/alignment for slice bounds and matching deallocation.
    layout: Layout,
}

impl AlignedBuffer {
    fn new(length: usize) -> io::Result<Self> {
        let layout = Layout::from_size_align(length.max(ALIGN), ALIGN).unwrap();
        // SAFETY: layout is non-zero with a valid power-of-two alignment.
        let ptr = NonNull::new(unsafe { alloc_zeroed(layout) })
            .ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "direct I/O buffer allocation failed"))?;
        Ok(Self { ptr, layout })
    }

    fn bytes(&mut self) -> &mut [u8] {
        // SAFETY: this allocation is initialized and exclusively accessed by its owner.
        // Called only before submission or after the corresponding CQE has been consumed.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.layout.size()) }
    }
}

// SAFETY: AlignedBuffer uniquely owns its allocation; moving it does not move the allocation.
unsafe impl Send for AlignedBuffer {}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        // SAFETY: the matching allocation remains owned, and no kernel operation references it.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) };
    }
}

struct Request {
    // Caller operation; an RMW stays classified as a write during its read phase.
    operation: IoOperation,
    // Aligned physical start, used for kernel offsets and overlap checks.
    offset: u64,
    // Requested data starts at this byte offset in `io_buffer`.
    data_offset_in_buffer: usize,
    // Logical byte count, excluding alignment padding from results and RMW updates.
    length: usize,
    // Physical byte count including padding, for I/O and overlap serialization.
    aligned_length: usize,
    // Aligned memory for disk reads/writes; kept alive until I/O completes.
    io_buffer: AlignedBuffer,
    // RMW input preserved during the initial read, then merged into `io_buffer`.
    payload: Option<Bytes>,
    // Current I/O direction; flips from read to write after an RMW read completes.
    reading: bool,
    // Bytes completed in this phase, allowing aligned short-I/O continuations.
    completed: usize,
    // Caller result channel; taken on finish/failure, also detects cancellation.
    reply: Option<oneshot::Sender<io::Result<Bytes>>>,
    // Holds request admission until this request is dropped.
    _slot: OwnedSemaphorePermit,
    // Holds the staging-buffer charge until this request is dropped.
    _bytes: OwnedSemaphorePermit,
}

impl Request {
    fn new(
        operation: IoOperation,
        offset: u64,
        length: usize,
        payload: &[u8],
        reply: oneshot::Sender<io::Result<Bytes>>,
        permits: (OwnedSemaphorePermit, OwnedSemaphorePermit),
    ) -> io::Result<Self> {
        let data_offset_in_buffer = offset as usize % ALIGN;
        let aligned_length = (data_offset_in_buffer + length).next_multiple_of(ALIGN);
        let mut io_buffer = AlignedBuffer::new(aligned_length)?;
        let rmw = operation == IoOperation::Write && (data_offset_in_buffer != 0 || length != aligned_length);
        let payload = if rmw {
            Some(Bytes::copy_from_slice(payload))
        } else {
            if operation == IoOperation::Write {
                io_buffer.bytes()[..length].copy_from_slice(payload);
            }
            None
        };
        Ok(Self {
            operation,
            offset: offset - data_offset_in_buffer as u64,
            data_offset_in_buffer,
            length,
            aligned_length,
            io_buffer,
            payload,
            reading: operation == IoOperation::Read || rmw,
            completed: 0,
            reply: Some(reply),
            _slot: permits.0,
            _bytes: permits.1,
        })
    }

    fn conflicts(&self, other: &Self) -> bool {
        (self.operation == IoOperation::Write || other.operation == IoOperation::Write)
            && self.offset < other.offset + other.aligned_length as u64
            && other.offset < self.offset + self.aligned_length as u64
    }

    fn entry(&mut self, fd: i32, slot: usize) -> squeue::Entry {
        let fd = types::Fd(fd);
        // SAFETY: completed is within io_buffer. The request owns this memory
        // in its active slot until the completion is consumed.
        let ptr = unsafe { self.io_buffer.ptr.as_ptr().add(self.completed) };
        let length = (self.aligned_length - self.completed) as u32;
        let offset = self.offset + self.completed as u64;
        let entry = if self.reading {
            opcode::Read::new(fd, ptr, length).offset(offset).build()
        } else {
            opcode::Write::new(fd, ptr, length).offset(offset).build()
        };
        entry.user_data(slot as u64)
    }

    // true means a further operation (short-I/O remainder or RMW write) is needed.
    fn complete(&mut self, result: i32) -> io::Result<bool> {
        if result == -libc::EINTR {
            return Ok(true);
        }
        if result < 0 {
            return Err(io::Error::from_raw_os_error(-result));
        }
        let count = result as usize;
        if count == 0 || count > self.aligned_length - self.completed {
            return Err(self.short_error());
        }
        self.completed += count;
        if self.completed != self.aligned_length {
            // An unaligned remainder cannot be resubmitted with O_DIRECT.
            if !self.completed.is_multiple_of(ALIGN) {
                return Err(self.short_error());
            }
            return Ok(true);
        }
        if let Some(payload) = self.payload.take() {
            self.io_buffer.bytes()[self.data_offset_in_buffer..self.data_offset_in_buffer + self.length]
                .copy_from_slice(&payload);
            self.reading = false;
            self.completed = 0;
            return Ok(true);
        }
        Ok(false)
    }

    fn short_error(&self) -> io::Error {
        io::Error::new(
            if self.reading {
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
                Bytes::copy_from_slice(
                    &self.io_buffer.bytes()[self.data_offset_in_buffer..self.data_offset_in_buffer + self.length],
                )
            } else {
                Bytes::new()
            }
        });
        let _ = self.reply.take().unwrap().send(result);
    }
}

struct Driver {
    // Reads caller demand for scheduling; closes budgets on exit to release waiters.
    admission: Arc<Admission>,
    // Thread-owned kernel submission/completion queues; no cross-thread ring access.
    ring: IoUring,
    // Direct-I/O payload file; taken to retain ownership on abnormal exit.
    file: Option<File>,
    // Exclusive directory lock, retained if kernel I/O may still be active.
    lock: Option<File>,
    // Eventfd polled alongside the ring so new work need not wait for completion.
    wake: Arc<OwnedFd>,
    // Incoming admitted requests; disconnection starts draining shutdown.
    receiver: mpsc::Receiver<Request>,
    // Unsubmitted requests in arrival order, preserving overlap ordering.
    pending: VecDeque<Request>,
    // Owns in-flight requests through completion; CQEs identify their slot indices.
    active: Vec<Option<Request>>,
}

impl Driver {
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
        let has_pending_or_active_reads = self.pending.iter().any(|r| r.operation == IoOperation::Read)
            || self.active.iter().flatten().any(|r| r.operation == IoOperation::Read);
        let mut writes = self
            .active
            .iter()
            .flatten()
            .filter(|r| r.operation != IoOperation::Read)
            .count();
        // First admit the small write allowance, so continuous reads cannot starve
        // writes. Reads then get every remaining slot; with no writes they get all
        // MAX_IN_FLIGHT_IO. Never preempt active writes or reorder conflicting I/O.
        for reading in [false, true] {
            let mut index = 0;
            while index < self.pending.len() {
                let Some(slot) = self.active.iter().position(Option::is_none) else {
                    return;
                };
                // Recheck pre-admission demand while filling a write-only batch.
                let write_limit = if has_pending_or_active_reads || self.admission.reads.load(Ordering::Acquire) != 0 {
                    WRITES_WITH_READS
                } else {
                    MAX_IN_FLIGHT_IO
                };
                if !reading && writes >= write_limit {
                    break;
                }
                let request = &self.pending[index];
                let blocked = (request.operation == IoOperation::Read) != reading
                    || self.active.iter().flatten().any(|other| request.conflicts(other))
                    || self.pending.iter().take(index).any(|other| request.conflicts(other));
                if blocked {
                    index += 1;
                    continue;
                }
                self.active[slot] = self.pending.remove(index);
                self.submit_slot(slot);
                writes += usize::from(!reading);
            }
        }
    }

    fn submit_slot(&mut self, slot: usize) {
        let entry = self.active[slot]
            .as_mut()
            .unwrap()
            .entry(self.file.as_ref().unwrap().as_raw_fd(), slot);
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
                fd: self.wake.as_raw_fd(),
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
            return Err(stopped());
        }
        if fds[1].revents & libc::POLLIN != 0 {
            let mut value = 0u64;
            // SAFETY: value is writable for the required eight bytes; eventfd is nonblocking.
            unsafe { libc::read(self.wake.as_raw_fd(), (&mut value as *mut u64).cast(), 8) };
        }
        Ok(())
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        for semaphore in self.admission.requests.iter().chain(&self.admission.buffers) {
            semaphore.close();
        }
        if self.active.iter().any(Option::is_some) {
            // An abnormal driver exit cannot prove the kernel has stopped using pointers.
            // Closing a ring may tear it down asynchronously. Leak only the bounded active
            // set and file/lock owners rather than risking use-after-free or early reuse.
            // Normal shutdown drains all completions and never takes this path.
            tracing::error!(target: "feuer::storage::io", "retaining active I/O resources after driver failure");
            for mut request in self.active.iter_mut().filter_map(Option::take) {
                if let Some(reply) = request.reply.take() {
                    let _ = reply.send(Err(stopped()));
                }
                std::mem::forget(request);
            }
            std::mem::forget(self.file.take());
            std::mem::forget(self.lock.take());
        }
    }
}

fn notify(wake: &OwnedFd) {
    let value = 1u64;
    loop {
        // SAFETY: value is readable for eight bytes and wake is an owned eventfd.
        let result = unsafe { libc::write(wake.as_raw_fd(), (&value as *const u64).cast(), 8) };
        if result >= 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            // EAGAIN means a notification is already pending (eventfd counter full).
            return;
        }
    }
}
