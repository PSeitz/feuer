use std::{
    fmt,
    fs::{File, OpenOptions, create_dir_all},
    future::Future,
    io,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use bytes::Bytes;
use feuer_memory::BufferPool;
use fs4::fs_std::FileExt as LockFileExt;
use tokio::runtime::Handle;
use tracing::{Instrument, Span, field};

use crate::{DataFileError, DataFileResult, IoMetrics, IoOperation, uring};

const DATA_FILE_NAME: &str = "data";
const LOCK_FILE_NAME: &str = ".feuer.lock";

/// Disk byte count for a payload, including alignment padding and a 4-KiB minimum.
pub(crate) fn payload_disk_bytes(length: u64) -> u64 {
    length.max(1).next_multiple_of(uring::DIRECT_IO_ALIGNMENT_BYTES as u64)
}

/// Queues, path, capacity, and metrics shared by cloned data-file handles.
struct DataFileState {
    read_queue: uring::ReadQueue,
    write_queue: uring::WriteQueue,
    data_path: PathBuf,
    capacity: u64,
    metrics: Arc<IoMetrics>,
}

/// One exclusively owned, fixed-capacity Linux direct-I/O payload file.
///
/// Reads and writes have separate io_uring rings and worker threads, with up to
/// 64 active reads and 8 active writes, plus waiting channels for another 64 reads and 8 writes.
/// Queues schedule in arrival order without checking for conflicts.
/// Submission order does not guarantee completion order.
/// Each I/O request transfers at most 1 MiB. Request limits do not bound full read-result allocations.
///
/// # Caller-owned concurrency and cancellation
///
/// This file neither checks conflicts nor protects disk regions from reuse.
/// Overlapping I/O can overwrite or mix disk contents. Callers must either serialize
/// conflicting access or validate returned buffers against expected checksums and
/// accept mismatches as cache misses.
///
/// Dropping an I/O future does not cancel submitted kernel I/O. An abandoned write
/// can overwrite a reused region, even after a newer write completes. The queue
/// retains I/O buffers until completion, but holds no disk reservations.
///
/// Capacity must be a positive multiple of 4096. Opening fails
/// if io_uring or verified O_DIRECT alignment is unavailable; there is no fallback.
/// Normal last-handle drop drains submitted I/O and joins both queue threads, which can block.
/// Temporary submission resource pressure (EAGAIN) is retried. A queue-level error is not a
/// completion for outstanding requests: on unrecoverable queue failure or thread unwind, their
/// backing allocations and the file/directory lock are retained until process exit to prevent
/// use-after-free and late writes into a reopened cache. Per-request error completions release
/// buffers normally. Returned Bytes never retain the file or queue.
#[derive(Clone)]
pub struct DataFile {
    state: Arc<DataFileState>,
}

impl fmt::Debug for DataFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataFile")
            .field("capacity", &self.state.capacity)
            .finish_non_exhaustive()
    }
}

impl DataFile {
    /// Opens a raw O_DIRECT data file and exclusively locks its cache directory.
    ///
    /// Requires Linux with io_uring and filesystem STATX_DIOALIGN support. The
    /// directory is created if needed and the file is set to exactly `capacity`.
    /// A second concurrent open fails with [`crate::DataFileErrorKind::AlreadyOpen`].
    pub async fn open(directory: impl AsRef<Path>, capacity: u64, metrics: Arc<IoMetrics>) -> DataFileResult<Self> {
        Self::open_with_buffer_pool(directory, capacity, metrics, BufferPool::unpooled()).await
    }

    /// Opens storage using the buffer pool belonging to its memory-cache instance.
    pub async fn open_with_buffer_pool(
        directory: impl AsRef<Path>,
        capacity: u64,
        metrics: Arc<IoMetrics>,
        buffer_pool: Arc<BufferPool>,
    ) -> DataFileResult<Self> {
        let directory = directory.as_ref().to_path_buf();
        let runtime = Handle::try_current().map_err(|_| DataFileError::RuntimeUnavailable)?;
        let started = Instant::now();
        let span = tracing::info_span!(
            target: "feuer::storage::io",
            "feuer.storage.data_file.open",
            capacity,
            outcome = field::Empty,
            error_kind = field::Empty,
            duration_seconds = field::Empty,
        );
        let result = async {
            let state = runtime
                .spawn_blocking(move || open_file_state(directory, capacity, buffer_pool, metrics))
                .await
                .map_err(|source| DataFileError::Task {
                    operation: IoOperation::OpenDataFile,
                    source: Box::new(source),
                })??;
            let file = Self { state: Arc::new(state) };
            // Exercise an actual aligned direct read before claiming open succeeded.
            file.state
                .read_queue
                .read(0, uring::DIRECT_IO_ALIGNMENT_BYTES)
                .await
                .map_err(|source| file.io_error(IoOperation::OpenDataFile, source))?;
            Ok(file)
        }
        .instrument(span.clone())
        .await;
        record_span_outcome(&span, started.elapsed(), &result);
        result
    }

    /// Returns the fixed physical capacity in bytes.
    pub fn capacity(&self) -> u64 {
        self.state.capacity
    }

    /// Reads exactly the requested bytes with at most two partial alignment pages
    /// of overhead. The result may retain alignment padding in its backing allocation.
    /// Callers must either prevent overlapping writes or validate the returned
    /// bytes against an expected checksum; see [`DataFile`]'s concurrency contract.
    pub async fn read_at(&self, offset: u64, length: usize) -> DataFileResult<Bytes> {
        self.measure_io(IoOperation::Read, offset, length, async {
            check_range_fits_file(IoOperation::Read, offset, length as u64, self.state.capacity)?;
            if length == 0 {
                return Ok(Bytes::new());
            }
            self.read_slice(offset, length).await
        })
        .await
    }

    /// Reads one contiguous payload into a buffer, omitting final alignment padding.
    /// The caller supplies the aligned payload address; even empty payloads occupy one alignment page.
    pub(crate) async fn read_payload(&self, address: u64, length: usize) -> DataFileResult<Bytes> {
        self.measure_io(IoOperation::Read, address, length, self.read_slice(address, length))
            .await
    }

    /// Copies a payload read result only if it saves at least 25% of backing capacity and at least 32 KiB.
    /// Derives read capacity from the whole payload length. Allocation failure leaves the result unchanged.
    pub(crate) fn shrink_read_buffer(&self, bytes: Bytes, payload_length: u64) -> (Bytes, usize) {
        let capacity = BufferPool::allocation_capacity(payload_disk_bytes(payload_length) as usize);
        let saved_bytes = capacity - BufferPool::allocation_capacity(bytes.len());
        if saved_bytes < 32 * 1024 || saved_bytes < capacity.div_ceil(4) {
            return (bytes, capacity);
        }
        let Ok(mut buffer) = self.state.read_queue.allocate_buffer(bytes.len()) else {
            return (bytes, capacity);
        };
        buffer.as_mut_slice().copy_from_slice(&bytes);
        let capacity = buffer.capacity();
        (buffer.into_bytes(), capacity)
    }

    /// Writes bytes at a 4096-byte-aligned offset; the byte count must also be a multiple of 4096.
    ///
    /// Retains aligned byte slices directly; copies unaligned inputs into aligned
    /// buffers of at most 1 MiB each. Dropping this future may leave a partial write
    /// and does not stop submitted writes; see [`DataFile`]'s concurrency contract.
    /// Publish a cached byte range only after success.
    ///
    /// # Panics
    ///
    /// Panics if the offset or byte count is not a multiple of 4096.
    pub async fn write_at(&self, offset: u64, bytes: &Bytes) -> DataFileResult<()> {
        assert!(offset.is_multiple_of(uring::DIRECT_IO_ALIGNMENT_BYTES as u64));
        assert!(bytes.len().is_multiple_of(uring::DIRECT_IO_ALIGNMENT_BYTES));
        self.write_padded(offset, bytes.len(), bytes).await
    }

    /// Writes bytes with zero padding to fill an aligned disk byte range without an intermediate buffer.
    /// The queue splits requests at 1 MiB and retains only I/O buffers.
    pub(crate) async fn write_padded(&self, offset: u64, length: usize, bytes: &Bytes) -> DataFileResult<()> {
        self.measure_io(IoOperation::Write, offset, length, async {
            check_range_fits_file(IoOperation::Write, offset, length as u64, self.state.capacity)?;
            self.state
                .write_queue
                .write_padded(offset, length, bytes)
                .await
                .map_err(|source| self.io_error(IoOperation::Write, source))
        })
        .await
    }

    fn io_error(&self, operation: IoOperation, source: io::Error) -> DataFileError {
        DataFileError::Io {
            operation,
            path: self.state.data_path.clone(),
            source,
        }
    }

    async fn measure_io<T>(
        &self,
        operation: IoOperation,
        offset: u64,
        length: usize,
        execute: impl Future<Output = DataFileResult<T>>,
    ) -> DataFileResult<T> {
        let started = Instant::now();
        let observed_bytes = u64::try_from(length).unwrap_or(u64::MAX);
        let span = tracing::trace_span!(
            target: "feuer::storage::io",
            "feuer.storage.data_file.io",
            operation = operation.as_str(),
            offset,
            bytes = observed_bytes,
            outcome = field::Empty,
            error_kind = field::Empty,
            duration_seconds = field::Empty,
        );
        let result = execute.instrument(span.clone()).await;
        let elapsed = started.elapsed();
        self.state
            .metrics
            .record(operation, observed_bytes, elapsed, result.is_ok());
        record_span_outcome(&span, elapsed, &result);
        result
    }

    /// Reads exact bytes through aligned I/O, omitting padding.
    async fn read_slice(&self, offset: u64, length: usize) -> DataFileResult<Bytes> {
        let operation = IoOperation::Read;
        let padding = offset % uring::DIRECT_IO_ALIGNMENT_BYTES as u64;
        let buffer_length = usize::try_from(payload_disk_bytes(padding + length as u64)).map_err(|_| {
            DataFileError::LengthOverflow {
                operation,
                length: usize::MAX,
            }
        })?;
        let bytes = self
            .state
            .read_queue
            .read(offset - padding, buffer_length)
            .await
            .map_err(|source| self.io_error(operation, source))?;
        Ok(bytes.slice(padding as usize..padding as usize + length))
    }
}

fn open_file_state(
    directory: PathBuf,
    capacity: u64,
    buffer_pool: Arc<BufferPool>,
    metrics: Arc<IoMetrics>,
) -> DataFileResult<DataFileState> {
    if capacity == 0 || capacity > i64::MAX as u64 || !capacity.is_multiple_of(uring::DIRECT_IO_ALIGNMENT_BYTES as u64)
    {
        return Err(DataFileError::InvalidCapacity);
    }
    create_dir_all(&directory).map_err(|source| DataFileError::Io {
        operation: IoOperation::CreateDirectory,
        path: directory.clone(),
        source,
    })?;
    let lock_path = directory.join(LOCK_FILE_NAME);
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|source| DataFileError::Io {
            operation: IoOperation::OpenLockFile,
            path: lock_path.clone(),
            source,
        })?;
    let locked = LockFileExt::try_lock_exclusive(&lock_file).map_err(|source| DataFileError::Io {
        operation: IoOperation::LockDirectory,
        path: lock_path,
        source,
    })?;
    if !locked {
        return Err(DataFileError::AlreadyOpen { directory });
    }
    let data_path = directory.join(DATA_FILE_NAME);
    let error = |operation, source| DataFileError::Io {
        operation,
        path: data_path.clone(),
        source,
    };
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .custom_flags(libc::O_DIRECT)
        .open(&data_path)
        .map_err(|source| error(IoOperation::OpenDataFile, source))?;
    if !file
        .metadata()
        .map_err(|source| error(IoOperation::InspectDataFile, source))?
        .is_file()
    {
        return Err(DataFileError::InvalidDataFile { path: data_path });
    }
    check_direct_io_alignment(&file).map_err(|source| error(IoOperation::InspectDataFile, source))?;
    // Construct both rings before resizing, so unavailable io_uring does not resize an existing cache.
    let files = Arc::new(uring::DataFileAndDirectoryLock {
        file,
        _directory_lock: Some(lock_file),
    });
    let read_queue =
        uring::ReadQueue::new(files.clone(), buffer_pool).map_err(|source| error(IoOperation::OpenDataFile, source))?;
    let write_queue =
        uring::WriteQueue::new(files.clone()).map_err(|source| error(IoOperation::OpenDataFile, source))?;
    files
        .file
        .set_len(capacity)
        .map_err(|source| error(IoOperation::ResizeDataFile, source))?;
    Ok(DataFileState {
        read_queue,
        write_queue,
        data_path,
        capacity,
        metrics,
    })
}

fn check_direct_io_alignment(file: &File) -> io::Result<()> {
    let mut stat = std::mem::MaybeUninit::<libc::statx>::zeroed();
    // SAFETY: the fd is owned, the empty path is NUL-terminated, and stat is writable.
    let result = unsafe {
        libc::statx(
            file.as_raw_fd(),
            c"".as_ptr(),
            libc::AT_EMPTY_PATH,
            libc::STATX_DIOALIGN,
            stat.as_mut_ptr(),
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: statx succeeded and initialized the output.
    let stat = unsafe { stat.assume_init() };
    if stat.stx_mask & libc::STATX_DIOALIGN == 0
        || !uring::DIRECT_IO_ALIGNMENT_BYTES.is_multiple_of(stat.stx_dio_mem_align as usize)
        || !uring::DIRECT_IO_ALIGNMENT_BYTES.is_multiple_of(stat.stx_dio_offset_align as usize)
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "filesystem does not report compatible direct I/O alignment",
        ));
    }
    Ok(())
}

/// Checks that the requested byte range fits in the file, rejecting offset-plus-length overflow.
fn check_range_fits_file(operation: IoOperation, offset: u64, length: u64, capacity: u64) -> DataFileResult<()> {
    if offset.checked_add(length).map(|end| end > capacity).unwrap_or(true) {
        return Err(DataFileError::RangeExceedsCapacity {
            operation,
            offset,
            length,
            capacity,
        });
    }
    Ok(())
}

fn record_span_outcome<T>(span: &Span, elapsed: std::time::Duration, result: &DataFileResult<T>) {
    span.record("duration_seconds", elapsed.as_secs_f64());
    span.record("outcome", if result.is_ok() { "success" } else { "error" });
    if let Err(error) = result {
        span.record("error_kind", error.kind().as_str());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DataFileErrorKind, allocation::CHUNK_BYTES};
    use tempfile::tempdir;

    const CAPACITY: u64 = 4 * uring::MAX_IO_REQUEST_BYTES as u64;

    #[test]
    fn public_io_types_are_send_sync_static() {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}
        assert_send_sync_static::<DataFile>();
        assert_send_sync_static::<DataFileError>();
    }

    #[tokio::test]
    async fn reads_exact_unaligned_ranges_after_aligned_writes() {
        let temp = tempdir().unwrap();
        let directory = temp.path().join("cache/inner");
        let file = DataFile::open(&directory, CAPACITY, IoMetrics::noop()).await.unwrap();
        let original = Bytes::from(vec![0x55; CAPACITY as usize]);
        file.write_at(0, &original).await.unwrap();
        let mut page = vec![0x55; 4096];
        page[3..31].copy_from_slice(b"unaligned positional payload");
        file.write_at(0, &Bytes::from(page)).await.unwrap();
        assert_eq!(file.read_at(13, 10).await.unwrap(), Bytes::from_static(b"positional"));
        assert_eq!(file.read_at(0, 3).await.unwrap(), original.slice(..3));
        assert_eq!(file.read_at(4096, 16).await.unwrap(), original.slice(..16));
        // Exercise multi-chunk writes, unaligned reads across a chunk boundary,
        // and the final physical page.
        let payload = Bytes::from(
            (0..uring::MAX_IO_REQUEST_BYTES + 4096)
                .map(|index| (index % 251) as u8)
                .collect::<Vec<_>>(),
        );
        file.write_at(4096, &payload).await.unwrap();
        assert_eq!(
            file.read_at(4099, payload.len() - 6).await.unwrap(),
            payload.slice(3..payload.len() - 3)
        );
        let mut page = vec![0x55; 4096];
        page[4093..].copy_from_slice(b"end");
        file.write_at(CAPACITY - 4096, &Bytes::from(page)).await.unwrap();
        assert_eq!(file.read_at(CAPACITY - 3, 3).await.unwrap(), Bytes::from_static(b"end"));
        assert_eq!(
            std::fs::metadata(directory.join(DATA_FILE_NAME)).unwrap().len(),
            CAPACITY
        );
    }

    #[tokio::test]
    async fn read_buffer_copy_requires_minimum_savings() {
        let temp = tempdir().unwrap();
        let file = DataFile::open(temp.path(), CAPACITY, IoMetrics::noop()).await.unwrap();
        file.write_at(0, &Bytes::from(vec![0x55; CAPACITY as usize]))
            .await
            .unwrap();
        let mib = CHUNK_BYTES as usize;
        for (payload_length, slice_length, expected_capacity, copied) in [
            (3 * mib, 2 * mib, 2 * mib, true),
            (4 * mib, 2 * mib, 2 * mib, true),
            (4 * mib, 2 * mib + 1, 3 * mib, true),
            (4 * mib, 4 * mib - 1, 4 * mib, false),
            (3 * mib, mib, mib, true),
        ] {
            let source = file.read_payload(0, payload_length).await.unwrap();
            let slice = source.slice(..slice_length);
            let (bytes, capacity) = file.shrink_read_buffer(slice.clone(), payload_length as u64);
            assert_eq!(bytes, slice);
            assert_eq!(capacity, expected_capacity);
            assert_eq!(bytes.as_ptr() != source.as_ptr(), copied);
        }
    }

    #[tokio::test]
    async fn recovery_reads_one_metadata_chunk_without_neighbors() {
        let temp = tempdir().unwrap();
        let file = DataFile::open(temp.path(), 2 * CHUNK_BYTES, IoMetrics::noop())
            .await
            .unwrap();
        let metadata = Bytes::from(vec![0x55; CHUNK_BYTES as usize]);
        file.write_at(0, &metadata).await.unwrap();
        file.write_at(CHUNK_BYTES, &Bytes::from(vec![0x99; CHUNK_BYTES as usize]))
            .await
            .unwrap();
        let recovered = file.read_at(0, CHUNK_BYTES as usize).await.unwrap();
        assert_eq!(recovered.len(), metadata.len());
        assert_eq!(recovered, metadata);
    }

    #[tokio::test]
    async fn disk_reads_preserve_held_bytes_across_overwrites() {
        let temp = tempdir().unwrap();
        let file = DataFile::open(temp.path(), CAPACITY, IoMetrics::noop()).await.unwrap();
        let length = 2 * uring::DIRECT_IO_ALIGNMENT_BYTES;
        file.write_at(0, &Bytes::from(vec![0x55; length])).await.unwrap();
        let bytes = file.read_at(0, length).await.unwrap();
        let slice = bytes.slice(1..);
        drop(bytes);
        file.write_at(0, &Bytes::from(vec![0x99; length])).await.unwrap();
        let other = file.read_at(0, length).await.unwrap();
        assert_eq!(&slice[..], &vec![0x55; length - 1]);
        drop(slice);
        let latest = file.read_at(0, length).await.unwrap();
        assert_eq!(&latest[..], &vec![0x99; length]);
        drop(file);
        assert_eq!(latest, other);
    }

    #[tokio::test]
    async fn rejects_ranges_exceeding_capacity_and_accepts_empty_ranges() {
        let temp = tempdir().unwrap();
        let file = DataFile::open(temp.path(), CAPACITY, IoMetrics::noop()).await.unwrap();
        assert_eq!(
            file.read_at(CAPACITY - 1, 2).await.unwrap_err().kind(),
            DataFileErrorKind::RangeExceedsCapacity
        );
        let bytes = Bytes::from(vec![0; 4096]);
        for result in [
            file.write_at(u64::MAX - 4095, &bytes).await,
            file.write_padded(u64::MAX - 4095, bytes.len(), &bytes).await,
        ] {
            assert_eq!(result.unwrap_err().kind(), DataFileErrorKind::RangeExceedsCapacity);
        }
        assert!(file.read_at(CAPACITY, 0).await.unwrap().is_empty());
        assert!(file.read_payload(0, 0).await.unwrap().is_empty());
        assert!(file.read_payload(CAPACITY, 0).await.is_err());
        file.write_at(CAPACITY, &Bytes::new()).await.unwrap();
    }

    #[tokio::test]
    async fn fails_short_reads_instead_of_returning_uncertain_bytes() {
        let temp = tempdir().unwrap();
        let file = DataFile::open(temp.path(), CAPACITY, IoMetrics::noop()).await.unwrap();
        OpenOptions::new()
            .write(true)
            .open(temp.path().join(DATA_FILE_NAME))
            .unwrap()
            .set_len(4)
            .unwrap();
        match file.read_at(0, 8).await.unwrap_err() {
            DataFileError::Io { source, .. } => assert_eq!(source.kind(), io::ErrorKind::UnexpectedEof),
            error => panic!("unexpected error: {error:?}"),
        }
    }

    #[tokio::test]
    async fn holds_exclusive_ownership_and_reopens_after_shutdown() {
        let temp = tempdir().unwrap();
        let file = DataFile::open(temp.path(), CAPACITY, IoMetrics::noop()).await.unwrap();
        let clone = file.clone();
        drop(file);
        assert_eq!(
            DataFile::open(temp.path(), CAPACITY, IoMetrics::noop())
                .await
                .unwrap_err()
                .kind(),
            DataFileErrorKind::AlreadyOpen
        );
        let mut page = vec![0; 4096];
        page[17..27].copy_from_slice(b"persistent");
        clone.write_at(0, &Bytes::from(page)).await.unwrap();
        drop(clone);
        let file = DataFile::open(temp.path(), CAPACITY / 2, IoMetrics::noop())
            .await
            .unwrap();
        assert_eq!(file.capacity(), CAPACITY / 2);
        assert_eq!(file.read_at(17, 10).await.unwrap(), Bytes::from_static(b"persistent"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_mixed_io_with_caller_serialized_shared_page() {
        let temp = tempdir().unwrap();
        let file = DataFile::open(temp.path(), CAPACITY, IoMetrics::noop()).await.unwrap();
        let mut tasks = Vec::new();
        let shared_pages = Arc::new(tokio::sync::Mutex::new(()));
        // Callers serialize access to shared pages; page-disjoint I/O runs concurrently.
        for i in 0..256u64 {
            let file = file.clone();
            let shared_pages = shared_pages.clone();
            tasks.push(tokio::spawn(async move {
                {
                    let _guard = shared_pages.lock().await;
                    let value = Bytes::from(vec![(i % 251) as u8; 4096]);
                    file.write_at(0, &value).await.unwrap();
                    assert_eq!(file.read_at(3, 31).await.unwrap(), value.slice(3..34));
                }
                let value = Bytes::from(vec![(i % 251) as u8; 4096]);
                let offset = 16384 + i * 4096;
                file.write_at(offset, &value).await.unwrap();
                assert_eq!(file.read_at(offset, value.len()).await.unwrap(), value);
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        for i in 0..256u64 {
            assert_eq!(
                file.read_at(16384 + i * 4096, 4096).await.unwrap(),
                Bytes::from(vec![(i % 251) as u8; 4096])
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn canceled_callers_do_not_release_submitted_buffers_or_lock_early() {
        let temp = tempdir().unwrap();
        let file = DataFile::open(temp.path(), CAPACITY, IoMetrics::noop()).await.unwrap();
        let mut tasks = Vec::new();
        for i in 0..256u64 {
            let file = file.clone();
            tasks.push(tokio::spawn(async move {
                file.write_at(i * 4096, &Bytes::from(vec![0xab; 4096])).await
            }));
        }
        tokio::task::yield_now().await;
        for task in &tasks {
            task.abort();
        }
        for task in tasks {
            let _ = task.await;
        }
        drop(file); // joins the queue thread, even if completion receivers were dropped
        let file = DataFile::open(temp.path(), CAPACITY, IoMetrics::noop()).await.unwrap();
        file.write_at(0, &Bytes::from(vec![0x55; uring::MAX_IO_REQUEST_BYTES]))
            .await
            .unwrap();
        assert_eq!(
            file.read_at(0, uring::MAX_IO_REQUEST_BYTES).await.unwrap(),
            Bytes::from(vec![0x55; uring::MAX_IO_REQUEST_BYTES])
        );
    }

    #[test]
    fn reports_missing_runtime() {
        use std::{
            future::Future,
            task::{Context, Poll, Waker},
        };
        let temp = tempdir().unwrap();
        let mut open = Box::pin(DataFile::open(temp.path(), CAPACITY, IoMetrics::noop()));
        let mut context = Context::from_waker(Waker::noop());
        let Poll::Ready(result) = open.as_mut().poll(&mut context) else {
            panic!("unexpectedly pending");
        };
        assert_eq!(result.unwrap_err().kind(), DataFileErrorKind::Task);
    }

    #[test]
    fn rejects_unverified_direct_io_instead_of_falling_back() {
        use std::os::fd::FromRawFd;

        // A memory-backed file has no usable disk direct-I/O alignment.
        // SAFETY: name is NUL-terminated; memfd_create returns a fresh descriptor.
        let fd = unsafe { libc::memfd_create(c"feuer-alignment-test".as_ptr(), libc::MFD_CLOEXEC) };
        assert!(fd >= 0);
        // SAFETY: fd was just created and has no other owner.
        let file = unsafe { File::from_raw_fd(fd) };
        assert_eq!(
            check_direct_io_alignment(&file).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
    }

    #[tokio::test]
    async fn validates_capacity_and_omits_paths_from_debug() {
        let temp = tempdir().unwrap();
        for capacity in [0, 1, 4095, 4097, u64::MAX, 1 << 63] {
            assert_eq!(
                DataFile::open(temp.path(), capacity, IoMetrics::noop())
                    .await
                    .unwrap_err()
                    .kind(),
                DataFileErrorKind::InvalidConfiguration
            );
        }
        let file = DataFile::open(temp.path(), CAPACITY, IoMetrics::noop()).await.unwrap();
        assert_eq!(format!("{file:?}"), format!("DataFile {{ capacity: {CAPACITY}, .. }}"));
    }
}
