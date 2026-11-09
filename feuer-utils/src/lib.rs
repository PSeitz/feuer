//! Shared types for the Feuer cache tiers.
//!
//! Memory and disk exchange hashed object identities.

use std::hash::Hash;

use xxhash_rust::xxh3::Xxh3;

pub mod config;

/// An immutable object's identity, represented by its XXH3-128 hash with seed zero.
///
/// The cache stores only hashes and treats colliding keys as the same object.
/// Keys must be non-adversarial.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectKeyHash(pub u128);

impl ObjectKeyHash {
    /// Streams the key's [`Hash`] representation into XXH3-128.
    ///
    /// `Hash` must include everything that distinguishes objects, including a namespace
    /// when different key types share a cache. Keep its encoding stable to reuse disk entries.
    /// Rust does not guarantee `Hash` encoding across compiler versions or platforms.
    /// Invalidate persistent caches when the encoding changes.
    pub fn from_hash(key: &(impl Hash + ?Sized)) -> Self {
        let mut hasher = Xxh3::new();
        key.hash(&mut hasher);
        Self(hasher.digest128())
    }
}

impl From<&str> for ObjectKeyHash {
    fn from(key: &str) -> Self {
        Self::from_hash(key)
    }
}

impl From<String> for ObjectKeyHash {
    fn from(key: String) -> Self {
        Self::from(key.as_str())
    }
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use xxhash_rust::xxh3::xxh3_128;

    use super::*;

    #[test]
    fn key_hash_uses_hash_input_and_full_xxh3_128_output() {
        assert_eq!(size_of::<ObjectKeyHash>(), 16);
        assert_eq!(ObjectKeyHash::from("abc").0, xxh3_128(b"abc\xff"));
        let key = "object/é/東京";
        assert_eq!(ObjectKeyHash::from(key), ObjectKeyHash::from(key.to_owned()));
        assert_eq!(ObjectKeyHash::from(key), ObjectKeyHash::from_hash(key));
        assert_eq!(ObjectKeyHash::from_hash(&42u64).0, xxh3_128(&42u64.to_ne_bytes()));
        assert_ne!(
            ObjectKeyHash::from_hash(&("ab", "c")),
            ObjectKeyHash::from_hash(&("a", "bc"))
        );
    }
}
