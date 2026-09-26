//! Private Linux range-storage foundations for Feuer.
//!
//! One exclusively owned, fixed-capacity O_DIRECT file with a bounded io_uring
//! driver. Range allocation, integrity validation, and recovery are separate layers.

#[cfg(not(target_os = "linux"))]
compile_error!("feuer-storage requires Linux with io_uring and O_DIRECT support");

// Experimental allocator, not yet connected to the disk range engine.
#[cfg(all(test, target_os = "linux"))]
mod allocation;
mod error;
#[cfg(target_os = "linux")]
mod file;
mod metrics;
#[cfg(target_os = "linux")]
mod uring;

pub use error::{DataFileError, DataFileErrorKind, DataFileResult, IoOperation};
#[cfg(target_os = "linux")]
pub use file::DataFile;
pub use metrics::IoMetrics;
