//! Private range-storage foundations for Feuer.
//!
//! The current boundary is one exclusively owned, fixed-capacity data file with
//! checked request-sized buffered positional I/O. Direct I/O and the range disk
//! engine will be built above this foundation.

mod error;
mod file;
mod metrics;

pub use error::{Error, ErrorKind, IoOperation, Result};
pub use file::DataFile;
pub use metrics::IoMetrics;
