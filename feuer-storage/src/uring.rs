//! Separate single-owner rings for reads and writes. Only this module hands buffer pointers to the kernel.
//! Requests must be nonempty, with offsets and lengths aligned to DIRECT_IO_ALIGNMENT_BYTES.

use std::{
    fs::File,
    io,
    ops::Range,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::Arc,
    thread::{self, JoinHandle},
};

use bytes::Bytes;
use feuer_memory::{AlignedBuffer, BufferPool};
use io_uring::{IoUring, opcode, squeue, types};
use tokio::sync::{mpsc, oneshot};

use crate::{
    IoOperation,
    allocation::{DiskRegion, DiskRegionReadGuard},
};

// Buffer addresses, physical offsets, and I/O lengths use this alignment.
// Opening verifies that the filesystem's direct-I/O requirements divide it.
pub(crate) const DIRECT_IO_ALIGNMENT_BYTES: usize = feuer_memory::BUFFER_ALIGNMENT;
// Maximum physical bytes per chunk, including alignment padding. DataFile
// reduces the logical chunk size when its starting offset is unaligned.
pub(crate) const MAX_IO_CHUNK_BYTES: usize = 1024 * 1024;
// Each direction has this many active slots plus an equally sized waiting channel.
const MAX_IN_FLIGHT_READS: usize = 64;
// Local-SSD benchmarks saturated 1-MiB writes at QD8. QD64 added no write-only
// throughput, but raised write p99 from 4.6 to 47 ms and worsened small-read latency.
// See benchmarks/ssd/uring-20260928/REPORT.md.
const MAX_IN_FLIGHT_WRITES: usize = 8;

#[cfg(test)]
mod tests;

/// A handle that submits I/O requests and owns the queue thread's lifetime.
struct IoQueueHandle {
    // Sends admitted requests; taken on drop to signal shutdown before joining.
    sender: Option<mpsc::Sender<IoRequest>>,
    // Shared eventfd wakes the queue for new work or shutdown.
    wake_fd: Arc<OwnedFd>,
    // Queue thread; taken and joined on drop so submitted I/O drains first.
    thread: Option<JoinHandle<()>>,
}

impl IoQueueHandle {
    fn new(file: Arc<File>, directory_lock: Arc<File>, operation: IoOperation) -> io::Result<Self> {
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
        let (sender, receiver) = mpsc::channel(max_in_flight);
        let mut queue = IoQueue {
            ring,
            file: Some(file),
            directory_lock: Some(directory_lock),
            wake_fd: wake_fd.clone(),
            receiver,
            active: (0..max_in_flight).map(|_| None).collect(),
        };
        let thread = thread::Builder::new().name(thread_name.into()).spawn(move || {
            if let Err(error) = queue.process_requests_until_disconnected() {
                tracing::error!(target: "feuer::storage::io", operation = operation.as_str(), %error, "io_uring queue stopped");
            }
        })?;
        Ok(Self {
            sender: Some(sender),
            wake_fd,
            thread: Some(thread),
        })
    }

    /// Reserves channel capacity before preparing I/O buffers.
    async fn reserve_request(&self) -> io::Result<mpsc::Permit<'_, IoRequest>> {
        self.sender
            .as_ref()
            .unwrap()
            .reserve()
            .await
            .map_err(|_| queue_stopped_error())
    }

    /// Submits the admitted request to the queue and waits for its buffers or an I/O error.
    async fn submit_and_wait(
        &self,
        offset: u64,
        buffers: IoBuffers,
        destination: Range<usize>,
        permit: mpsc::Permit<'_, IoRequest>,
    ) -> io::Result<IoBuffers> {
        let (result_sender, result_receiver) = oneshot::channel();
        permit.send(IoRequest::new(offset, buffers, destination, result_sender));
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

/// Submits reads and allocates their buffers.
pub(crate) struct ReadQueue {
    handle: IoQueueHandle,
    buffer_pool: Arc<BufferPool>,
}

impl ReadQueue {
    pub(crate) fn new(file: Arc<File>, directory_lock: Arc<File>, buffer_pool: Arc<BufferPool>) -> io::Result<Self> {
        Ok(Self {
            handle: IoQueueHandle::new(file, directory_lock, IoOperation::Read)?,
            buffer_pool,
        })
    }

    pub(crate) fn allocate_buffer(
        &self,
        length: usize,
        read_guard: Option<DiskRegionReadGuard>,
    ) -> io::Result<AlignedIoBuffer> {
        AlignedIoBuffer::new(length, read_guard, &self.buffer_pool)
    }

    pub(crate) async fn read(&self, offset: u64, length: usize) -> io::Result<Bytes> {
        let permit = self.handle.reserve_request().await?;
        let buffer = self.allocate_buffer(length, None)?;
        Ok(self
            .handle
            .submit_and_wait(offset, IoBuffers::Read(buffer), 0..length, permit)
            .await?
            .into_read()
            .into_bytes())
    }

    /// One metadata read. Never waits for channel capacity or allocates a buffer without it.
    /// The scanner retries later when the channel is full; admitted requests run in FIFO order.
    pub(crate) async fn try_read_recovery_chunk(&self, region: DiskRegionReadGuard) -> io::Result<Option<Bytes>> {
        let length = MAX_IO_CHUNK_BYTES;
        let offset = region.range().start;
        assert_eq!(region.range().end - offset, length as u64);
        let permit = match self.handle.sender.as_ref().unwrap().try_reserve() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(_)) => return Ok(None),
            Err(mpsc::error::TrySendError::Closed(_)) => return Err(queue_stopped_error()),
        };
        let buffer = self.allocate_buffer(length, Some(region))?;
        Ok(Some(
            self.handle
                .submit_and_wait(offset, IoBuffers::Read(buffer), 0..length, permit)
                .await?
                .into_read()
                .into_bytes(),
        ))
    }

    /// Transfers exclusive buffer ownership to the queue until this slice completes.
    pub(crate) async fn read_into(
        &self,
        offset: u64,
        buffer: AlignedIoBuffer,
        destination: Range<usize>,
    ) -> io::Result<AlignedIoBuffer> {
        let permit = self.handle.reserve_request().await?;
        Ok(self
            .handle
            .submit_and_wait(offset, IoBuffers::Read(buffer), destination, permit)
            .await?
            .into_read())
    }
}

/// Submits writes, retaining input slices or using unpooled scratch buffers.
pub(crate) struct WriteQueue {
    handle: IoQueueHandle,
}

impl WriteQueue {
    pub(crate) fn new(file: Arc<File>, directory_lock: Arc<File>) -> io::Result<Self> {
        Ok(Self {
            handle: IoQueueHandle::new(file, directory_lock, IoOperation::Write)?,
        })
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
        assert!(length > 0 && length <= MAX_IO_CHUNK_BYTES);
        assert!(length.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES));
        let permit = self.handle.reserve_request().await?;
        let buffers = IoBuffers::from_write_parts(length, parts, region)?;
        self.handle.submit_and_wait(offset, buffers, 0..length, permit).await?;
        Ok(())
    }
}

fn queue_stopped_error() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "io_uring queue stopped")
}

/// Read memory and disk ownership retained until kernel I/O completes.
pub(crate) struct AlignedIoBuffer {
    // Release disk ownership before the allocation can return to its memory pool.
    read_guard: Option<DiskRegionReadGuard>,
    buffer: AlignedBuffer,
}

impl AlignedIoBuffer {
    fn new(length: usize, read_guard: Option<DiskRegionReadGuard>, pool: &Arc<BufferPool>) -> io::Result<Self> {
        Ok(Self {
            read_guard,
            buffer: pool.allocate(length)?,
        })
    }

    pub(crate) fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    pub(crate) fn into_bytes(self) -> Bytes {
        // No I/O owns the buffer now; returned bytes must not retain disk regions.
        drop(self.read_guard);
        self.buffer.into_bytes()
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        self.buffer.as_mut_slice()
    }
}

impl AsRef<[u8]> for AlignedIoBuffer {
    fn as_ref(&self) -> &[u8] {
        self.buffer.as_ref()
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
                let mut buffer = AlignedBuffer::allocate_zeroed(copy_length)?;
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
                // The active request exclusively owns this destination through completion.
                let ptr = buffer.as_mut_slice()[range.start..range.end].as_mut_ptr();
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

/// One I/O request, owning its buffers through completion.
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
}

impl IoRequest {
    fn new(
        offset: u64,
        buffers: IoBuffers,
        destination: Range<usize>,
        reply: oneshot::Sender<io::Result<IoBuffers>>,
    ) -> Self {
        assert!(offset.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES as u64));
        assert!(destination.start.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES));
        let length = destination.len();
        assert!(length.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES));
        match &buffers {
            IoBuffers::Read(buffer) => assert!(destination.end <= buffer.as_ref().len()),
            IoBuffers::Write { bytes, .. } => assert_eq!(destination, 0..bytes.iter().map(Bytes::len).sum()),
        }
        assert!(length > 0 && length <= MAX_IO_CHUNK_BYTES);
        Self {
            offset,
            buffers,
            destination,
            completed_bytes: 0,
            reply: Some(reply),
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

    /// Sends the buffers or error as the request result.
    fn send_result(mut self, result: io::Result<()>) {
        let result = result.map(|()| self.buffers);
        let _ = self.reply.take().unwrap().send(result);
    }
}

struct IoQueue {
    // Thread-owned kernel submission/completion queues; no cross-thread ring access.
    ring: IoUring,
    // Direct-I/O payload file; taken to retain ownership on abnormal exit.
    file: Option<Arc<File>>,
    // Shared directory lock; both queues must drain before it can be released.
    directory_lock: Option<Arc<File>>,
    // Eventfd polled alongside the ring so new work need not wait for completion.
    wake_fd: Arc<OwnedFd>,
    // Holds up to active.len() waiting requests; receive only into free active slots.
    receiver: mpsc::Receiver<IoRequest>,
    // Owns in-flight requests through completion; CQEs identify their slot indices.
    active: Vec<Option<IoRequest>>,
}

impl IoQueue {
    /// Processes requests until the sender disconnects and all queued and active I/O drains.
    fn process_requests_until_disconnected(&mut self) -> io::Result<()> {
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
            let disconnected = self.receive_requests();
            let has_active_requests = self.active.iter().any(Option::is_some);
            if disconnected && !has_active_requests {
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

    /// Receives uncanceled requests into free active slots and prepares their submissions.
    /// Returns true when the channel is disconnected and drained.
    fn receive_requests(&mut self) -> bool {
        for slot in 0..self.active.len() {
            if self.active[slot].is_some() {
                continue;
            }
            let request = loop {
                match self.receiver.try_recv() {
                    Ok(request) if request.reply.as_ref().unwrap().is_closed() => continue,
                    Ok(request) => break request,
                    Err(mpsc::error::TryRecvError::Empty) => return false,
                    Err(mpsc::error::TryRecvError::Disconnected) => return true,
                }
            };
            self.active[slot] = Some(request);
            self.queue_active_request(slot);
        }
        false
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
        self.receiver.close();
        if self.active.iter().any(Option::is_some) {
            // An abnormal queue exit cannot prove the kernel has stopped using pointers.
            // Closing a ring may tear it down asynchronously. Leak only requests still in active
            // slots and file/lock owners rather than risking use-after-free or early reuse.
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
