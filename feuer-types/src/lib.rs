//! Shared contract types and internal retention evidence for Feuer's immutable-object range tiers.
//!
//! The top-level `feuer` crate re-exports the range and download contracts.
//! Private memory and disk components exchange hashed object identities and
//! byte ranges without a dependency cycle.

#[doc(hidden)]
pub mod config;

mod download;
mod range;

#[doc(hidden)]
pub mod retention;

pub use download::{Download, DownloadError};
pub use range::{ByteRange, InvalidByteRange};

/// An immutable object's identity, represented by its XXH3-128 hash with seed zero.
///
/// Only the public cache boundary hashes strings. Internal tiers retain no original keys
/// and accept the probabilistic identity of the hash (keys must not be adversarial).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectKeyHash(pub u128);

impl From<&str> for ObjectKeyHash {
    fn from(key: &str) -> Self {
        Self(xxhash_rust::xxh3::xxh3_128(key.as_bytes()))
    }
}

impl From<String> for ObjectKeyHash {
    fn from(key: String) -> Self {
        Self::from(key.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_hash_uses_stable_xxh3_128_vectors() {
        assert_eq!(ObjectKeyHash::from("").0, 0x99aa06d3014798d86001c324468d497f);
        assert_eq!(ObjectKeyHash::from("abc").0, 0x06b05ab6733a618578af5f94892f3950);
        assert_eq!(std::mem::size_of::<ObjectKeyHash>(), 16);
        let key = "object/é/東京";
        assert_eq!(ObjectKeyHash::from(key), ObjectKeyHash::from(key.to_owned()));
        assert_eq!(
            ObjectKeyHash::from(key).0,
            xxhash_rust::xxh3::xxh3_128_with_seed(key.as_bytes(), 0)
        );
    }
}
