//! Benchmark-only access to the production read queue without DataFile setup.

use std::{fs::File, io, os::fd::AsRawFd, sync::Arc};

use bytes::Bytes;

use crate::uring;

/// One production read queue and its driver thread, without a memory cache or idle retention.
pub struct ReadQueue(uring::ReadQueue);

impl ReadQueue {
    /// Owns an already-open read-only file; does not create, resize, or write it.
    pub fn new(file: File) -> io::Result<Self> {
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
        // Raw read-only benchmarks need no cache file lock.
        let files = Arc::new(uring::DataFileAndDirectoryLock {
            file,
            _directory_lock: None,
        });
        uring::ReadQueue::new(files, feuer_memory::BufferPool::unpooled()).map(Self)
    }

    /// Reads through the production admission, allocation, notification, and completion path.
    pub async fn read(&self, offset: u64, length: usize) -> io::Result<Bytes> {
        self.0.read(offset, length).await.map(|(bytes, _)| bytes)
    }
}
