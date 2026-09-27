//! Private Linux range-storage foundations for Feuer.
//!
//! A fixed-capacity O_DIRECT file with a bounded io_uring driver, plus an experimental
//! disk range cache with connected allocation, entry metadata writes and integrity-checked reads.
//! Restart recovery and public tier integration are not implemented.

#[cfg(not(target_os = "linux"))]
compile_error!("feuer-storage requires Linux with io_uring and O_DIRECT support");

#[cfg(target_os = "linux")]
mod allocation;
mod error;
#[cfg(target_os = "linux")]
mod file;
mod metrics;
#[cfg(target_os = "linux")]
mod range_cache;
#[cfg(target_os = "linux")]
mod uring;

pub use error::{DataFileError, DataFileErrorKind, DataFileResult, IoOperation};
#[cfg(target_os = "linux")]
pub use file::DataFile;
pub use metrics::IoMetrics;
#[cfg(target_os = "linux")]
pub use range_cache::{DiskRangeCache, DiskRangeCacheError};
