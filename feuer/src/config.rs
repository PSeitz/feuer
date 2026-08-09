use std::path::{Path, PathBuf};

use thiserror::Error;

/// Explicit capacities and location for one Feuer cache.
///
/// Capacities are measured in payload bytes. The disk file is fixed to
/// `disk_capacity` when the disk lifecycle is enabled. `memory_capacity` is a
/// soft eviction target divided among the in-memory shards; oversized entries
/// can make retained usage exceed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    directory: PathBuf,
    disk_capacity: u64,
    memory_capacity: u64,
}

impl Config {
    /// Creates a cache configuration with no implicit capacity defaults.
    pub fn new(directory: impl Into<PathBuf>, disk_capacity: u64, memory_capacity: u64) -> Result<Self, ConfigError> {
        if disk_capacity == 0 {
            return Err(ConfigError::InvalidDiskCapacity);
        }
        if memory_capacity == 0 {
            return Err(ConfigError::InvalidMemoryCapacity);
        }

        Ok(Self {
            directory: directory.into(),
            disk_capacity,
            memory_capacity,
        })
    }

    /// Returns the directory configured for the cache's future disk lifecycle.
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

/// An invalid Feuer configuration.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    /// A fixed data file cannot have zero capacity.
    #[error("disk capacity must be greater than zero")]
    InvalidDiskCapacity,
    /// The configured memory eviction target must be positive.
    #[error("memory capacity must be greater than zero")]
    InvalidMemoryCapacity,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_both_capacity_roles_to_be_explicit() {
        let config = Config::new("cache", 1 << 40, 256 << 20).unwrap();

        assert_eq!(config.directory(), Path::new("cache"));
        assert_eq!(config.disk_capacity(), 1 << 40);
        assert_eq!(config.memory_capacity(), 256 << 20);
    }

    #[test]
    fn represents_tib_scale_capacity() {
        let capacity = 4 * (1_u64 << 40);
        let config = Config::new("cache", capacity, 1).unwrap();

        assert_eq!(config.disk_capacity(), capacity);
        assert_eq!(config.memory_capacity(), 1);
    }

    #[test]
    fn rejects_zero_capacities() {
        assert_eq!(
            Config::new("cache", 0, 1).unwrap_err(),
            ConfigError::InvalidDiskCapacity
        );
        assert_eq!(
            Config::new("cache", 1, 0).unwrap_err(),
            ConfigError::InvalidMemoryCapacity
        );
    }
}
