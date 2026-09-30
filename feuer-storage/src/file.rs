use std::{
    fmt,
    fs::{File, OpenOptions, create_dir_all},
    future::Future,
    io,
    ops::Range,
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

use crate::{
    DataFileError, DataFileResult, IoMetrics, IoOperation,
    allocation::{DiskRegion, DiskRegionReadGuard},
    uring,
};

const DATA_FILE_NAME: &str = "data";
const LOCK_FILE_NAME: &str = ".feuer.lock";

/// Queues, path, and capacity state shared by cloned data-file handles.
struct DataFileState {
    read_queue: uring::IoQueueHandle,
    write_queue: uring::IoQueueHandle,
    data_path: PathBuf,
    capacity: u64,
}

/// One exclusively owned, fixed-capacity Linux direct-I/O payload file.
///
/// Reads and writes have separate io_uring rings and worker threads, with up to
/// 64 active reads and 8 active writes plus equally sized bounded waiting channels.
/// Queues schedule in arrival order without checking for conflicts.
/// Submission order does not guarantee completion order.
/// Each I/O request transfers at most 1 MiB. Request limits do not bound full read-result allocations.
///
/// # Caller-owned concurrency and cancellation
///
/// Callers must prevent overlapping I/O when either operation is a write. This
/// applies to physical byte ranges rounded outward to 4 KiB boundaries, not just
/// the requested bytes of an unaligned read.
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
/// Capacity must be a positive multiple of 4096. Opening fails
/// if io_uring or verified O_DIRECT alignment is unavailable; there is no fallback.
/// Dropping the last handle drains submitted I/O and joins both queue threads,
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
                .spawn_blocking(move || open_file_state(directory, capacity, buffer_pool))
                .await
                .map_err(|source| DataFileError::Task {
                    operation: IoOperation::OpenDataFile,
                    source: Box::new(source),
                })??;
            // Exercise an actual aligned direct read before claiming open succeeded.
            state
                .read_queue
                .read(0, uring::DIRECT_IO_ALIGNMENT_BYTES)
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
    /// of overhead. The result may retain alignment padding in its backing allocation.
    /// Callers must prevent writes to the aligned byte range while this read
    /// depends on its contents; see [`DataFile`]'s concurrency contract.
    pub async fn read_at(&self, offset: u64, length: usize) -> DataFileResult<Bytes> {
        self.measure_io(IoOperation::Read, offset, length, async {
            check_file_bounds(IoOperation::Read, offset, length as u64, self.state.capacity)?;
            if length == 0 {
                return Ok(Bytes::new());
            }
            let alignment = uring::DIRECT_IO_ALIGNMENT_BYTES as u64;
            let leading_padding_bytes = offset % alignment;
            let aligned_range = offset - leading_padding_bytes..(offset + length as u64).next_multiple_of(alignment);
            let (bytes, _) = self.read_aligned_ranges(&[aligned_range], Vec::new()).await?;
            Ok(bytes.slice(leading_padding_bytes as usize..leading_padding_bytes as usize + length))
        })
        .await
    }

    /// Reads payload regions into one buffer, omitting metadata gaps and final padding.
    pub(crate) async fn read_regions(
        &self,
        regions: Vec<DiskRegionReadGuard>,
        length: usize,
    ) -> DataFileResult<(Bytes, usize)> {
        let ranges: Vec<_> = regions.iter().map(DiskRegionReadGuard::range).collect();
        let offset = ranges.first().map_or(0, |range| range.start);
        self.measure_io(IoOperation::Read, offset, length, async {
            let (bytes, capacity) = self.read_aligned_ranges(&ranges, regions).await?;
            assert_eq!(bytes.len(), length.next_multiple_of(uring::DIRECT_IO_ALIGNMENT_BYTES));
            Ok((bytes.slice(..length), capacity))
        })
        .await
    }

    /// Recovery issues only one metadata read at a time and retries when the read channel is full.
    /// Its guard prevents concurrent writes from overwriting this page.
    pub(crate) async fn read_recovery_page(&self, region: &DiskRegion, address: u64) -> DataFileResult<Bytes> {
        let length = uring::DIRECT_IO_ALIGNMENT_BYTES;
        let page = region.slice(address..address + length as u64);
        self.measure_io(IoOperation::Read, address, length, async {
            loop {
                let result = self
                    .state
                    .read_queue
                    .try_read_recovery_page(page.read_guard())
                    .await
                    .map_err(|source| DataFileError::Io {
                        operation: IoOperation::Read,
                        path: self.state.data_path.clone(),
                        source,
                    })?;
                if let Some(bytes) = result {
                    return Ok(bytes);
                }
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
    }

    /// Writes complete 4096-byte-aligned blocks.
    ///
    /// Retains aligned byte slices directly; copies unaligned inputs into bounded
    /// aligned buffers. Callers must protect the full aligned byte range from
    /// conflicting access through completion.
    /// Dropping this future may leave a partial write and does not stop submitted
    /// writes: retain the disk region until they complete, as described in
    /// [`DataFile`]'s cancellation contract. Publish a cached byte range only after success.
    ///
    /// # Panics
    ///
    /// Panics if the offset or byte count is not a multiple of 4096.
    pub async fn write_at(&self, offset: u64, bytes: &Bytes) -> DataFileResult<()> {
        assert!(offset.is_multiple_of(uring::DIRECT_IO_ALIGNMENT_BYTES as u64));
        assert!(bytes.len().is_multiple_of(uring::DIRECT_IO_ALIGNMENT_BYTES));
        self.measure_io(IoOperation::Write, offset, bytes.len(), async {
            check_file_bounds(IoOperation::Write, offset, bytes.len() as u64, self.state.capacity)?;
            for start in (0..bytes.len()).step_by(uring::MAX_IO_CHUNK_BYTES) {
                let end = (start + uring::MAX_IO_CHUNK_BYTES).min(bytes.len());
                self.state
                    .write_queue
                    .write_parts(
                        offset + start as u64,
                        end - start,
                        &[(0, bytes.slice(start..end))],
                        None,
                    )
                    .await
                    .map_err(|source| DataFileError::Io {
                        operation: IoOperation::Write,
                        path: self.state.data_path.clone(),
                        source,
                    })?;
            }
            Ok(())
        })
        .await
    }

    /// Writes one complete chunk from parts at aligned offsets starting at zero, without
    /// an intermediate chunk buffer. Gaps are zero-filled. The queue retains the region until completion.
    pub(crate) async fn write_parts(&self, region: DiskRegion, parts: &[(usize, Bytes)]) -> DataFileResult<()> {
        let range = region.range();
        let offset = range.start;
        let length = (range.end - range.start) as usize;
        self.measure_io(IoOperation::Write, offset, length, async {
            check_file_bounds(IoOperation::Write, offset, length as u64, self.state.capacity)?;
            self.state
                .write_queue
                .write_parts(offset, length, parts, Some(region))
                .await
                .map_err(|source| DataFileError::Io {
                    operation: IoOperation::Write,
                    path: self.state.data_path.clone(),
                    source,
                })
        })
        .await
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
        self.metrics.record(operation, observed_bytes, elapsed, result.is_ok());
        record_span_outcome(&span, elapsed, &result);
        result
    }

    async fn read_aligned_ranges(
        &self,
        ranges: &[Range<u64>],
        read_guards: Vec<DiskRegionReadGuard>,
    ) -> DataFileResult<(Bytes, usize)> {
        let operation = IoOperation::Read;
        let mut buffer_length = 0usize;
        for range in ranges {
            check_file_bounds(operation, range.start, range.end - range.start, self.state.capacity)?;
            buffer_length =
                buffer_length
                    .checked_add((range.end - range.start) as usize)
                    .ok_or(DataFileError::LengthOverflow {
                        operation,
                        length: buffer_length,
                    })?;
        }
        let io_error = |source| DataFileError::Io {
            operation,
            path: self.state.data_path.clone(),
            source,
        };
        let mut buffer = self
            .state
            .read_queue
            .allocate_buffer(buffer_length, read_guards)
            .map_err(io_error)?;
        let mut destination_offset = 0;
        for range in ranges {
            for offset in (range.start..range.end).step_by(uring::MAX_IO_CHUNK_BYTES) {
                let read_length = (range.end - offset).min(uring::MAX_IO_CHUNK_BYTES as u64) as usize;
                buffer = self
                    .state
                    .read_queue
                    .read_into(offset, buffer, destination_offset..destination_offset + read_length)
                    .await
                    .map_err(io_error)?;
                destination_offset += read_length;
            }
        }
        let capacity = buffer.capacity();
        Ok((buffer.into_bytes(), capacity))
    }
}

fn open_file_state(directory: PathBuf, capacity: u64, buffer_pool: Arc<BufferPool>) -> DataFileResult<DataFileState> {
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
    let file = Arc::new(file);
    let lock_file = Arc::new(lock_file);
    let read_queue = uring::IoQueueHandle::new(file.clone(), lock_file.clone(), IoOperation::Read, buffer_pool.clone())
        .map_err(|source| error(IoOperation::OpenDataFile, source))?;
    let write_queue = uring::IoQueueHandle::new(file.clone(), lock_file, IoOperation::Write, buffer_pool)
        .map_err(|source| error(IoOperation::OpenDataFile, source))?;
    file.set_len(capacity)
        .map_err(|source| error(IoOperation::ResizeDataFile, source))?;
    Ok(DataFileState {
        read_queue,
        write_queue,
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

/// Checks that the requested bytes fit within file bounds, rejecting offset-plus-length overflow.
fn check_file_bounds(operation: IoOperation, offset: u64, length: u64, capacity: u64) -> DataFileResult<()> {
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
            (0..uring::MAX_IO_CHUNK_BYTES + 4096)
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
    async fn disk_reads_reuse_released_buffers_and_overwrite_old_contents() {
        let temp = tempdir().unwrap();
        let file = DataFile::open(temp.path(), CAPACITY, IoMetrics::noop()).await.unwrap();
        let length = 2 * uring::DIRECT_IO_ALIGNMENT_BYTES;
        file.write_at(0, &Bytes::from(vec![0x55; length])).await.unwrap();
        let bytes = file.read_at(0, length).await.unwrap();
        let address = bytes.as_ptr();
        let slice = bytes.slice(1..);
        drop(bytes);
        file.write_at(0, &Bytes::from(vec![0x99; length])).await.unwrap();
        let other = file.read_at(0, length).await.unwrap();
        assert_ne!(other.as_ptr(), address);
        assert_eq!(&slice[..], &vec![0x55; length - 1]);
        drop(slice);
        let reused = file.read_at(0, length).await.unwrap();
        assert_eq!(reused.as_ptr(), address);
        assert_eq!(&reused[..], &vec![0x99; length]);
        drop(file);
        assert_eq!(reused, other);
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
            file.write_at(u64::MAX - 4095, &Bytes::from(vec![0; 4096]))
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
