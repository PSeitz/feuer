//! Linux direct-I/O file access and a disk cache for object bytes.
//!
//! The fixed-capacity O_DIRECT file supports up to 64 active reads and 8 active writes through io_uring.
//! The experimental cache reserves whole chunks, stores entry metadata separately, and verifies payload checksums.
//! Opening recovers entries from metadata before making the cache available for reads and writes.

#[cfg(not(target_os = "linux"))]
compile_error!("feuer-storage requires Linux with io_uring and O_DIRECT support");

#[cfg(target_os = "linux")]
mod allocation;
#[cfg(target_os = "linux")]
mod disk_cache;
mod disk_metrics;
mod error;
#[cfg(target_os = "linux")]
mod file;
#[cfg(all(target_os = "linux", feature = "io-bench"))]
pub mod io_bench;
mod metrics;
#[cfg(test)]
#[path = "../../test_support/metrics.rs"]
mod test_metrics;
#[cfg(target_os = "linux")]
mod uring;

#[cfg(target_os = "linux")]
pub use disk_cache::{DiskCache, DiskCacheError, DiskWriter};
pub use disk_metrics::DiskMetrics;
pub use error::{DataFileError, DataFileErrorKind, DataFileResult, IoOperation};
#[cfg(target_os = "linux")]
pub use file::DataFile;
pub use metrics::IoMetrics;
