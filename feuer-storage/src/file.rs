use std::{
    fmt,
    fs::{File, OpenOptions, create_dir_all},
    io,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use bytes::Bytes;
use fs4::fs_std::FileExt as LockFileExt;
use tokio::runtime::Handle;
use tracing::{Instrument, Span, field};

use crate::{DataFileError, DataFileResult, IoMetrics, IoOperation, uring};

const DATA_FILE_NAME: &str = "data";
const LOCK_FILE_NAME: &str = ".feuer.lock";

/// Queue, path, and capacity state shared by cloned data-file handles.
struct DataFileState {
    queue: uring::IoQueueHandle,
    data_path: PathBuf,
    capacity: u64,
}

/// One exclusively owned, fixed-capacity Linux direct-I/O payload file.
///
/// A dedicated io_uring thread overlaps up to 64 operations, scheduling requests
/// in arrival order without checking for conflicts. Submission order does not
/// guarantee completion order. Reads do not throttle writes; either class can fill the ring.
/// Admission reserves 64 requests and 64 MiB of staging buffers for each of
/// reads and writes. Caller inputs and read-result allocations are outside
/// this staging-buffer budget.
/// Reads and writes accept arbitrary byte ranges; unaligned writes use
/// read-modify-write. Multi-chunk operations are not atomic. Write completion
/// permits subsequent reads, but does not guarantee crash durability.
///
/// # Caller-owned concurrency and cancellation
///
/// Callers must prevent overlapping I/O when either operation is a write. This
/// applies to physical byte ranges rounded outward to 4 KiB boundaries, not just
/// the requested bytes: disjoint unaligned writes can modify the same page.
/// Multiple reads may run concurrently. The upper storage layer owns disk-region
/// allocation and read guards; this file neither checks conflicts nor protects
/// disk regions from reuse.
///
/// Dropping an I/O future does not cancel submitted kernel I/O. In particular,
/// an abandoned write may still modify disk. Its disk region must remain reserved
/// and protected against conflicting access until the submitted I/O completes.
/// Keep the write future running to completion in the task that owns the region;
/// abandoning the result must not abort that task or release its reservation.
/// On queue failure, do not reuse regions whose completion is unknown.
/// Submitted buffers remain owned by this I/O layer until completion, even when
/// the caller drops its future. A canceled read whose result is discarded no
/// longer requires its disk contents to remain unchanged.
///
/// Capacity must be a positive multiple of 4096, at most i64::MAX. Opening fails
/// if io_uring or verified O_DIRECT alignment is unavailable; there is no fallback.
/// Dropping the last handle drains submitted I/O and joins the queue thread,
/// which can block. Returned Bytes never retain the file or queue.
#[derive(Clone)]
pub struct DataFile {
    state: Arc<DataFileState>,
    metrics: Arc<IoMetrics>,
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
                .spawn_blocking(move || open_file_state(directory, capacity))
                .await
                .map_err(|source| DataFileError::Task {
                    operation: IoOperation::OpenDataFile,
                    source: Box::new(source),
                })??;
            // Exercise an actual aligned direct read before claiming open succeeded.
            state
                .queue
                .execute(IoOperation::Read, 0, uring::DIRECT_IO_ALIGNMENT_BYTES, &[])
                .await
                .map_err(|source| DataFileError::Io {
                    operation: IoOperation::OpenDataFile,
                    path: state.data_path.clone(),
                    source,
                })?;
            Ok(Self {
                state: Arc::new(state),
                metrics,
            })
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
    /// of overhead. The result owns a compact allocation, not an I/O buffer.
    /// Callers must prevent writes to the aligned byte range while this read
    /// depends on its contents; see [`DataFile`]'s concurrency contract.
    pub async fn read_at(&self, offset: u64, length: usize) -> DataFileResult<Bytes> {
        self.execute_measured(IoOperation::Read, offset, length, &[]).await
    }

    /// Writes arbitrary bytes without modifying neighboring bytes in their pages.
    ///
    /// Internally copies bounded chunks into aligned buffers. Callers must protect
    /// the full aligned byte range from conflicting access through completion.
    /// Dropping this future may leave a partial write and does not stop submitted
    /// writes: retain the disk region until they complete, as described in
    /// [`DataFile`]'s cancellation contract. Publish a cached byte range only after success.
    pub async fn write_at(&self, offset: u64, bytes: &Bytes) -> DataFileResult<()> {
        self.execute_measured(IoOperation::Write, offset, bytes.len(), bytes)
            .await
            .map(|_| ())
    }

    async fn execute_measured(
        &self,
        operation: IoOperation,
        offset: u64,
        length: usize,
        payload: &[u8],
    ) -> DataFileResult<Bytes> {
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
        let result = self
            .execute_chunks(operation, offset, length, payload)
            .instrument(span.clone())
            .await;
        let elapsed = started.elapsed();
        self.metrics.record(operation, observed_bytes, elapsed, result.is_ok());
        record_span_outcome(&span, elapsed, &result);
        result
    }

    async fn execute_chunks(
        &self,
        operation: IoOperation,
        offset: u64,
        length: usize,
        payload: &[u8],
    ) -> DataFileResult<Bytes> {
        let length_u64 = u64::try_from(length).map_err(|_| DataFileError::LengthOverflow { operation, length })?;
        check_range(operation, offset, length_u64, self.state.capacity)?;
        let io_error = |source| DataFileError::Io {
            operation,
            path: self.state.data_path.clone(),
            source,
        };
        let mut read_bytes = Vec::new();
        if operation == IoOperation::Read
            && length > uring::MAX_IO_CHUNK_BYTES - offset as usize % uring::DIRECT_IO_ALIGNMENT_BYTES
        {
            read_bytes
                .try_reserve_exact(length)
                .map_err(|source| DataFileError::Allocation { length, source })?;
        }
        let mut completed_bytes = 0;
        while completed_bytes < length {
            let chunk_offset = offset + completed_bytes as u64;
            let chunk_length = (length - completed_bytes)
                .min(uring::MAX_IO_CHUNK_BYTES - chunk_offset as usize % uring::DIRECT_IO_ALIGNMENT_BYTES);
            let chunk_payload = if operation == IoOperation::Write {
                &payload[completed_bytes..completed_bytes + chunk_length]
            } else {
                &[]
            };
            let bytes = self
                .state
                .queue
                .execute(operation, chunk_offset, chunk_length, chunk_payload)
                .await
                .map_err(io_error)?;
            if operation == IoOperation::Read {
                if chunk_length == length {
                    return Ok(bytes);
                }
                read_bytes.extend_from_slice(&bytes);
            }
            completed_bytes += chunk_length;
        }
        Ok(Bytes::from(read_bytes))
    }
}

fn open_file_state(directory: PathBuf, capacity: u64) -> DataFileResult<DataFileState> {
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
    // Construct the ring before resizing, so an unavailable io_uring does not resize an existing cache.
    let resize_file = file
        .try_clone()
        .map_err(|source| error(IoOperation::OpenDataFile, source))?;
    let queue =
        uring::IoQueueHandle::new(file, lock_file).map_err(|source| error(IoOperation::OpenDataFile, source))?;
    resize_file
        .set_len(capacity)
        .map_err(|source| error(IoOperation::ResizeDataFile, source))?;
    Ok(DataFileState {
        queue,
        data_path,
        capacity,
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
        || stat.stx_dio_mem_align == 0
        || stat.stx_dio_offset_align == 0
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

fn check_range(operation: IoOperation, offset: u64, length: u64, capacity: u64) -> DataFileResult<()> {
    if offset.checked_add(length).is_none_or(|end| end > capacity) {
        return Err(DataFileError::OutOfBounds {
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
    match result {
        Ok(_) => {
            span.record("outcome", "success");
        }
        Err(error) => {
            span.record("outcome", "error");
            span.record("error_kind", error.kind().as_str());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataFileErrorKind;
    use tempfile::tempdir;

    const CAPACITY: u64 = 4 * uring::MAX_IO_CHUNK_BYTES as u64;

    #[test]
    fn public_io_types_are_send_sync_static() {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}
        assert_send_sync_static::<DataFile>();
        assert_send_sync_static::<DataFileError>();
    }

    #[tokio::test]
    async fn reads_exact_unaligned_ranges_and_preserves_neighbors() {
        let temp = tempdir().unwrap();
        let directory = temp.path().join("cache/inner");
        let file = DataFile::open(&directory, CAPACITY, IoMetrics::noop()).await.unwrap();
        let original = Bytes::from(vec![0x55; CAPACITY as usize]);
        file.write_at(0, &original).await.unwrap();
        let payload = Bytes::from_static(b"unaligned positional payload");
        file.write_at(3, &payload).await.unwrap();
        assert_eq!(file.read_at(13, 10).await.unwrap(), Bytes::from_static(b"positional"));
        assert_eq!(file.read_at(0, 3).await.unwrap(), original.slice(..3));
        assert_eq!(
            file.read_at(3 + payload.len() as u64, 16).await.unwrap(),
            original.slice(..16)
        );
        // Exercise a boundary crossing and the final physical page.
        let payload = Bytes::from(vec![0x99; uring::MAX_IO_CHUNK_BYTES + 17]);
        file.write_at(4093, &payload).await.unwrap();
        assert_eq!(file.read_at(4093, payload.len()).await.unwrap(), payload);
        file.write_at(CAPACITY - 3, &Bytes::from_static(b"end")).await.unwrap();
        assert_eq!(file.read_at(CAPACITY - 3, 3).await.unwrap(), Bytes::from_static(b"end"));
        assert_eq!(
            std::fs::metadata(directory.join(DATA_FILE_NAME)).unwrap().len(),
            CAPACITY
        );
    }

    #[tokio::test]
    async fn rejects_out_of_bounds_and_accepts_empty_ranges() {
        let temp = tempdir().unwrap();
        let file = DataFile::open(temp.path(), CAPACITY, IoMetrics::noop()).await.unwrap();
        assert_eq!(
            file.read_at(CAPACITY - 1, 2).await.unwrap_err().kind(),
            DataFileErrorKind::OutOfBounds
        );
        assert_eq!(
            file.write_at(u64::MAX, &Bytes::from_static(b"ab"))
                .await
                .unwrap_err()
                .kind(),
            DataFileErrorKind::OutOfBounds
        );
        assert!(file.read_at(CAPACITY, 0).await.unwrap().is_empty());
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
        clone.write_at(17, &Bytes::from_static(b"persistent")).await.unwrap();
        drop(clone);
        let file = DataFile::open(temp.path(), CAPACITY / 2, IoMetrics::noop())
            .await
            .unwrap();
        assert_eq!(file.capacity(), CAPACITY / 2);
        assert_eq!(file.read_at(17, 10).await.unwrap(), Bytes::from_static(b"persistent"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_mixed_io_with_caller_serialized_same_page_rmw() {
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
                    let value = Bytes::from(vec![(i % 251) as u8; 31]);
                    file.write_at(i * 33, &value).await.unwrap();
                    assert_eq!(file.read_at(i * 33, value.len()).await.unwrap(), value);
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
            assert_eq!(file.read_at(i * 33, 33).await.unwrap(), {
                let mut value = vec![(i % 251) as u8; 31];
                value.extend_from_slice(&[0, 0]);
                Bytes::from(value)
            });
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
        file.write_at(0, &Bytes::from(vec![0x55; uring::MAX_IO_CHUNK_BYTES]))
            .await
            .unwrap();
        assert_eq!(
            file.read_at(0, uring::MAX_IO_CHUNK_BYTES).await.unwrap(),
            Bytes::from(vec![0x55; uring::MAX_IO_CHUNK_BYTES])
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
