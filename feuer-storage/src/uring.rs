//! Separate single-owner rings for reads and writes. Only this module hands buffer pointers to the kernel.
//! Requests must be nonempty, with offsets and lengths aligned to DIRECT_IO_ALIGNMENT_BYTES.
//!
//! # Kernel pointer lifetime
//!
//! Active slots keep payload buffers and write descriptors alive at stable addresses until the
//! request's completion queue entry (CQE), including an error CQE. An error from submit() or poll()
//! does not prove the kernel has stopped using those buffers.
//!
//! EAGAIN retries pending submissions after a timed wait because no accepted I/O may exist to wake us.
//!
//! If the queue fails before we observe completion, we don't know whether the kernel still uses
//! the buffers. Drop leaks active slots rather than risk use-after-free.
//!
//! See <https://man7.org/linux/man-pages/man2/io_uring_enter.2.html> and
//! <https://man7.org/linux/man-pages/man7/io_uring_cancelation.7.html>.

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

use crate::IoOperation;

// Buffer addresses, physical offsets, and I/O lengths use this alignment.
// Opening verifies that the filesystem's direct-I/O requirements divide it.
pub(crate) const DIRECT_IO_ALIGNMENT_BYTES: usize = feuer_memory::BUFFER_ALIGNMENT;
// Maximum bytes transferred by one I/O request, including alignment padding.
pub(crate) const MAX_IO_REQUEST_BYTES: usize = 1024 * 1024;
// Each direction has this many active requests plus an equally sized waiting channel.
const MAX_IN_FLIGHT_READS: usize = 64;
// Local-SSD benchmarks saturated 1-MiB writes at QD8. QD64 added no write-only
// throughput, but raised write p99 from 4.6 to 47 ms and worsened small-read latency.
// See benchmarks/ssd/uring-20260928/REPORT.md.
const MAX_IN_FLIGHT_WRITES: usize = 8;

#[cfg(test)]
mod tests;

/// The data file and optional cache-directory lock retained together by I/O queues.
pub(crate) struct DataFileAndDirectoryLock {
    pub(crate) file: File,
    pub(crate) _directory_lock: Option<File>,
}

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
    fn new(files: Arc<DataFileAndDirectoryLock>, operation: IoOperation) -> io::Result<Self> {
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
            files,
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

    /// Submits the admitted request and waits for a read buffer, write success, or an I/O error.
    async fn submit_and_wait(
        &self,
        offset: u64,
        buffers: IoBuffers,
        buffer_range: Range<usize>,
        permit: mpsc::Permit<'_, IoRequest>,
    ) -> io::Result<Option<AlignedBuffer>> {
        let (result_sender, result_receiver) = oneshot::channel();
        permit.send(IoRequest::new(offset, buffers, buffer_range, result_sender));
        wake_queue(&self.wake_fd);
        result_receiver.await.map_err(|_| queue_stopped_error())?
    }
}

impl Drop for IoQueueHandle {
    fn drop(&mut self) {
        self.sender.take();
        wake_queue(&self.wake_fd);
        // The queue drains submitted I/O before releasing buffers and the directory lock.
        let _ = self.thread.take().map(JoinHandle::join);
    }
}

/// Submits reads and allocates their buffers.
pub(crate) struct ReadQueue {
    handle: IoQueueHandle,
    buffer_pool: Arc<BufferPool>,
}

impl ReadQueue {
    pub(crate) fn new(files: Arc<DataFileAndDirectoryLock>, buffer_pool: Arc<BufferPool>) -> io::Result<Self> {
        Ok(Self {
            handle: IoQueueHandle::new(files, IoOperation::Read)?,
            buffer_pool,
        })
    }

    pub(crate) fn allocate_buffer(&self, length: usize) -> io::Result<AlignedBuffer> {
        self.buffer_pool.allocate(length)
    }

    /// Reads into one allocation, splitting at the request-size limit.
    pub(crate) async fn read(&self, offset: u64, length: usize) -> io::Result<Bytes> {
        let mut permit = self.handle.reserve_request().await?;
        let mut buffer = self.allocate_buffer(length)?;
        let mut start = 0;
        loop {
            let end = (start + MAX_IO_REQUEST_BYTES).min(length);
            buffer = self
                .handle
                .submit_and_wait(offset + start as u64, IoBuffers::Read(buffer), start..end, permit)
                .await?
                .expect("read completion returns its buffer");
            if end == length {
                break;
            }
            start = end;
            permit = self.handle.reserve_request().await?;
        }
        Ok(buffer.into_bytes())
    }
}

/// Submits writes, retaining input slices or using unpooled scratch buffers.
pub(crate) struct WriteQueue {
    handle: IoQueueHandle,
}

impl WriteQueue {
    pub(crate) fn new(files: Arc<DataFileAndDirectoryLock>) -> io::Result<Self> {
        Ok(Self {
            handle: IoQueueHandle::new(files, IoOperation::Write)?,
        })
    }

    /// Writes bytes with zero padding to fill `length`, splitting at the request-size limit.
    /// Aligned payload slices are used directly; only other bytes need a copy.
    pub(crate) async fn write_padded(&self, offset: u64, length: usize, bytes: &Bytes) -> io::Result<()> {
        for start in (0..length).step_by(MAX_IO_REQUEST_BYTES) {
            let request_length = (length - start).min(MAX_IO_REQUEST_BYTES);
            let end = (start + request_length).min(bytes.len());
            let permit = self.handle.reserve_request().await?;
            let buffers = IoBuffers::from_write_bytes(request_length, bytes.slice(start.min(end)..end))?;
            self.handle
                .submit_and_wait(offset + start as u64, buffers, 0..request_length, permit)
                .await?;
        }
        Ok(())
    }
}

fn queue_stopped_error() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "io_uring queue stopped")
}

/// Buffers owned by a read or vectored write until completion.
enum IoBuffers {
    Read(AlignedBuffer),
    Write {
        bytes: [Bytes; 2],
        vectors: [libc::iovec; 2],
    },
}

impl IoBuffers {
    /// Takes the aligned prefix and copies the rest with zero padding.
    /// This prepares memory only; it does not issue a disk write.
    fn from_write_bytes(length: usize, mut bytes: Bytes) -> io::Result<Self> {
        assert!(bytes.len() <= length && length.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES));
        let aligned_length = if (bytes.as_ptr() as usize).is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES) {
            bytes.len() / DIRECT_IO_ALIGNMENT_BYTES * DIRECT_IO_ALIGNMENT_BYTES
        } else {
            0
        };
        let copy_length = length - aligned_length;
        let tail = if copy_length > 0 {
            let mut buffer = AlignedBuffer::allocate_zeroed(copy_length)?;
            buffer.as_mut_slice()[..bytes.len() - aligned_length].copy_from_slice(&bytes[aligned_length..]);
            buffer.into_bytes()
        } else {
            Bytes::new()
        };
        Ok(Self::Write {
            vectors: [libc::iovec {
                iov_base: std::ptr::null_mut(),
                iov_len: 0,
            }; 2],
            bytes: [bytes.split_to(aligned_length), tail],
        })
    }

    fn submission_entry(&mut self, fd: types::Fd, offset: u64, range: Range<usize>) -> squeue::Entry {
        match self {
            Self::Read(buffer) => {
                // The active request exclusively owns this destination through completion.
                let ptr = buffer.as_mut_slice()[range.start..range.end].as_mut_ptr();
                opcode::Read::new(fd, ptr, range.len() as u32).offset(offset).build()
            }
            Self::Write { bytes, vectors } => {
                let mut bytes_to_skip = range.start;
                for (bytes, vector) in bytes.iter().zip(vectors.iter_mut()) {
                    let skip = bytes_to_skip.min(bytes.len());
                    bytes_to_skip -= skip;
                    let remaining_bytes = &bytes[skip..];
                    *vector = libc::iovec {
                        iov_base: remaining_bytes.as_ptr().cast_mut().cast(),
                        iov_len: remaining_bytes.len(),
                    };
                }
                // The request owns both descriptors; empty slices transfer no bytes.
                opcode::Writev::new(fd, vectors.as_ptr(), vectors.len() as u32)
                    .offset(offset)
                    .build()
            }
        }
    }
}

// SAFETY: read allocations and immutable Bytes stay at stable addresses when moved.
// Only the queue submits pointers; write descriptors stay in its fixed active slots.
unsafe impl Send for IoBuffers {}

/// One I/O request, owning its buffers through completion; writes cover both buffers.
struct IoRequest {
    // Exclusive disk end of this request, unchanged by short completions.
    disk_end: u64,
    // Aligned memory for disk reads/writes; kept alive until I/O completes.
    buffers: IoBuffers,
    // Remaining byte range within the I/O buffers to read into or write from.
    buffer_range: Range<usize>,
    // Caller result channel; taken on finish/failure, also detects cancellation.
    reply: Option<oneshot::Sender<io::Result<Option<AlignedBuffer>>>>,
}

impl IoRequest {
    fn new(
        offset: u64,
        buffers: IoBuffers,
        buffer_range: Range<usize>,
        reply: oneshot::Sender<io::Result<Option<AlignedBuffer>>>,
    ) -> Self {
        assert!(offset.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES as u64));
        assert!(buffer_range.start.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES));
        let length = buffer_range.len();
        assert!(length.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES));
        if let IoBuffers::Read(buffer) = &buffers {
            assert!(buffer_range.end <= buffer.as_ref().len());
        }
        assert!(length > 0 && length <= MAX_IO_REQUEST_BYTES);
        Self {
            disk_end: offset + length as u64,
            buffers,
            buffer_range,
            reply: Some(reply),
        }
    }

    fn submission_entry(&mut self, fd: i32, request_index: usize) -> squeue::Entry {
        let offset = self.disk_end - self.buffer_range.len() as u64;
        self.buffers
            .submission_entry(types::Fd(fd), offset, self.buffer_range.clone())
            .user_data(request_index as u64)
    }

    /// Advances the remaining buffer range after a kernel completion; EINTR leaves it unchanged.
    fn apply_completion_result(&mut self, result: i32) -> io::Result<()> {
        if result == -libc::EINTR {
            return Ok(());
        }
        if result < 0 {
            return Err(io::Error::from_raw_os_error(-result));
        }
        let completion_bytes = result as usize;
        // An unaligned completion leaves a remainder that cannot be resubmitted with O_DIRECT.
        if completion_bytes == 0 || !completion_bytes.is_multiple_of(DIRECT_IO_ALIGNMENT_BYTES) {
            return Err(self.incomplete_io_error());
        }
        self.buffer_range.start += completion_bytes;
        Ok(())
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

    /// Returns the completed read buffer; writes release their buffers before reporting success.
    fn send_result(self, result: io::Result<()>) {
        let _ = self.reply.unwrap().send(result.map(|()| match self.buffers {
            IoBuffers::Read(buffer) => Some(buffer),
            IoBuffers::Write { .. } => None,
        }));
    }
}

struct IoQueue {
    // Thread-owned kernel submission/completion queues; no cross-thread ring access.
    ring: IoUring,
    // Both queues must drain before releasing the data file and directory lock.
    files: Arc<DataFileAndDirectoryLock>,
    // Eventfd polled alongside the ring so new work need not wait for completion.
    wake_fd: Arc<OwnedFd>,
    // Holds up to active.len() waiting requests; receive only when the active array has room.
    receiver: mpsc::Receiver<IoRequest>,
    // Fixed addresses for in-flight buffers and write descriptors; completions identify slot indexes.
    active: Box<[Option<IoRequest>]>,
}

impl IoQueue {
    /// Processes requests until the sender disconnects and all queued and active I/O drains.
    fn process_requests_until_disconnected(&mut self) -> io::Result<()> {
        self.process_requests_with_submit(|ring| ring.submit())
    }

    /// Uses the supplied submission function so tests can inject resource pressure and partial acceptance.
    fn process_requests_with_submit(
        &mut self,
        mut submit: impl FnMut(&mut IoUring) -> io::Result<usize>,
    ) -> io::Result<()> {
        loop {
            // Release the completion-queue borrow before retrying a short request.
            while let Some(completion) = { self.ring.completion().next() } {
                let request_index = completion.user_data() as usize;
                let request = self.active[request_index]
                    .as_mut()
                    .expect("completion for inactive request");
                match request.apply_completion_result(completion.result()) {
                    Ok(()) if !request.buffer_range.is_empty() => self.queue_active_request(request_index),
                    result => self.active[request_index].take().unwrap().send_result(result),
                }
            }
            self.receive_requests();
            if self.receiver.is_closed() && self.receiver.is_empty() && self.active.iter().all(Option::is_none) {
                return Ok(());
            }
            // submit() never waits for a completion. poll watches BOTH completions and
            // new requests, so an outstanding slow read cannot stall fresh submissions.
            match submit(&mut self.ring) {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    // EAGAIN leaves requests owned by their slots. Avoid spinning, but retry
                    // after 1 ms even if no request was accepted and no completion can wake us.
                    self.wait_for_completion_or_wakeup(1)?;
                    continue;
                }
                Err(error) => return Err(error),
            }
            // A short submission must be retried before sleeping; queued SQEs might
            // otherwise have no completion capable of waking us.
            if !self.ring.submission().is_empty() {
                continue;
            }
            self.wait_for_completion_or_wakeup(-1)?;
        }
    }

    /// Receives uncanceled requests into the active array where it is empty and prepares their submissions.
    fn receive_requests(&mut self) {
        for request_index in 0..self.active.len() {
            if self.active[request_index].is_some() {
                continue;
            }
            let request = loop {
                match self.receiver.try_recv() {
                    Ok(request) if request.reply.as_ref().unwrap().is_closed() => continue,
                    Ok(request) => break request,
                    Err(_) => return,
                }
            };
            self.active[request_index] = Some(request);
            self.queue_active_request(request_index);
        }
    }

    /// Queues the active request's remaining I/O in the ring without submitting it to the kernel yet.
    fn queue_active_request(&mut self, request_index: usize) {
        let entry = self.active[request_index]
            .as_mut()
            .unwrap()
            .submission_entry(self.files.file.as_raw_fd(), request_index);
        // SAFETY: the fixed active slot owns buffers and descriptors through completion.
        // The ring is sized for all active requests, each with at most one submitted or queued SQE.
        unsafe {
            self.ring
                .submission()
                .push(&entry)
                .expect("ring sized for all active requests")
        };
    }

    /// Waits for a ring completion or an eventfd wakeup for new requests or shutdown.
    fn wait_for_completion_or_wakeup(&self, timeout_millis: i32) -> io::Result<()> {
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
        let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_millis) };
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
            // Closing a ring may tear it down asynchronously. Leak the fixed active slots
            // and their file owner rather than risking use-after-free or early reuse.
            // Normal shutdown drains all completions and never takes this path.
            tracing::error!(target: "feuer::storage::io", "retaining active I/O resources after queue failure");
            for request in Box::leak(std::mem::take(&mut self.active)).iter_mut().flatten() {
                let _ = request.reply.take().unwrap().send(Err(queue_stopped_error()));
            }
            std::mem::forget(self.files.clone());
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
