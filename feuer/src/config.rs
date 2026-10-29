use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use feuer_historian::DEFAULT_ACCESS_COUNT_HALF_LIFE;
use feuer_memory::{read_idle_buffer_pool_percent, retention::RECLAIM_SAMPLE_SIZE};
use feuer_types::config::read_env_number;
use thiserror::Error;

/// Capacities and location for a Feuer cache.
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
    directory: PathBuf,
    file_name: String,
    payload_version: u64,
    disk_capacity: u64,
    memory_capacity: u64,
    idle_buffer_pool_percent: u64,
    access_count_half_life: u64,
    eviction: SampledEvictionConfig,
}

impl CacheConfig {
    /// Creates a cache configuration with no implicit capacity defaults.
    ///
    /// Requires positive memory capacity. Disk capacity is checked when opening the cache.
    /// Uses [`SampledEvictionConfig::new`] for eviction settings.
    /// Reads `FEUER_ACCESS_COUNT_HALF_LIFE`, defaulting to 262,144 requests.
    /// The half-life must be positive and applies to both tiers of this cache.
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

        Ok(Self {
            directory: directory.into(),
            file_name: "data".to_owned(),
            payload_version: 0,
            disk_capacity,
            memory_capacity,
            eviction: SampledEvictionConfig::new()?,
            access_count_half_life: read_env_number("FEUER_ACCESS_COUNT_HALF_LIFE", DEFAULT_ACCESS_COUNT_HALF_LIFE, 1)
                .map_err(|_| CacheConfigError::InvalidAccessCountHalfLife)?,
            idle_buffer_pool_percent: read_idle_buffer_pool_percent()
                .map_err(|_| CacheConfigError::InvalidIdleBufferPoolPercent)?,
        })
    }

    /// Sets the backing filename within the cache directory. Defaults to `data`.
    /// Different filenames allow independent caches in the same directory.
    /// Must be a single filename, without NUL bytes or the reserved `.feuer.` prefix.
    pub fn with_file_name(mut self, file_name: impl Into<String>) -> Result<Self, CacheConfigError> {
        let file_name = file_name.into();
        if Path::new(&file_name).file_name() != Some(OsStr::new(&file_name))
            || file_name.contains('\0')
            || file_name.starts_with(".feuer.")
        {
            return Err(CacheConfigError::InvalidFileName);
        }
        self.file_name = file_name;
        Ok(self)
    }

    /// Sets the application's payload format version. Defaults to zero.
    /// Opening with a different version clears all previous disk entries, including on downgrade.
    /// This is independent of Feuer's disk format version and unused when disk is disabled.
    pub fn with_payload_version(mut self, payload_version: u64) -> Self {
        self.payload_version = payload_version;
        self
    }

    /// Returns the application's payload format version.
    pub const fn payload_version(&self) -> u64 {
        self.payload_version
    }

    /// Sets the sampled eviction settings shared by memory and disk.
    pub fn with_eviction(mut self, eviction: SampledEvictionConfig) -> Self {
        self.eviction = eviction;
        self
    }

    /// Returns the sampled eviction settings.
    pub const fn eviction(&self) -> &SampledEvictionConfig {
        &self.eviction
    }

    /// Sets the access count half-life in requests across all keys of this cache.
    /// Applies to both tiers. Overrides `FEUER_ACCESS_COUNT_HALF_LIFE` for this cache.
    pub fn with_access_count_half_life(mut self, half_life: u64) -> Result<Self, CacheConfigError> {
        if half_life == 0 {
            return Err(CacheConfigError::InvalidAccessCountHalfLife);
        }
        self.access_count_half_life = half_life;
        Ok(self)
    }

    /// Returns the access count half-life in requests across all keys of this cache.
    pub const fn access_count_half_life(&self) -> u64 {
        self.access_count_half_life
    }

    /// Sets the maximum bytes retained in idle buffers as a percentage of memory capacity (0–100).
    /// Zero disables idle retention. Overrides `FEUER_IDLE_BUFFER_POOL_PERCENT` for this cache.
    pub fn with_idle_buffer_pool_percent(mut self, percent: u64) -> Result<Self, CacheConfigError> {
        if percent > 100 {
            return Err(CacheConfigError::InvalidIdleBufferPoolPercent);
        }
        self.idle_buffer_pool_percent = percent;
        Ok(self)
    }

    /// Returns the maximum bytes retained in idle buffers as a percentage of memory capacity.
    pub const fn idle_buffer_pool_percent(&self) -> u64 {
        self.idle_buffer_pool_percent
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

    /// Returns the soft memory eviction target for entry allocation charges in bytes.
    pub const fn memory_capacity(&self) -> u64 {
        self.memory_capacity
    }
}

/// Settings for evicting the lowest-scoring entry from a sample.
/// Scoring parameters belong to the supplied [`crate::RetentionScorer`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampledEvictionConfig {
    sample_size: usize,
}

impl SampledEvictionConfig {
    /// Reads `FEUER_RECLAIM_SAMPLE_SIZE`, defaulting to 64 when unset.
    /// Accepts size suffixes as multipliers (e.g. `1KiB` for 1024).
    /// The result must be positive and fit `usize`.
    pub fn new() -> Result<Self, CacheConfigError> {
        Ok(Self {
            sample_size: read_env_number("FEUER_RECLAIM_SAMPLE_SIZE", RECLAIM_SAMPLE_SIZE, 1)
                .map_err(|_| CacheConfigError::InvalidReclaimSampleSize)?,
        })
    }

    /// Sets the maximum candidates inspected per decision.
    /// Overrides the value read from `FEUER_RECLAIM_SAMPLE_SIZE`.
    pub fn with_sample_size(mut self, sample_size: usize) -> Result<Self, CacheConfigError> {
        if sample_size == 0 {
            return Err(CacheConfigError::InvalidReclaimSampleSize);
        }
        self.sample_size = sample_size;
        Ok(self)
    }

    /// Returns the maximum candidates inspected per eviction decision.
    pub const fn sample_size(&self) -> usize {
        self.sample_size
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
    /// The idle buffer ceiling must be a percentage between 0 and 100.
    #[error(
        "idle buffer pool percent must be between 0 and 100; check FEUER_IDLE_BUFFER_POOL_PERCENT or the explicit setting"
    )]
    InvalidIdleBufferPoolPercent,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_environment_overrides(test: &str, name: &str, child: &str, cases: &[(Option<&str>, &str)]) {
        for &(setting, expected) in cases {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", test]).env(child, expected).env_remove(name);
            if let Some(setting) = setting {
                command.env(name, setting);
            }
            let output = command.output().unwrap();
            assert!(output.status.success(), "{output:?}");
        }
    }

    #[test]
    fn configures_independent_files_in_one_directory() {
        let config = CacheConfig::new("cache", 1, 1).unwrap();
        assert_eq!(config.file_name(), "data");
        assert_eq!(config.clone().with_file_name("images").unwrap().file_name(), "images");
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
                config.clone().with_file_name(name),
                Err(CacheConfigError::InvalidFileName)
            );
        }
    }

    #[test]
    fn reclaim_sample_size_environment_override() {
        if let Ok(expected) = std::env::var("FEUER_TEST_RECLAIM_SAMPLE_CHILD") {
            let result = CacheConfig::new("cache", 1, 1).map(|config| config.eviction().sample_size());
            let expected = expected
                .parse::<usize>()
                .map_err(|_| CacheConfigError::InvalidReclaimSampleSize);
            assert_eq!(result, expected);
            return;
        }
        check_environment_overrides(
            "config::tests::reclaim_sample_size_environment_override",
            "FEUER_RECLAIM_SAMPLE_SIZE",
            "FEUER_TEST_RECLAIM_SAMPLE_CHILD",
            &[(Some("1KiB"), "1024"), (Some("0"), "error"), (Some("bad"), "error")],
        );
    }

    #[test]
    fn explicit_reclaim_sample_size_overrides_configuration() {
        let eviction = SampledEvictionConfig::new().unwrap();
        assert_eq!(
            eviction.clone().with_sample_size(0),
            Err(CacheConfigError::InvalidReclaimSampleSize)
        );
        let eviction = eviction.with_sample_size(128).unwrap();
        let config = CacheConfig::new("cache", 1, 1).unwrap().with_eviction(eviction.clone());
        assert_eq!(config.eviction(), &eviction);
        assert_eq!(config.eviction().sample_size(), 128);
    }

    #[test]
    fn access_count_half_life_environment_override() {
        const NAME: &str = "FEUER_ACCESS_COUNT_HALF_LIFE";
        const CHILD: &str = "FEUER_TEST_HALF_LIFE_CHILD";
        if let Ok(expected) = std::env::var(CHILD) {
            assert_eq!(
                CacheConfig::new("cache", 1, 1).map(|config| config.access_count_half_life()),
                expected
                    .parse::<u64>()
                    .map_err(|_| CacheConfigError::InvalidAccessCountHalfLife)
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
        assert_eq!(
            CacheConfig::new("cache", 1, 1).unwrap().with_access_count_half_life(0),
            Err(CacheConfigError::InvalidAccessCountHalfLife)
        );
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
        check_environment_overrides(
            "config::tests::idle_buffer_pool_percent_environment_override",
            NAME,
            CHILD,
            &[
                (None, "7"),
                (Some("0"), "0"),
                (Some("10"), "10"),
                (Some("100"), "100"),
                (Some("101"), "error"),
                (Some("-1"), "error"),
                (Some("1.5"), "error"),
                (Some("bad"), "error"),
            ],
        );
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
    fn preserves_configured_capacities() {
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
