use std::path::{Path, PathBuf};

use feuer_types::{config::read_env_number, retention::RECLAIM_SAMPLE_SIZE};
use thiserror::Error;

/// Explicit capacities and location for one Feuer cache.
///
/// `disk_capacity` includes metadata and alignment overhead.
/// `memory_capacity` is a soft eviction target divided among the in-memory shards; oversized entries
/// can make entry allocation charges exceed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheConfig {
    directory: PathBuf,
    disk_capacity: u64,
    memory_capacity: u64,
    reclaim_sample_size: usize,
}

impl CacheConfig {
    /// Creates a cache configuration with no implicit capacity defaults.
    ///
    /// Reads `FEUER_RECLAIM_SAMPLE_SIZE` for the eviction candidate limit, defaulting
    /// to 64 when unset. Accepts size suffixes as multipliers (e.g. `1KiB` for 1024);
    /// the result must be positive and fit `usize`.
    pub fn new(
        directory: impl Into<PathBuf>,
        disk_capacity: u64,
        memory_capacity: u64,
    ) -> Result<Self, CacheConfigError> {
        if disk_capacity == 0 {
            return Err(CacheConfigError::InvalidDiskCapacity);
        }
        if memory_capacity == 0 {
            return Err(CacheConfigError::InvalidMemoryCapacity);
        }

        Ok(Self {
            directory: directory.into(),
            disk_capacity,
            memory_capacity,
            reclaim_sample_size: read_env_number("FEUER_RECLAIM_SAMPLE_SIZE", RECLAIM_SAMPLE_SIZE, 1)
                .map_err(|_| CacheConfigError::InvalidReclaimSampleSize)?,
        })
    }

    /// Sets the maximum candidates inspected per memory or disk eviction decision.
    /// Overrides the value read from `FEUER_RECLAIM_SAMPLE_SIZE`.
    pub fn with_reclaim_sample_size(mut self, sample_size: usize) -> Result<Self, CacheConfigError> {
        if sample_size == 0 {
            return Err(CacheConfigError::InvalidReclaimSampleSize);
        }
        self.reclaim_sample_size = sample_size;
        Ok(self)
    }

    /// Returns the maximum candidates inspected per eviction decision.
    pub const fn reclaim_sample_size(&self) -> usize {
        self.reclaim_sample_size
    }

    /// Returns the directory containing the cache's exclusively locked backing file.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Returns the configured disk capacity in bytes.
    pub const fn disk_capacity(&self) -> u64 {
        self.disk_capacity
    }

    /// Returns the soft memory payload target in bytes.
    pub const fn memory_capacity(&self) -> u64 {
        self.memory_capacity
    }
}

/// An invalid Feuer configuration.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum CacheConfigError {
    /// A fixed data file cannot have zero capacity.
    #[error("disk capacity must be greater than zero")]
    InvalidDiskCapacity,
    /// The configured memory eviction target must be positive.
    #[error("memory capacity must be greater than zero")]
    InvalidMemoryCapacity,
    /// The eviction candidate limit must be a positive integer that fits in `usize`.
    #[error(
        "reclaim sample size must be a positive number fitting usize; check FEUER_RECLAIM_SAMPLE_SIZE (e.g. 1KiB) or the explicit setting"
    )]
    InvalidReclaimSampleSize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reclaim_sample_size_environment_override() {
        if let Ok(expected) = std::env::var("FEUER_TEST_RECLAIM_SAMPLE_CHILD") {
            let result = CacheConfig::new("cache", 1, 1).map(|config| config.reclaim_sample_size());
            let expected = expected
                .parse::<usize>()
                .map_err(|_| CacheConfigError::InvalidReclaimSampleSize);
            assert_eq!(result, expected);
            return;
        }
        for (value, expected) in [("1KiB", "1024"), ("0", "error"), ("bad", "error")] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "config::tests::reclaim_sample_size_environment_override"])
                .env("FEUER_TEST_RECLAIM_SAMPLE_CHILD", expected)
                .env("FEUER_RECLAIM_SAMPLE_SIZE", value)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
        }
    }

    #[test]
    fn explicit_reclaim_sample_size_overrides_configuration() {
        let config = CacheConfig::new("cache", 1, 1).unwrap();
        assert_eq!(
            config.clone().with_reclaim_sample_size(0),
            Err(CacheConfigError::InvalidReclaimSampleSize)
        );
        assert_eq!(config.with_reclaim_sample_size(128).unwrap().reclaim_sample_size(), 128);
    }

    #[test]
    fn requires_both_capacity_roles_to_be_explicit() {
        for (disk_capacity, memory_capacity) in [(1 << 40, 256 << 20), (4 * (1 << 40), 1)] {
            let config = CacheConfig::new("cache", disk_capacity, memory_capacity).unwrap();

            assert_eq!(config.directory(), Path::new("cache"));
            assert_eq!(config.disk_capacity(), disk_capacity);
            assert_eq!(config.memory_capacity(), memory_capacity);
        }
    }

    #[test]
    fn rejects_zero_capacities() {
        for (disk_capacity, memory_capacity, error) in [
            (0, 1, CacheConfigError::InvalidDiskCapacity),
            (1, 0, CacheConfigError::InvalidMemoryCapacity),
        ] {
            assert_eq!(CacheConfig::new("cache", disk_capacity, memory_capacity), Err(error));
        }
    }
}
