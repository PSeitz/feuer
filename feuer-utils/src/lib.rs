//! Shared types for the Feuer cache tiers.
//!
//! Memory and disk exchange hashed object identities.

use std::{hash::Hash, io};

use siphasher::sip128::{Hasher128, SipHasher13};

pub mod config;

/// An immutable object's identity, hashed with its cache's SipHash-1-3-128 key.
///
/// The cache stores only hashes and treats colliding keys as the same object.
/// Identities are specific to the cache that produced them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectKeyHash(pub u128);

/// The secret key used to hash one cache's object identities.
#[derive(Clone)]
pub struct ObjectKeyHasher {
    key: [u8; 16],
}

impl ObjectKeyHasher {
    /// Generates a secret key using operating-system randomness.
    pub fn new() -> io::Result<Self> {
        let mut key = [0; 16];
        getrandom::fill(&mut key).map_err(io::Error::other)?;
        Ok(Self::from_key(key))
    }

    /// Restores the secret key from a cache header.
    pub fn from_key(key: [u8; 16]) -> Self {
        Self { key }
    }

    /// Returns the secret key for persistence in the cache header.
    /// Keep the cache file private and do not log this value.
    pub fn key(&self) -> [u8; 16] {
        self.key
    }

    /// Streams the key's [`Hash`] representation into SipHash-1-3-128.
    ///
    /// Include everything that distinguishes objects, including a namespace when key types
    /// share a cache. Rust does not guarantee `Hash` encoding across compiler versions or
    /// platforms. Invalidate persistent caches when the encoding changes.
    pub fn hash_key(&self, key: &(impl Hash + ?Sized)) -> ObjectKeyHash {
        let mut hasher = SipHasher13::new_with_key(&self.key);
        key.hash(&mut hasher);
        ObjectKeyHash(hasher.finish128().as_u128())
    }
}

#[cfg(test)]
mod tests {
    use std::{hash::Hasher, mem::size_of};

    use super::*;

    #[test]
    fn key_hash_uses_hash_input_and_full_siphash_128_output() {
        let hasher = ObjectKeyHasher::from_key([7; 16]);
        let mut expected = SipHasher13::new_with_key(&hasher.key());
        expected.write(b"abc\xff");
        assert_eq!(size_of::<ObjectKeyHash>(), 16);
        assert_eq!(hasher.hash_key("abc").0, expected.finish128().as_u128());
        let key = "object/é/東京";
        assert_eq!(hasher.hash_key(key), hasher.hash_key(&key.to_owned()));
        assert_ne!(hasher.hash_key(&("ab", "c")), hasher.hash_key(&("a", "bc")));
    }

    #[test]
    fn restoring_secret_preserves_hashes() {
        let hasher = ObjectKeyHasher::new().unwrap();
        let restored = ObjectKeyHasher::from_key(hasher.key());
        let other = ObjectKeyHasher::new().unwrap();
        assert_eq!(hasher.hash_key("object"), restored.hash_key("object"));
        assert_ne!(hasher.key(), other.key());
        assert_ne!(hasher.hash_key("object"), other.hash_key("object"));
    }
}
