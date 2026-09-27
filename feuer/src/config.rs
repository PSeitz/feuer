use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use feuer_types::{EvictionPolicy, retention::RECLAIM_SAMPLE_SIZE};
use thiserror::Error;

/// Explicit capacities and location for one Feuer cache.
///
/// `disk_capacity` is the fixed physical file size, including metadata and alignment.
/// Opening requires a positive multiple of 1 MiB. `memory_capacity` is a
/// soft eviction target divided among the in-memory shards; oversized entries
/// can make retained usage exceed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheConfig {
    directory: PathBuf,
    disk_capacity: u64,
    memory_capacity: u64,
    reclaim_sample_size: usize,
    eviction_policy: EvictionPolicy,
}

impl CacheConfig {
    /// Creates a cache configuration with no implicit capacity defaults.
    ///
    /// Reads `FEUER_EVICTION_POLICY` (`cost-aware` or `s3fifo`), defaulting to
    /// cost-aware when unset. Invalid values are rejected.
    /// Reads `FEUER_RECLAIM_SAMPLE_SIZE` for the eviction candidate limit, defaulting
    /// to 64 when unset. A set value must be a positive `usize` integer.
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
            eviction_policy: parse_eviction_policy(std::env::var_os("FEUER_EVICTION_POLICY").as_deref())?,
            reclaim_sample_size: parse_reclaim_sample_size(std::env::var_os("FEUER_RECLAIM_SAMPLE_SIZE").as_deref())?,
        })
    }

    /// Selects the eviction policy for both tiers, overriding `FEUER_EVICTION_POLICY`.
    /// S3-FIFO uses independent per-tier queues and disables memory range trimming.
    pub fn with_eviction_policy(mut self, policy: EvictionPolicy) -> Self {
        self.eviction_policy = policy;
        self
    }

    /// Returns the policy used by both tiers.
    pub const fn eviction_policy(&self) -> EvictionPolicy {
        self.eviction_policy
    }

    /// Sets the maximum candidates inspected per memory or disk eviction decision.
    /// For S3-FIFO this bounds queue-head processing per decision.
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

    /// Returns the configured fixed physical data-file capacity in bytes.
    pub const fn disk_capacity(&self) -> u64 {
        self.disk_capacity
    }

    /// Returns the soft memory payload target in bytes.
    pub const fn memory_capacity(&self) -> u64 {
        self.memory_capacity
    }
}

fn parse_eviction_policy(value: Option<&OsStr>) -> Result<EvictionPolicy, CacheConfigError> {
    match value {
        None => Ok(EvictionPolicy::default()),
        Some(value) => match value.to_str() {
            Some("cost-aware") => Ok(EvictionPolicy::CostAware),
            Some("s3fifo") => Ok(EvictionPolicy::S3Fifo),
            _ => Err(CacheConfigError::InvalidEvictionPolicy),
        },
    }
}

fn parse_reclaim_sample_size(value: Option<&OsStr>) -> Result<usize, CacheConfigError> {
    let Some(value) = value else {
        return Ok(RECLAIM_SAMPLE_SIZE);
    };
    value
        .to_str()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|&value| value > 0)
        .ok_or(CacheConfigError::InvalidReclaimSampleSize)
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
    /// The eviction policy must be one of the supported names.
    #[error("FEUER_EVICTION_POLICY must be cost-aware or s3fifo")]
    InvalidEvictionPolicy,
    /// The eviction candidate limit must be a positive integer that fits in `usize`.
    #[error(
        "reclaim sample size must be a positive usize integer; check FEUER_RECLAIM_SAMPLE_SIZE or the explicit setting"
    )]
    InvalidReclaimSampleSize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_eviction_policy() {
        assert_eq!(parse_eviction_policy(None), Ok(EvictionPolicy::CostAware));
        for (value, policy) in [
            ("cost-aware", EvictionPolicy::CostAware),
            ("s3fifo", EvictionPolicy::S3Fifo),
        ] {
            assert_eq!(parse_eviction_policy(Some(OsStr::new(value))), Ok(policy));
        }
        for value in ["", "lru", "S3FIFO", " s3fifo", "cost-aware "] {
            assert_eq!(
                parse_eviction_policy(Some(OsStr::new(value))),
                Err(CacheConfigError::InvalidEvictionPolicy)
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert_eq!(
                parse_eviction_policy(Some(OsStr::from_bytes(b"\xff"))),
                Err(CacheConfigError::InvalidEvictionPolicy)
            );
        }
    }

    #[test]
    fn explicit_eviction_policy_overrides_configuration() {
        let config = CacheConfig::new("cache", 1, 1).unwrap();
        let config = config.with_eviction_policy(EvictionPolicy::S3Fifo);
        assert_eq!(config.eviction_policy(), EvictionPolicy::S3Fifo);
        assert_eq!(config.clone().eviction_policy(), EvictionPolicy::S3Fifo);
        assert_eq!(
            config.with_eviction_policy(EvictionPolicy::CostAware).eviction_policy(),
            EvictionPolicy::CostAware
        );
    }

    #[test]
    fn parses_reclaim_sample_size() {
        assert_eq!(parse_reclaim_sample_size(None), Ok(64));
        for value in [1, 32, 128, usize::MAX] {
            assert_eq!(
                parse_reclaim_sample_size(Some(OsStr::new(&value.to_string()))),
                Ok(value)
            );
        }
        for value in ["", "0", "-1", "abc", "1.5", " 64", "18446744073709551616"] {
            assert_eq!(
                parse_reclaim_sample_size(Some(OsStr::new(value))),
                Err(CacheConfigError::InvalidReclaimSampleSize)
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert_eq!(
                parse_reclaim_sample_size(Some(OsStr::from_bytes(b"\xff"))),
                Err(CacheConfigError::InvalidReclaimSampleSize)
            );
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
        let config = CacheConfig::new("cache", 1 << 40, 256 << 20).unwrap();

        assert_eq!(config.directory(), Path::new("cache"));
        assert_eq!(config.disk_capacity(), 1 << 40);
        assert_eq!(config.memory_capacity(), 256 << 20);
    }

    #[test]
    fn represents_tib_scale_capacity() {
        let capacity = 4 * (1_u64 << 40);
        let config = CacheConfig::new("cache", capacity, 1).unwrap();

        assert_eq!(config.disk_capacity(), capacity);
        assert_eq!(config.memory_capacity(), 1);
    }

    #[test]
    fn rejects_zero_capacities() {
        assert_eq!(
            CacheConfig::new("cache", 0, 1).unwrap_err(),
            CacheConfigError::InvalidDiskCapacity
        );
        assert_eq!(
            CacheConfig::new("cache", 1, 0).unwrap_err(),
            CacheConfigError::InvalidMemoryCapacity
        );
    }
}
