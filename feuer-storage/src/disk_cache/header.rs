//! Global file metadata in the first chunk, separate from shard metadata and payload chunks.

use bytes::Bytes;
use feuer_utils::ObjectKeyHasher;
use twox_hash::XxHash64;

use super::{DiskCacheError, recovery::shard_disk_range};
use crate::{DataFile, allocation::CHUNK_BYTES};

const HEADER_BYTES: usize = 4096;
const FORMAT_TAG: &[u8; 8] = b"FEUHDR16";

/// Clears shard metadata chains when the file header is incompatible, then writes a replacement.
/// A matching format, payload version, capacity, and header checksum preserve the file and SipHash key.
pub(super) fn clear_incompatible_metadata(
    file: &DataFile,
    payload_version: u64,
    num_shards: usize,
) -> Result<ObjectKeyHasher, DiskCacheError> {
    let header = file.read_at_blocking(0, HEADER_BYTES)?;
    if header[8..16] == *FORMAT_TAG
        && header[16..24] == payload_version.to_le_bytes()
        && header[24..32] == file.capacity().to_le_bytes()
        && header[..8] == XxHash64::oneshot(0, &header[8..]).to_le_bytes()
    {
        return Ok(ObjectKeyHasher::from_key(header[32..48].try_into().unwrap()));
    }
    let key_hasher = ObjectKeyHasher::new().map_err(DiskCacheError::HashKeyGeneration)?;
    let empty_chunk = Bytes::from(vec![0; CHUNK_BYTES as usize]);
    for shard_index in 0..num_shards {
        let start = shard_disk_range(file.capacity(), num_shards, shard_index).start;
        file.write_at_blocking(start, &empty_chunk)?;
    }
    // Persist every cleared chain before publishing the new payload version and SipHash key.
    file.sync_all()?;
    let mut header = vec![0; HEADER_BYTES];
    header[8..16].copy_from_slice(FORMAT_TAG);
    header[16..24].copy_from_slice(&payload_version.to_le_bytes());
    header[24..32].copy_from_slice(&file.capacity().to_le_bytes());
    header[32..48].copy_from_slice(&key_hasher.key());
    let checksum = XxHash64::oneshot(0, &header[8..]);
    header[..8].copy_from_slice(&checksum.to_le_bytes());
    file.write_at_blocking(0, &header)?;
    // Persist the new header before allowing entries with the new payload format and SipHash key.
    file.sync_all()?;
    Ok(key_hasher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DiskCache, DiskCacheOptions, IoMetrics,
        disk_cache::{
            page_format::{ENTRIES_PER_METADATA_CHUNK, ENTRIES_PER_METADATA_PAGE},
            tests::{download, entry_disk_ranges, with_manual_metadata_writes},
        },
    };
    use feuer_utils::ObjectKeyHash;
    use std::path::Path;

    /// Opens a cache with manually written metadata.
    fn open_cache(directory: &Path, capacity_chunks: u64, payload_version: u64) -> DiskCache {
        with_manual_metadata_writes(
            DiskCache::open(
                directory,
                capacity_chunks * CHUNK_BYTES,
                DiskCacheOptions {
                    payload_version,
                    ..DiskCacheOptions::default()
                },
            )
            .unwrap(),
        )
    }

    async fn insert_entries(cache: &DiskCache, keys: impl ExactSizeIterator<Item = usize>, payload_length: usize) {
        let count = keys.len();
        let entries = keys
            .map(|key| (ObjectKeyHash(key as u128), download(0, payload_length)))
            .collect();
        assert_eq!(cache.insert_batch(entries).await.unwrap(), count);
    }

    #[tokio::test]
    async fn keyed_entries_and_header_survive_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let other_directory = tempfile::tempdir().unwrap();
        let cache = open_cache(directory.path(), 4, 7);
        let other = open_cache(other_directory.path(), 4, 7);
        assert_ne!(cache.key_hasher().key(), other.key_hasher().key());
        let key = cache.key_hasher().hash_key("object");
        let payload = download(19, 100);
        assert!(cache.insert(key, payload.clone()).await.unwrap());
        cache.write_dirty_metadata_pages().await;
        let header = cache.disk.file.read_at(0, HEADER_BYTES).await.unwrap();
        assert_eq!(header[32..48], cache.key_hasher().key());
        drop(cache);

        let cache = open_cache(directory.path(), 4, 7);
        assert_eq!(cache.disk.file.read_at(0, HEADER_BYTES).await.unwrap(), header);
        assert_eq!(cache.key_hasher().hash_key("object"), key);
        assert_eq!(
            cache.get(&cache.key_hasher().hash_key("object")).await.unwrap(),
            payload
        );
        let secret = cache.key_hasher().key();
        drop(cache);

        let cache = open_cache(directory.path(), 4, 8);
        assert_ne!(cache.key_hasher().key(), secret);
        assert!(cache.get(&key).await.is_none());
        let secret = cache.key_hasher().key();
        drop(cache);

        let cache = open_cache(directory.path(), 5, 8);
        assert_ne!(cache.key_hasher().key(), secret);
    }

    #[tokio::test]
    async fn payload_version_changes_discard_entries_across_shards_and_reopens() {
        let directory = tempfile::tempdir().unwrap();
        let cache = open_cache(directory.path(), 384, 0);
        assert_eq!(cache.disk.shards.len(), 2);
        // Each shard needs two pages of entry metadata, so clearing only the first page is insufficient.
        insert_entries(&cache, 0..204, 1).await;
        cache.write_dirty_metadata_pages().await;
        drop(cache);

        let cache = open_cache(directory.path(), 384, 0);
        for key in 0..204 {
            assert!(cache.get(&ObjectKeyHash(key)).await.is_some());
        }
        drop(cache);

        for payload_version in [u64::MAX, 0] {
            let cache = open_cache(directory.path(), 384, payload_version);
            for key in 0..206 {
                assert!(cache.get(&ObjectKeyHash(key)).await.is_none());
            }
            for key in 204..206 {
                assert!(cache.insert(ObjectKeyHash(key), download(0, 1)).await.unwrap());
            }
            cache.write_dirty_metadata_pages().await;
            drop(cache);
            let cache = open_cache(directory.path(), 384, payload_version);
            for key in 0..204 {
                assert!(cache.get(&ObjectKeyHash(key)).await.is_none());
            }
            for key in 204..206 {
                assert!(cache.get(&ObjectKeyHash(key)).await.is_some());
            }
            drop(cache);
        }
    }

    #[tokio::test]
    async fn version_change_preserves_payload_bytes_and_initializes_reused_metadata_chunks() {
        let directory = tempfile::tempdir().unwrap();
        let cache = open_cache(directory.path(), 255, 0);
        // Fill two metadata chunks and two pages in a third chunk of shard zero.
        let entries = 2 * ENTRIES_PER_METADATA_CHUNK + ENTRIES_PER_METADATA_PAGE + 1;
        insert_entries(&cache, (0..2 * entries).step_by(2), 2).await;
        cache.write_dirty_metadata_pages().await;
        let second_chunk_address = {
            let metadata = cache.disk.shards[0].metadata_pages.lock().unwrap();
            assert_eq!(metadata.chunks.len(), 3);
            metadata.chunks[1].reserved_chunk.disk_byte_range().start
        };
        let file = &cache.disk.file;
        let old_metadata = file.read_at(second_chunk_address, CHUNK_BYTES as usize).await.unwrap();
        let (payload, _) = entry_disk_ranges(&cache, &ObjectKeyHash(0)).await;
        drop(cache);

        let cache = open_cache(directory.path(), 255, 1);
        let file = &cache.disk.file;
        assert_eq!(file.read_at(payload.start, 2).await.unwrap(), download(0, 2));
        assert_eq!(
            file.read_at(second_chunk_address, CHUNK_BYTES as usize).await.unwrap(),
            old_metadata
        );
        // Reuse the second chunk with one entry, replacing its stale pages and link to the third chunk.
        let new_entries = ENTRIES_PER_METADATA_CHUNK + 1;
        let first_new_key = 2 * entries;
        insert_entries(&cache, (first_new_key..first_new_key + 2 * new_entries).step_by(2), 2).await;
        let shard = &cache.disk.shards[0];
        let reused_chunk_address = shard.metadata_pages.lock().unwrap().chunks[1]
            .reserved_chunk
            .disk_byte_range()
            .start;
        assert_eq!(reused_chunk_address, second_chunk_address);
        let short_directory = tempfile::tempdir().unwrap();
        let short_file = DataFile::open(short_directory.path(), second_chunk_address, IoMetrics::noop()).unwrap();
        assert!(shard.write_dirty_metadata_pages(&short_file).await.is_err());
        // Only the first chunk changes after the second chunk's initialization failed.
        shard.remove_entry(&ObjectKeyHash(first_new_key as u128));
        let first_dirty_page = *shard.metadata_pages.lock().unwrap().dirty_pages.first().unwrap();
        assert_eq!(first_dirty_page.chunk_index, 0);
        cache.write_dirty_metadata_pages().await;
        drop(cache);

        let cache = open_cache(directory.path(), 255, 1);
        for key in 0..entries {
            assert!(!cache.contains(&ObjectKeyHash(2 * key as u128)));
        }
        assert!(!cache.contains(&ObjectKeyHash(first_new_key as u128)));
        for key in 1..new_entries {
            assert!(cache.contains(&ObjectKeyHash((first_new_key + 2 * key) as u128)));
        }
    }

    #[tokio::test]
    async fn capacity_changes_do_not_recover_entries_from_a_previous_payload_version() {
        let directory = tempfile::tempdir().unwrap();
        // The second shard starts at chunk 129 or 130. Resizing back must not follow the old chain.
        for (capacity_chunks, payload_version) in [(258, 0), (260, 1), (258, 1)] {
            let cache = open_cache(directory.path(), capacity_chunks, payload_version);
            assert!(cache.get(&ObjectKeyHash(3)).await.is_none());
            if payload_version == 0 {
                // The second payload stays beyond either shard start.
                for key in [1, 3] {
                    assert!(cache.insert(ObjectKeyHash(key), download(0, 2)).await.unwrap());
                }
                cache.write_dirty_metadata_pages().await;
            }
        }
    }

    #[tokio::test]
    async fn invalid_headers_clear_entries_and_rewrite_header() {
        let directory = tempfile::tempdir().unwrap();
        for header_byte_offset in [
            None,
            Some(0),
            Some(8),
            Some(16),
            Some(24),
            Some(32),
            Some(47),
            Some(48),
            Some(HEADER_BYTES - 1),
        ] {
            let cache = open_cache(directory.path(), 384, 7);
            assert!(cache.insert(ObjectKeyHash(1), download(0, 1)).await.unwrap());
            cache.write_dirty_metadata_pages().await;
            let header = cache.disk.file.read_at(0, HEADER_BYTES).await.unwrap();
            assert_eq!(&header[8..16], b"FEUHDR16");
            assert_eq!(header[16..24], 7u64.to_le_bytes());
            assert_eq!(header[24..32], (384 * CHUNK_BYTES).to_le_bytes());
            assert_eq!(header[32..48], cache.key_hasher().key());
            assert!(header[48..].iter().all(|&byte| byte == 0));
            let mut corrupt_header = header.to_vec();
            if let Some(offset) = header_byte_offset {
                corrupt_header[offset] ^= 1;
                if offset == 8 {
                    // The old format must be rejected even with an intact checksum.
                    corrupt_header[8..16].copy_from_slice(b"FEUHDR15");
                    corrupt_header[32..].fill(0);
                    let checksum = XxHash64::oneshot(0, &corrupt_header[8..]);
                    corrupt_header[..8].copy_from_slice(&checksum.to_le_bytes());
                }
            } else {
                corrupt_header.fill(0);
            }
            cache.disk.file.write_at(0, &Bytes::from(corrupt_header)).await.unwrap();
            drop(cache);
            let cache = open_cache(directory.path(), 384, 7);
            assert!(cache.get(&ObjectKeyHash(1)).await.is_none());
            let replacement = cache.disk.file.read_at(0, HEADER_BYTES).await.unwrap();
            assert_eq!(&replacement[8..32], &header[8..32]);
            assert_ne!(&replacement[32..48], &header[32..48]);
            assert_eq!(replacement[32..48], cache.key_hasher().key());
            assert!(replacement[48..].iter().all(|&byte| byte == 0));
            assert_eq!(replacement[..8], XxHash64::oneshot(0, &replacement[8..]).to_le_bytes());
            drop(cache);
            let cache = open_cache(directory.path(), 384, 7);
            assert_eq!(cache.disk.file.read_at(0, HEADER_BYTES).await.unwrap(), replacement);
        }
    }
}
