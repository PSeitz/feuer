use std::{
    fmt,
    fs::{File, OpenOptions, create_dir_all},
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use bytes::Bytes;
use fs4::fs_std::FileExt as LockFileExt;
use tokio::runtime::Handle;
use tracing::{Instrument, Span, field};

use crate::{Error, IoMetrics, IoOperation, Result};

const DATA_FILE_NAME: &str = "data";
const LOCK_FILE_NAME: &str = ".feuer.lock";

struct Inner {
    file: File,
    _lock_file: File,
    data_path: PathBuf,
    capacity: u64,
}

/// One exclusively owned, fixed-capacity raw payload file.
///
/// Reads and writes are positional and may use arbitrary non-zero or zero
/// lengths and offsets as long as the complete operation fits in `capacity`.
/// The methods run blocking filesystem calls on the Tokio blocking pool. The
/// data file has no embedded headers; persistent metadata belongs in Feuer's
/// journal and checkpoints.
#[derive(Clone)]
pub struct DataFile {
    inner: Arc<Inner>,
    runtime: Handle,
    metrics: Arc<IoMetrics>,
}

impl fmt::Debug for DataFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataFile")
            .field("capacity", &self.inner.capacity)
            .finish_non_exhaustive()
    }
}

impl DataFile {
    /// Opens the one raw data file and exclusively locks its cache directory.
    ///
    /// The directory is created if needed. The data file is set to exactly
    /// `capacity` bytes, which may create a sparse file depending on the
    /// filesystem. A second concurrent open of the same directory fails with
    /// [`crate::ErrorKind::AlreadyOpen`].
    pub async fn open(directory: impl AsRef<Path>, capacity: u64, metrics: Arc<IoMetrics>) -> Result<Self> {
        let directory = directory.as_ref().to_path_buf();
        let runtime = Handle::try_current().map_err(|_| Error::RuntimeUnavailable)?;
        let started = Instant::now();
        let span = tracing::info_span!(
            target: "feuer::storage::io",
            "feuer.storage.data_file.open",
            capacity,
            outcome = field::Empty,
            error_kind = field::Empty,
            duration_seconds = field::Empty,
        );

        let joined = runtime
            .spawn_blocking(move || open_inner(directory, capacity))
            .instrument(span.clone())
            .await;
        let result = joined
            .map_err(|source| Error::Task {
                operation: IoOperation::OpenDataFile,
                source: Box::new(source),
            })
            .and_then(|result| result);
        record_span_outcome(&span, started, &result);

        Ok(Self {
            inner: Arc::new(result?),
            runtime,
            metrics,
        })
    }

    /// Returns the fixed physical capacity in bytes.
    pub fn capacity(&self) -> u64 {
        self.inner.capacity
    }

    /// Positionally reads exactly `length` bytes starting at `offset`.
    ///
    /// The returned [`Bytes`] owns one compact request-sized allocation and
    /// does not retain the file, a physical-space guard, or a larger extent.
    pub async fn read_at(&self, offset: u64, length: usize) -> Result<Bytes> {
        let observed_bytes = u64::try_from(length).unwrap_or(u64::MAX);
        self.execute(IoOperation::Read, Some(offset), observed_bytes, move |inner| {
            let length_u64 = checked_length(IoOperation::Read, length)?;
            check_range(IoOperation::Read, offset, length_u64, inner.capacity)?;

            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(length)
                .map_err(|source| Error::Allocation { length, source })?;
            bytes.resize(length, 0);
            read_exact_at(&inner.file, &mut bytes, offset).map_err(|source| Error::Io {
                operation: IoOperation::Read,
                path: inner.data_path.clone(),
                source,
            })?;
            Ok(Bytes::from(bytes))
        })
        .await
    }

    /// Positionally writes the complete byte string starting at `offset`.
    ///
    /// Cloning the input for the blocking task shares its allocation; this
    /// method does not copy the payload and does not consume the caller's
    /// in-memory owner.
    pub async fn write_at(&self, offset: u64, bytes: &Bytes) -> Result<()> {
        let bytes = bytes.clone();
        let observed_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        self.execute(IoOperation::Write, Some(offset), observed_bytes, move |inner| {
            let length = checked_length(IoOperation::Write, bytes.len())?;
            check_range(IoOperation::Write, offset, length, inner.capacity)?;
            write_all_at(&inner.file, &bytes, offset).map_err(|source| Error::Io {
                operation: IoOperation::Write,
                path: inner.data_path.clone(),
                source,
            })
        })
        .await
    }

    /// Synchronizes file data without requiring all file metadata to persist.
    pub async fn sync_data(&self) -> Result<()> {
        self.execute(IoOperation::SyncData, None, 0, |inner| {
            inner.file.sync_data().map_err(|source| Error::Io {
                operation: IoOperation::SyncData,
                path: inner.data_path.clone(),
                source,
            })
        })
        .await
    }

    /// Synchronizes file data and metadata.
    pub async fn sync_all(&self) -> Result<()> {
        self.execute(IoOperation::SyncAll, None, 0, |inner| {
            inner.file.sync_all().map_err(|source| Error::Io {
                operation: IoOperation::SyncAll,
                path: inner.data_path.clone(),
                source,
            })
        })
        .await
    }

    async fn execute<T, F>(&self, operation: IoOperation, offset: Option<u64>, bytes: u64, job: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Arc<Inner>) -> Result<T> + Send + 'static,
    {
        let started = Instant::now();
        let span = tracing::trace_span!(
            target: "feuer::storage::io",
            "feuer.storage.data_file.io",
            operation = operation.as_str(),
            offset = field::Empty,
            bytes,
            outcome = field::Empty,
            error_kind = field::Empty,
            duration_seconds = field::Empty,
        );
        if let Some(offset) = offset {
            span.record("offset", offset);
        }

        let inner = self.inner.clone();
        let joined = self
            .runtime
            .spawn_blocking(move || job(inner))
            .instrument(span.clone())
            .await;
        let result = joined
            .map_err(|source| Error::Task {
                operation,
                source: Box::new(source),
            })
            .and_then(|result| result);
        let elapsed = started.elapsed();
        self.metrics.record(operation, bytes, elapsed, result.is_ok());
        record_span_outcome_with_elapsed(&span, elapsed, &result);
        result
    }
}

fn open_inner(directory: PathBuf, capacity: u64) -> Result<Inner> {
    if capacity == 0 {
        return Err(Error::InvalidCapacity);
    }

    create_dir_all(&directory).map_err(|source| Error::Io {
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
        .map_err(|source| Error::Io {
            operation: IoOperation::OpenLockFile,
            path: lock_path.clone(),
            source,
        })?;
    let locked = LockFileExt::try_lock_exclusive(&lock_file).map_err(|source| Error::Io {
        operation: IoOperation::LockDirectory,
        path: lock_path,
        source,
    })?;
    if !locked {
        return Err(Error::AlreadyOpen { directory });
    }

    let data_path = directory.join(DATA_FILE_NAME);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&data_path)
        .map_err(|source| Error::Io {
            operation: IoOperation::OpenDataFile,
            path: data_path.clone(),
            source,
        })?;
    let metadata = file.metadata().map_err(|source| Error::Io {
        operation: IoOperation::InspectDataFile,
        path: data_path.clone(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(Error::InvalidDataFile { path: data_path });
    }
    file.set_len(capacity).map_err(|source| Error::Io {
        operation: IoOperation::ResizeDataFile,
        path: data_path.clone(),
        source,
    })?;

    Ok(Inner {
        file,
        _lock_file: lock_file,
        data_path,
        capacity,
    })
}

fn checked_length(operation: IoOperation, length: usize) -> Result<u64> {
    u64::try_from(length).map_err(|_| Error::LengthOverflow { operation, length })
}

fn check_range(operation: IoOperation, offset: u64, length: u64, capacity: u64) -> Result<()> {
    if offset.checked_add(length).is_none_or(|end| end > capacity) {
        return Err(Error::OutOfBounds {
            operation,
            offset,
            length,
            capacity,
        });
    }
    Ok(())
}

fn record_span_outcome<T>(span: &Span, started: Instant, result: &Result<T>) {
    record_span_outcome_with_elapsed(span, started.elapsed(), result);
}

fn record_span_outcome_with_elapsed<T>(span: &Span, elapsed: std::time::Duration, result: &Result<T>) {
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

fn read_exact_at(file: &File, mut bytes: &mut [u8], mut offset: u64) -> io::Result<()> {
    while !bytes.is_empty() {
        match read_at(file, bytes, offset) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "positional read completed before filling the request buffer",
                ));
            }
            Ok(read) => {
                offset += read as u64;
                bytes = &mut bytes[read..];
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn write_all_at(file: &File, mut bytes: &[u8], mut offset: u64) -> io::Result<()> {
    while !bytes.is_empty() {
        match write_at(file, bytes, offset) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "positional write completed without writing bytes",
                ));
            }
            Ok(written) => {
                offset += written as u64;
                bytes = &bytes[written..];
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn read_at(file: &File, bytes: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;

    file.read_at(bytes, offset)
}

#[cfg(windows)]
fn read_at(file: &File, bytes: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;

    file.seek_read(bytes, offset)
}

#[cfg(not(any(unix, windows)))]
fn read_at(_file: &File, _bytes: &mut [u8], _offset: u64) -> io::Result<usize> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "positional file reads are not supported on this target",
    ))
}

#[cfg(unix)]
fn write_at(file: &File, bytes: &[u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;

    file.write_at(bytes, offset)
}

#[cfg(windows)]
fn write_at(file: &File, bytes: &[u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;

    file.seek_write(bytes, offset)
}

#[cfg(not(any(unix, windows)))]
fn write_at(_file: &File, _bytes: &[u8], _offset: u64) -> io::Result<usize> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "positional file writes are not supported on this target",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorKind;
    use tempfile::tempdir;

    #[test]
    fn public_io_types_are_send_sync_static() {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}

        assert_send_sync_static::<DataFile>();
        assert_send_sync_static::<Error>();
    }

    #[tokio::test]
    async fn reads_only_the_exact_arbitrary_requested_range() {
        let temp = tempdir().unwrap();
        let directory = temp.path().join("cache/inner");
        let file = DataFile::open(&directory, 64 * 1024, IoMetrics::noop()).await.unwrap();
        let payload = Bytes::from_static(b"unaligned positional payload");

        file.write_at(3, &payload).await.unwrap();
        let result = file.read_at(13, 10).await.unwrap();
        file.sync_data().await.unwrap();
        file.sync_all().await.unwrap();

        assert_eq!(result, Bytes::from_static(b"positional"));
        assert_eq!(result.len(), 10);
        assert_eq!(file.capacity(), 64 * 1024);
        assert_eq!(
            std::fs::metadata(directory.join(DATA_FILE_NAME)).unwrap().len(),
            64 * 1024
        );
    }

    #[tokio::test]
    async fn rejects_every_range_that_is_not_fully_inside_capacity() {
        let temp = tempdir().unwrap();
        let file = DataFile::open(temp.path(), 16, IoMetrics::noop()).await.unwrap();

        let read_error = file.read_at(15, 2).await.unwrap_err();
        assert_eq!(read_error.kind(), ErrorKind::OutOfBounds);
        assert!(matches!(
            read_error,
            Error::OutOfBounds {
                operation: IoOperation::Read,
                offset: 15,
                length: 2,
                capacity: 16,
            }
        ));

        let write_error = file.write_at(u64::MAX, &Bytes::from_static(b"ab")).await.unwrap_err();
        assert_eq!(write_error.kind(), ErrorKind::OutOfBounds);
        assert!(matches!(
            write_error,
            Error::OutOfBounds {
                operation: IoOperation::Write,
                offset: u64::MAX,
                length: 2,
                capacity: 16,
            }
        ));

        assert!(file.read_at(16, 0).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn fails_a_short_completion_instead_of_returning_uncertain_bytes() {
        let temp = tempdir().unwrap();
        let file = DataFile::open(temp.path(), 16, IoMetrics::noop()).await.unwrap();
        let data_path = temp.path().join(DATA_FILE_NAME);
        OpenOptions::new()
            .write(true)
            .open(data_path)
            .unwrap()
            .set_len(4)
            .unwrap();

        let error = file.read_at(0, 8).await.unwrap_err();

        assert_eq!(error.kind(), ErrorKind::Io);
        match error {
            Error::Io { source, .. } => assert_eq!(source.kind(), io::ErrorKind::UnexpectedEof),
            error => panic!("unexpected error: {error:?}"),
        }
    }

    #[tokio::test]
    async fn holds_exclusive_ownership_until_the_last_clone_is_dropped() {
        let temp = tempdir().unwrap();
        let file = DataFile::open(temp.path(), 16, IoMetrics::noop()).await.unwrap();
        let clone = file.clone();

        let error = DataFile::open(temp.path(), 16, IoMetrics::noop()).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::AlreadyOpen);

        drop(file);
        let error = DataFile::open(temp.path(), 16, IoMetrics::noop()).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::AlreadyOpen);

        drop(clone);
        DataFile::open(temp.path(), 16, IoMetrics::noop()).await.unwrap();
    }

    #[tokio::test]
    async fn reopening_applies_the_new_explicit_fixed_capacity() {
        let temp = tempdir().unwrap();
        let file = DataFile::open(temp.path(), 16, IoMetrics::noop()).await.unwrap();
        drop(file);

        let file = DataFile::open(temp.path(), 31, IoMetrics::noop()).await.unwrap();

        assert_eq!(file.capacity(), 31);
        assert_eq!(std::fs::metadata(temp.path().join(DATA_FILE_NAME)).unwrap().len(), 31);
    }

    #[test]
    fn reports_a_missing_runtime_as_a_structured_error() {
        use std::{
            future::Future,
            task::{Context, Poll, Waker},
        };

        let temp = tempdir().unwrap();
        let mut open = Box::pin(DataFile::open(temp.path(), 16, IoMetrics::noop()));
        let mut context = Context::from_waker(Waker::noop());

        let Poll::Ready(result) = open.as_mut().poll(&mut context) else {
            panic!("open unexpectedly remained pending without a runtime");
        };
        assert_eq!(result.unwrap_err().kind(), ErrorKind::Task);
    }

    #[tokio::test]
    async fn rejects_zero_capacity_and_omits_paths_from_debug_output() {
        let temp = tempdir().unwrap();
        let error = DataFile::open(temp.path(), 0, IoMetrics::noop()).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidConfiguration);

        let file = DataFile::open(temp.path(), 16, IoMetrics::noop()).await.unwrap();
        let output = format!("{file:?}");
        assert_eq!(output, "DataFile { capacity: 16, .. }");
        assert!(!output.contains(&temp.path().display().to_string()));
    }
}
