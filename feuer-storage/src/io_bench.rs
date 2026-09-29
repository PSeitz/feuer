//! Benchmark-only access to the production read queue without DataFile setup.

use std::{fs::File, io, os::fd::AsRawFd, sync::Arc};

use bytes::Bytes;

use crate::{IoMetrics, IoOperation, uring::IoQueueHandle};

/// One production read queue, including admission, buffer pooling, and its driver thread.
pub struct ReadQueue(IoQueueHandle);

impl ReadQueue {
    /// Uses an already-open read-only file; does not create, resize, or write it.
    pub fn new(file: Arc<File>) -> io::Result<Self> {
        // SAFETY: F_GETFL only queries flags of the live descriptor.
        let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if flags & libc::O_ACCMODE != libc::O_RDONLY {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "benchmark requires a read-only file",
            ));
        }
        // Raw read-only benchmarks need no cache directory lock. Retain the same
        // file in that ownership slot; leave queue and buffer lifetime logic intact.
        IoQueueHandle::new(file.clone(), file, IoOperation::Read, &IoMetrics::noop()).map(Self)
    }

    /// Reads through the production admission, allocation, notification, and completion path.
    pub async fn read(&self, offset: u64, length: usize) -> io::Result<Bytes> {
        self.0.read(offset, length).await
    }
}
