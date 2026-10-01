//! Recover entries by following metadata-only chunks, never by scanning payload chunks.

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    ops::Range,
    path::{Path, PathBuf},
    time::Duration,
};

use super::{page_format::*, *};

const RECOVERY_FILE_FORMAT_ID: &[u8; 8] = b"FEUMET11";
const RECOVERY_FILE_NAME: &str = "recovery-heads";
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(10);

pub(super) struct RecoveryState {
    path: PathBuf,
}

/// Metadata-chain heads and the file capacity needed to interpret them during recovery.
struct RecoveryHeads {
    capacity: u64,
    heads: Vec<u64>,
}

pub(super) fn shard_range(capacity: u64, count: usize, index: usize) -> Range<u64> {
    let chunks = capacity / CHUNK_BYTES;
    chunks * index as u64 / count as u64 * CHUNK_BYTES..chunks * (index + 1) as u64 / count as u64 * CHUNK_BYTES
}

impl RecoveryHeads {
    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(RECOVERY_FILE_FORMAT_ID);
        bytes.extend_from_slice(&self.capacity.to_le_bytes());
        bytes.extend_from_slice(&(self.heads.len() as u64).to_le_bytes());
        for head in &self.heads {
            bytes.extend_from_slice(&head.to_le_bytes());
        }
        bytes.extend_from_slice(&XxHash64::oneshot(0, &bytes).to_le_bytes());
        bytes
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 40 || &bytes[..8] != RECOVERY_FILE_FORMAT_ID {
            return None;
        }
        let capacity = read_u64(bytes, 8)?;
        let shards = usize::try_from(read_u64(bytes, 16)?).ok()?;
        if !(1..=64).contains(&shards)
            || capacity == 0
            || capacity > i64::MAX as u64
            || !capacity.is_multiple_of(CHUNK_BYTES)
        {
            return None;
        }
        let length = 24 + 8 * shards;
        if bytes.len() != length + 8 || read_u64(bytes, length)? != XxHash64::oneshot(0, &bytes[..length]) {
            return None;
        }
        let heads: Vec<_> = bytes[24..length]
            .as_chunks::<8>()
            .0
            .iter()
            .map(|bytes| u64::from_le_bytes(*bytes))
            .collect();
        for (index, &head) in heads.iter().enumerate() {
            let range = shard_range(capacity, shards, index);
            if head != NO_CHUNK && (!head.is_multiple_of(CHUNK_BYTES) || head < range.start || head >= range.end) {
                return None;
            }
        }
        Some(Self { capacity, heads })
    }

    fn save(&self, path: &Path) -> io::Result<()> {
        let temporary = path.with_extension("tmp");
        let mut file = File::create(&temporary)?;
        file.write_all(&self.encode())?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        File::open(path.parent().unwrap())?.sync_all()
    }
}

impl RecoveryState {
    pub(super) fn open(directory: &Path, capacity: u64, shards: usize) -> io::Result<(Self, Vec<u64>)> {
        let path = directory.join(RECOVERY_FILE_NAME);
        let mut bytes = Vec::new();
        let saved = File::open(&path).and_then(|file| file.take(1024).read_to_end(&mut bytes));
        let heads = match saved.ok().and_then(|_| RecoveryHeads::decode(&bytes)) {
            Some(heads) if heads.capacity == capacity && heads.heads.len() == shards => heads,
            _ => {
                tracing::info!(target: "feuer::storage", recovery_file = %path.display(), "resetting incompatible or missing metadata heads");
                let heads = RecoveryHeads {
                    capacity,
                    heads: vec![NO_CHUNK; shards],
                };
                heads.save(&path)?;
                heads
            }
        };
        Ok((Self { path }, heads.heads))
    }
}

impl DiskRangeCacheState {
    fn recovery_heads(&self) -> RecoveryHeads {
        RecoveryHeads {
            capacity: self.file.capacity(),
            heads: self
                .shards
                .iter()
                .map(|shard| shard.metadata_head.load(Ordering::Relaxed))
                .collect(),
        }
    }

    #[cfg(test)]
    pub(super) fn save_metadata_heads(&self) -> io::Result<()> {
        self.recovery_heads().save(&self.recovery.path)
    }

    pub(super) fn start_checkpoint_task(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let mut saved_heads = self.recovery_heads().heads;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(CHECKPOINT_INTERVAL).await;
                let Some(disk) = weak.upgrade() else { return };
                let heads = disk.recovery_heads();
                if heads.heads == saved_heads {
                    continue;
                }
                match tokio::task::spawn_blocking(move || {
                    heads.save(&disk.recovery.path)?;
                    Ok::<_, io::Error>(heads.heads)
                })
                .await
                {
                    Ok(Ok(heads)) => saved_heads = heads,
                    result => tracing::warn!(target: "feuer::storage", ?result, "could not save metadata heads"),
                }
            }
        });
    }

    /// Scan a shard's metadata before the cache becomes available.
    pub(super) async fn recover_shard(&self, shard_index: usize) {
        let shard = &self.shards[shard_index];
        let bounds = shard_range(self.file.capacity(), self.shards.len(), shard_index);
        // Reserve every metadata chunk before accepting any payload addresses from the records.
        shard.load_metadata_chain(&self.file, bounds.clone()).await;
        let mut payload_chunks = BTreeMap::<u64, DiskRegion>::new();
        let mut payload_ranges = BTreeMap::<u64, u64>::new();
        let count = shard.metadata.lock().unwrap().chunks.len();
        let mut entries = Vec::new();
        for chunk in 0..count {
            {
                let mut pages = shard.metadata.lock().unwrap();
                for slot in 0..RECORDS_PER_CHUNK {
                    let record = pages.chunks[chunk].record(slot);
                    if record.iter().all(|&byte| byte == 0) {
                        continue;
                    }
                    let entry = decode_entry(record, bounds.clone()).filter(|(key, _, _, payload)| {
                        self.shard_index_for_key(key) == shard_index
                            && payload_ranges
                                .range(..payload.end)
                                .next_back()
                                .is_none_or(|(_, end)| *end <= payload.start)
                    });
                    let Some((key, object_range, checksum, payload)) = entry else {
                        pages.discard_record(chunk, slot);
                        continue;
                    };
                    let start = payload.start / CHUNK_BYTES * CHUNK_BYTES;
                    let end = payload.end.next_multiple_of(CHUNK_BYTES);
                    if let std::collections::btree_map::Entry::Vacant(entry) = payload_chunks.entry(start) {
                        let Some(region) = shard
                            .allocator
                            .reserve_for_recovery(start / CHUNK_BYTES, (end - start) / CHUNK_BYTES)
                        else {
                            pages.discard_record(chunk, slot);
                            continue;
                        };
                        region.mark_recovered();
                        entry.insert(region);
                    }
                    let region = &payload_chunks[&start];
                    if payload.end > region.range().end {
                        pages.discard_record(chunk, slot);
                        continue;
                    }
                    let payload_region = region.slice(payload.clone());
                    let storage = DiskEntry {
                        in_flight_read: Weak::new(),
                        eviction_position: 0,
                        object_range,
                        payload_checksum: checksum,
                        metadata_slot: Some(shard.metadata_slot(chunk, slot)),
                        payload_region,
                    };
                    payload_ranges.insert(payload.start, payload.end);
                    entries.push((key, storage));
                }
            }
            shard.entry_index.lock().unwrap().insert_batch(&mut entries);
            tokio::task::yield_now().await;
        }
        shard.allocator.finish_recovery();
    }
}

impl DiskCacheShard {
    /// Read and repair the metadata chain before any entry can claim payload chunks.
    async fn load_metadata_chain(&self, file: &DataFile, bounds: Range<u64>) {
        let mut pages = metadata::MetadataPages::default();
        let mut address = self.metadata_head.load(Ordering::Relaxed);
        while address != NO_CHUNK {
            if !address.is_multiple_of(CHUNK_BYTES) || !bounds.contains(&address) {
                break;
            }
            let Some(region) = self.allocator.reserve_for_recovery(address / CHUNK_BYTES, 1) else {
                break;
            };
            let Ok(bytes) = file.read_recovery_chunk(&region).await else {
                break;
            };
            region.mark_recovered();
            let chunk = metadata::MetadataChunk {
                region,
                bytes: bytes.to_vec(),
            };
            let next = chunk.next();
            let chunk_index = pages.add_chunk(chunk);
            for page in 0..RECORD_PAGES {
                let bytes =
                    &pages.chunks[chunk_index].bytes[page * METADATA_PAGE_BYTES..(page + 1) * METADATA_PAGE_BYTES];
                let valid = validate_page(
                    bytes,
                    ENTRY_METADATA_PAGE_TAG,
                    0,
                    address + (page * METADATA_PAGE_BYTES) as u64,
                    page as u64,
                )
                .is_some_and(|(count, contents)| {
                    count == RECORDS_PER_PAGE as u64 && contents[PAGE_CONTENT_BYTES..].iter().all(|&byte| byte == 0)
                });
                if !valid {
                    pages.repair_page(chunk_index, page);
                }
            }
            match next {
                Some(next) => address = next,
                None => break,
            }
        }
        if address != NO_CHUNK {
            pages.end_chain();
        }
        if pages.chunks.is_empty() {
            self.metadata_head.store(NO_CHUNK, Ordering::Relaxed);
        }
        *self.metadata.lock().unwrap() = pages;
    }
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
}

fn valid_payload_range(range: &Range<u64>, bounds: Range<u64>) -> bool {
    range.start < range.end
        && range.start >= bounds.start
        && range.end <= bounds.end
        && range.start.is_multiple_of(PAYLOAD_ALIGNMENT_BYTES)
        && range.end.is_multiple_of(PAYLOAD_ALIGNMENT_BYTES)
        && (range.end <= (range.start / CHUNK_BYTES + 1) * CHUNK_BYTES || range.start.is_multiple_of(CHUNK_BYTES))
}

type DecodedEntry = (ObjectKeyHash, ByteRange, u64, Range<u64>);

fn decode_entry(bytes: &[u8], bounds: Range<u64>) -> Option<DecodedEntry> {
    if bytes.len() != ENTRY_METADATA_BYTES {
        return None;
    }
    let key = ObjectKeyHash(u128::from_le_bytes(bytes[..16].try_into().ok()?));
    let start = read_u64(bytes, 16)?;
    let range = ByteRange::new(start, start.checked_add(read_u64(bytes, 24)?)?).ok()?;
    let checksum = read_u64(bytes, 40)?;
    let start = read_u64(bytes, 32)?;
    let length =
        range.len().checked_add(PAYLOAD_ALIGNMENT_BYTES - 1)? / PAYLOAD_ALIGNMENT_BYTES * PAYLOAD_ALIGNMENT_BYTES;
    let payload = start..start.checked_add(length)?;
    valid_payload_range(&payload, bounds).then_some((key, range, checksum, payload))
}

#[cfg(test)]
pub(super) mod tests;
