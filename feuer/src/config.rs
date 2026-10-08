use std::path::{Path, PathBuf};

use feuer_memory::{DEFAULT_IDLE_BUFFER_POOL_PERCENT, retention::RECLAIM_SAMPLE_SIZE};
use feuer_types::config::read_env_number;
use thiserror::Error;

/// Explicit capacities and location for one Feuer cache.
///
/// `disk_capacity` includes metadata and alignment overhead; zero disables the disk tier.
/// When disk is disabled, `directory` is unused.
/// `memory_capacity` is a soft eviction target divided among the in-memory shards; oversized entries
/// can make entry allocation charges exceed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheConfig {
    directory: PathBuf,
    disk_capacity: u64,
    memory_capacity: u64,
    reclaim_sample_size: usize,
    idle_buffer_pool_percent: u64,
}

impl CacheConfig {
    /// Creates a cache configuration with no implicit capacity defaults.
    ///
    /// Reads `FEUER_RECLAIM_SAMPLE_SIZE` for the eviction candidate limit, defaulting
    /// to 64 when unset. Accepts size suffixes as multipliers (e.g. `1KiB` for 1024);
    /// the result must be positive and fit `usize`.
    /// Reads `FEUER_IDLE_BUFFER_POOL_PERCENT` for the idle buffer ceiling, defaulting
    /// to 7 when unset. The percentage must be between 0 and 100.
    pub fn new(
        directory: impl Into<PathBuf>,
        disk_capacity: u64,
        memory_capacity: u64,
    ) -> Result<Self, CacheConfigError> {
        if memory_capacity == 0 {
            return Err(CacheConfigError::InvalidMemoryCapacity);
        }

        let idle_buffer_pool_percent =
            read_env_number("FEUER_IDLE_BUFFER_POOL_PERCENT", DEFAULT_IDLE_BUFFER_POOL_PERCENT, 0)
                .map_err(|_| CacheConfigError::InvalidIdleBufferPoolPercent)?;
        if idle_buffer_pool_percent > 100 {
            return Err(CacheConfigError::InvalidIdleBufferPoolPercent);
        }

        Ok(Self {
            directory: directory.into(),
            disk_capacity,
            memory_capacity,
            reclaim_sample_size: read_env_number("FEUER_RECLAIM_SAMPLE_SIZE", RECLAIM_SAMPLE_SIZE, 1)
                .map_err(|_| CacheConfigError::InvalidReclaimSampleSize)?,
            idle_buffer_pool_percent,
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

    /// Sets the maximum idle buffers as a percentage of memory capacity (0–100).
    /// Zero disables idle retention. Overrides `FEUER_IDLE_BUFFER_POOL_PERCENT` for this cache.
    pub fn with_idle_buffer_pool_percent(mut self, percent: u64) -> Result<Self, CacheConfigError> {
        if percent > 100 {
            return Err(CacheConfigError::InvalidIdleBufferPoolPercent);
        }
        self.idle_buffer_pool_percent = percent;
        Ok(self)
    }

    /// Returns the maximum idle buffers as a percentage of memory capacity.
    pub const fn idle_buffer_pool_percent(&self) -> u64 {
        self.idle_buffer_pool_percent
    }

    /// Returns the maximum candidates inspected per eviction decision.
    pub const fn reclaim_sample_size(&self) -> usize {
        self.reclaim_sample_size
    }

    /// Returns the directory containing the cache's exclusively locked backing file.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Returns the configured disk capacity in bytes; zero disables the disk tier.
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
    /// The configured memory eviction target must be positive.
    #[error("memory capacity must be greater than zero")]
    InvalidMemoryCapacity,
    /// The eviction candidate limit must be a positive integer that fits in `usize`.
    #[error(
        "reclaim sample size must be a positive number fitting usize; check FEUER_RECLAIM_SAMPLE_SIZE (e.g. 1KiB) or the explicit setting"
    )]
    InvalidReclaimSampleSize,
    /// The idle buffer ceiling must be a percentage between 0 and 100.
    #[error(
        "idle buffer pool percent must be between 0 and 100; check FEUER_IDLE_BUFFER_POOL_PERCENT or the explicit setting"
    )]
    InvalidIdleBufferPoolPercent,
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
    fn idle_buffer_pool_percent_environment_override() {
        const NAME: &str = "FEUER_IDLE_BUFFER_POOL_PERCENT";
        const CHILD: &str = "FEUER_TEST_IDLE_BUFFER_POOL_PERCENT_CHILD";
        if let Ok(expected) = std::env::var(CHILD) {
            let result = CacheConfig::new("cache", 1, 1).map(|config| config.idle_buffer_pool_percent());
            let expected = expected
                .parse::<u64>()
                .map_err(|_| CacheConfigError::InvalidIdleBufferPoolPercent);
            assert_eq!(result, expected);
            return;
        }
        for (setting, expected) in [
            (None, "7"),
            (Some("0"), "0"),
            (Some("10"), "10"),
            (Some("100"), "100"),
            (Some("101"), "error"),
            (Some("-1"), "error"),
            (Some("1.5"), "error"),
            (Some("bad"), "error"),
        ] {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .args([
                    "--exact",
                    "config::tests::idle_buffer_pool_percent_environment_override",
                ])
                .env(CHILD, expected)
                .env_remove(NAME);
            if let Some(setting) = setting {
                child.env(NAME, setting);
            }
            let output = child.output().unwrap();
            assert!(output.status.success(), "{output:?}");
        }
    }

    #[test]
    fn explicit_idle_buffer_pool_percent_overrides_configuration() {
        let config = CacheConfig::new("cache", 1, 1).unwrap();
        for percent in [0, 10, 100] {
            assert_eq!(
                config
                    .clone()
                    .with_idle_buffer_pool_percent(percent)
                    .unwrap()
                    .idle_buffer_pool_percent(),
                percent
            );
        }
        for percent in [101, u64::MAX] {
            assert_eq!(
                config.clone().with_idle_buffer_pool_percent(percent),
                Err(CacheConfigError::InvalidIdleBufferPoolPercent)
            );
        }
    }

    #[test]
    fn requires_both_capacity_roles_to_be_explicit() {
        for (disk_capacity, memory_capacity) in [(0, 256 << 20), (1 << 40, 256 << 20), (4 * (1 << 40), 1)] {
            let config = CacheConfig::new("cache", disk_capacity, memory_capacity).unwrap();

            assert_eq!(config.directory(), Path::new("cache"));
            assert_eq!(config.disk_capacity(), disk_capacity);
            assert_eq!(config.memory_capacity(), memory_capacity);
        }
    }

    #[test]
    fn rejects_zero_memory_capacity() {
        for disk_capacity in [0, 1] {
            assert_eq!(
                CacheConfig::new("cache", disk_capacity, 0),
                Err(CacheConfigError::InvalidMemoryCapacity)
            );
        }
    }
}
