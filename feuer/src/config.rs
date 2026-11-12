use std::{
    num::{NonZeroU64, NonZeroUsize},
    path::{Path, PathBuf},
};

use thiserror::Error;

/// Resolved settings for an open cache, available through [`crate::Cache::config`].
/// Configure a cache through [`crate::CacheBuilder`].
///
/// `disk_capacity` includes the 1-MiB header chunk, shard metadata, and alignment overhead.
/// Nonzero capacity must be at least 2 MiB. Changing the file capacity invalidates disk entries.
/// Zero disables the disk tier.
/// When disk is disabled, `directory`, `file_name`, and `payload_version` are unused.
/// `memory_capacity` must be positive. It is a soft eviction target divided among the in-memory shards.
/// Oversized entries can make entry allocation charges exceed it. Applications using zero memory
/// capacity to disable caching should bypass cache construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheConfig {
    pub(super) directory: PathBuf,
    pub(super) file_name: String,
    pub(super) payload_version: u64,
    pub(super) disk_capacity: u64,
    pub(super) memory_capacity: u64,
    pub(super) memory_shard_count: usize,
    pub(super) access_count_half_life: NonZeroU64,
    pub(super) history_entry_limit: NonZeroUsize,
    pub(super) reclaim_sample_size: usize,
}

impl CacheConfig {
    /// Returns the application's payload format version.
    pub const fn payload_version(&self) -> u64 {
        self.payload_version
    }

    /// Returns the maximum candidates inspected per eviction decision.
    pub const fn reclaim_sample_size(&self) -> usize {
        self.reclaim_sample_size
    }

    /// Returns the access count half-life in requests across all keys of this cache.
    pub const fn access_count_half_life(&self) -> NonZeroU64 {
        self.access_count_half_life
    }

    /// Returns the entry-count cleanup threshold for the history shared by both cache tiers.
    pub const fn history_entry_limit(&self) -> NonZeroUsize {
        self.history_entry_limit
    }

    /// Returns the directory containing the cache's exclusively locked backing file.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Returns the backing filename within the cache directory.
    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    /// Returns the configured disk capacity in bytes. Zero disables the disk tier.
    pub const fn disk_capacity(&self) -> u64 {
        self.disk_capacity
    }

    /// Returns the number of memory cache shards.
    pub const fn memory_shard_count(&self) -> usize {
        self.memory_shard_count
    }

    /// Returns the soft memory eviction target for entry allocation charges in bytes.
    pub const fn memory_capacity(&self) -> u64 {
        self.memory_capacity
    }
}

/// An invalid Feuer configuration.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum CacheConfigError {
    /// The backing filename contains a path, NUL bytes, or the reserved `.feuer.` prefix.
    #[error("cache file name must be a single filename without NUL bytes or the reserved .feuer. prefix")]
    InvalidFileName,
    /// The configured memory eviction target must be positive.
    #[error("memory capacity must be greater than zero")]
    InvalidMemoryCapacity,
    /// The memory shard count must be positive.
    #[error("memory shard count must be greater than zero; check FEUER_MEMORY_SHARD_COUNT or the explicit setting")]
    InvalidMemoryShardCount,
    /// The eviction candidate limit must be a positive integer that fits in `usize`.
    #[error(
        "reclaim sample size must be a positive number fitting usize; check FEUER_RECLAIM_SAMPLE_SIZE (e.g. 1KiB) or the explicit setting"
    )]
    InvalidReclaimSampleSize,
    /// The access count half-life must be positive.
    #[error(
        "access count half-life must be greater than zero; check FEUER_ACCESS_COUNT_HALF_LIFE or the explicit setting"
    )]
    InvalidAccessCountHalfLife,
    /// The history entry limit must be a positive integer that fits in `usize`.
    #[error(
        "history entry limit must be a positive number fitting usize; check FEUER_HISTORY_ENTRY_LIMIT or the explicit setting"
    )]
    InvalidHistoryEntryLimit,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cache, CacheBuilder};
    use std::{
        env::{current_exe, var},
        process::Command,
    };

    fn builder() -> CacheBuilder<'static> {
        Cache::builder().memory_capacity(1)
    }

    fn check_environment_overrides(test: &str, name: &str, child: &str, cases: &[(Option<&str>, &str)]) {
        for &(setting, expected) in cases {
            let mut command = Command::new(current_exe().unwrap());
            command.args(["--exact", test]).env(child, expected).env_remove(name);
            if let Some(setting) = setting {
                command.env(name, setting);
            }
            let output = command.output().unwrap();
            assert!(output.status.success(), "{output:?}");
        }
    }

    #[test]
    fn memory_shard_count() {
        for (capacity, expected) in [(1, 1), (100 * 1024 * 1024 * 1024, 12)] {
            assert_eq!(
                builder()
                    .memory_capacity(capacity)
                    .resolve_config()
                    .unwrap()
                    .memory_shard_count(),
                expected
            );
            let cache = builder()
                .memory_capacity(capacity)
                .memory_shard_count(4)
                .open()
                .unwrap();
            assert_eq!(cache.config().memory_shard_count(), 4);
        }
        assert_eq!(
            builder().memory_shard_count(0).resolve_config(),
            Err(CacheConfigError::InvalidMemoryShardCount)
        );
    }

    #[test]
    fn memory_shard_count_environment_override() {
        if let Ok(expected) = var("FEUER_TEST_MEMORY_SHARD_COUNT_CHILD") {
            assert_eq!(
                builder().resolve_config().map(|config| config.memory_shard_count()),
                expected
                    .parse::<usize>()
                    .map_err(|_| CacheConfigError::InvalidMemoryShardCount)
            );
            assert_eq!(
                builder()
                    .memory_shard_count(4)
                    .resolve_config()
                    .unwrap()
                    .memory_shard_count(),
                4
            );
            return;
        }
        check_environment_overrides(
            "config::tests::memory_shard_count_environment_override",
            "FEUER_MEMORY_SHARD_COUNT",
            "FEUER_TEST_MEMORY_SHARD_COUNT_CHILD",
            &[
                (None, "1"),
                (Some("16"), "16"),
                (Some("0"), "error"),
                (Some("bad"), "error"),
                (Some("18446744073709551616"), "error"),
            ],
        );
    }

    #[test]
    fn rejects_invalid_file_names() {
        for name in [
            "",
            ".",
            "..",
            "../data",
            "cache/data",
            ".feuer.lock",
            ".feuer.images.lock",
            "data\0",
        ] {
            assert_eq!(
                builder().file_name(name).resolve_config(),
                Err(CacheConfigError::InvalidFileName)
            );
        }
    }

    #[test]
    fn reclaim_sample_size_environment_override() {
        if let Ok(expected) = var("FEUER_TEST_RECLAIM_SAMPLE_CHILD") {
            let result = builder().resolve_config().map(|config| config.reclaim_sample_size());
            let expected = expected
                .parse::<usize>()
                .map_err(|_| CacheConfigError::InvalidReclaimSampleSize);
            assert_eq!(result, expected);
            assert_eq!(
                builder()
                    .reclaim_sample_size(128)
                    .resolve_config()
                    .unwrap()
                    .reclaim_sample_size(),
                128
            );
            return;
        }
        check_environment_overrides(
            "config::tests::reclaim_sample_size_environment_override",
            "FEUER_RECLAIM_SAMPLE_SIZE",
            "FEUER_TEST_RECLAIM_SAMPLE_CHILD",
            &[
                (None, "64"),
                (Some("1KiB"), "1024"),
                (Some("0"), "error"),
                (Some("bad"), "error"),
            ],
        );
    }

    #[test]
    fn access_count_half_life_environment_override() {
        const NAME: &str = "FEUER_ACCESS_COUNT_HALF_LIFE";
        const CHILD: &str = "FEUER_TEST_HALF_LIFE_CHILD";
        if let Ok(expected) = var(CHILD) {
            assert_eq!(
                builder()
                    .resolve_config()
                    .map(|config| config.access_count_half_life().get()),
                expected
                    .parse::<u64>()
                    .map_err(|_| CacheConfigError::InvalidAccessCountHalfLife)
            );
            assert_eq!(
                builder()
                    .access_count_half_life(128)
                    .resolve_config()
                    .unwrap()
                    .access_count_half_life()
                    .get(),
                128
            );
            return;
        }
        check_environment_overrides(
            "config::tests::access_count_half_life_environment_override",
            NAME,
            CHILD,
            &[
                (None, "262144"),
                (Some("1"), "1"),
                (Some("64KiB"), "65536"),
                (Some("18446744073709551615"), "18446744073709551615"),
                (Some("0"), "error"),
                (Some("bad"), "error"),
            ],
        );
    }

    #[test]
    fn history_entry_limit_environment_override() {
        if let Ok(expected) = var("FEUER_TEST_HISTORY_ENTRY_LIMIT_CHILD") {
            assert_eq!(
                builder()
                    .resolve_config()
                    .map(|config| config.history_entry_limit().get()),
                expected
                    .parse::<usize>()
                    .map_err(|_| CacheConfigError::InvalidHistoryEntryLimit)
            );
            assert_eq!(
                builder()
                    .history_entry_limit(32)
                    .resolve_config()
                    .unwrap()
                    .history_entry_limit()
                    .get(),
                32
            );
            return;
        }
        check_environment_overrides(
            "config::tests::history_entry_limit_environment_override",
            "FEUER_HISTORY_ENTRY_LIMIT",
            "FEUER_TEST_HISTORY_ENTRY_LIMIT_CHILD",
            &[
                (None, "10000000"),
                (Some("1"), "1"),
                (Some("1KiB"), "1024"),
                (Some("0"), "error"),
                (Some("bad"), "error"),
                (Some("18446744073709551616"), "error"),
            ],
        );
    }
}
