use std::{collections::TryReserveError, fmt, io, path::PathBuf};

/// One bounded data-file operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IoOperation {
    /// Create the cache directory.
    CreateDirectory,
    /// Open the cache-directory lock file.
    OpenLockFile,
    /// Acquire exclusive ownership of the cache directory.
    LockDirectory,
    /// Open the raw payload data file.
    OpenDataFile,
    /// Inspect the raw payload data file.
    InspectDataFile,
    /// Set the raw payload data file to its configured capacity.
    ResizeDataFile,
    /// Positionally read payload bytes.
    Read,
    /// Positionally write payload bytes.
    Write,
    /// Synchronize payload data without requiring all file metadata.
    SyncData,
    /// Synchronize payload data and file metadata.
    SyncAll,
}

impl IoOperation {
    /// Returns the stable bounded label used by traces and metrics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CreateDirectory => "create_directory",
            Self::OpenLockFile => "open_lock_file",
            Self::LockDirectory => "lock_directory",
            Self::OpenDataFile => "open_data_file",
            Self::InspectDataFile => "inspect_data_file",
            Self::ResizeDataFile => "resize_data_file",
            Self::Read => "read",
            Self::Write => "write",
            Self::SyncData => "sync_data",
            Self::SyncAll => "sync_all",
        }
    }
}

impl fmt::Display for IoOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A stable category for a range-storage error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// The fixed-file configuration is invalid.
    InvalidConfiguration,
    /// Another cache instance owns the cache directory.
    AlreadyOpen,
    /// A positional operation exceeds the configured file capacity.
    OutOfBounds,
    /// A request-sized read buffer could not be allocated.
    Allocation,
    /// An operating-system file operation failed.
    Io,
    /// The blocking I/O task failed before returning its result.
    Task,
}

impl ErrorKind {
    /// Returns the stable bounded label used by traces and metrics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "invalid_configuration",
            Self::AlreadyOpen => "already_open",
            Self::OutOfBounds => "out_of_bounds",
            Self::Allocation => "allocation",
            Self::Io => "io",
            Self::Task => "task",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An error from the fixed-capacity positional data file.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A zero-capacity data file was requested.
    #[error("data-file capacity must be greater than zero")]
    InvalidCapacity,
    /// Another process or cache instance holds the directory lock.
    #[error("cache directory is already open: {}", .directory.display())]
    AlreadyOpen {
        /// The exclusively owned cache directory.
        directory: PathBuf,
    },
    /// The configured data path is not a regular file.
    #[error("data-file path is not a regular file: {}", .path.display())]
    InvalidDataFile {
        /// The rejected data-file path.
        path: PathBuf,
    },
    /// A positional request does not fit in the configured capacity.
    #[error("{operation} range at offset {offset} with length {length} exceeds data-file capacity {capacity}")]
    OutOfBounds {
        /// The rejected operation.
        operation: IoOperation,
        /// The requested file offset.
        offset: u64,
        /// The requested byte length.
        length: u64,
        /// The configured data-file capacity.
        capacity: u64,
    },
    /// A platform buffer length cannot be represented as a file length.
    #[error("{operation} length {length} cannot be represented as a u64")]
    LengthOverflow {
        /// The rejected operation.
        operation: IoOperation,
        /// The platform buffer length.
        length: usize,
    },
    /// Allocation of an exact read result failed.
    #[error("failed to allocate {length} bytes for positional read: {source}")]
    Allocation {
        /// The requested result length.
        length: usize,
        /// The allocation failure.
        #[source]
        source: TryReserveError,
    },
    /// An operating-system file operation failed.
    #[error("{operation} failed for {}: {source}", .path.display())]
    Io {
        /// The failed operation.
        operation: IoOperation,
        /// The path on which the operation was attempted.
        path: PathBuf,
        /// The operating-system error.
        #[source]
        source: io::Error,
    },
    /// No compatible Tokio runtime was active when the file was opened.
    #[error("opening a data file requires an active Tokio runtime")]
    RuntimeUnavailable,
    /// A blocking I/O task failed before returning its operation result.
    #[error("blocking {operation} task failed: {source}")]
    Task {
        /// The operation run by the failed task.
        operation: IoOperation,
        /// The runtime task failure.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl Error {
    /// Returns the stable category of this error.
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::InvalidCapacity | Self::InvalidDataFile { .. } => ErrorKind::InvalidConfiguration,
            Self::AlreadyOpen { .. } => ErrorKind::AlreadyOpen,
            Self::OutOfBounds { .. } | Self::LengthOverflow { .. } => ErrorKind::OutOfBounds,
            Self::Allocation { .. } => ErrorKind::Allocation,
            Self::Io { .. } => ErrorKind::Io,
            Self::RuntimeUnavailable | Self::Task { .. } => ErrorKind::Task,
        }
    }

    /// Returns the operation that failed.
    pub const fn operation(&self) -> IoOperation {
        match self {
            Self::InvalidCapacity => IoOperation::OpenDataFile,
            Self::AlreadyOpen { .. } => IoOperation::LockDirectory,
            Self::InvalidDataFile { .. } => IoOperation::InspectDataFile,
            Self::RuntimeUnavailable => IoOperation::OpenDataFile,
            Self::OutOfBounds { operation, .. }
            | Self::LengthOverflow { operation, .. }
            | Self::Io { operation, .. }
            | Self::Task { operation, .. } => *operation,
            Self::Allocation { .. } => IoOperation::Read,
        }
    }
}

/// A result returned by range-storage operations.
pub type Result<T> = std::result::Result<T, Error>;
