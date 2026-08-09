//! Feuer is a tiered cache for ranges of immutable objects.
//!
//! The current public boundary provides exact requested byte ranges, opaque
//! object identities, one soft-capacity memory tier, and per-call asynchronous
//! download callbacks. The memory tier uses bounded aged request evidence and
//! compaction; best-effort disk population and recovery remain in progress.

mod cache;
mod config;

pub use cache::{Cache, GetOrFetchError};
pub use config::{Config, ConfigError};
pub use feuer_types::{ByteRange, Download, DownloadError, InvalidRange, ObjectKey};
