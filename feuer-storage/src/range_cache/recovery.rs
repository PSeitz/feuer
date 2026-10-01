//! Best-effort, incremental recovery. New writes may claim any still-free chunk first.

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    ops::Range,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use super::*;

const RECOVERY_FILE_FORMAT_ID: &[u8; 8] = b"FEUEND10";
const RECOVERY_FILE_NAME: &str = "recovery-ends";
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(10);

pub(super) struct RecoveryState {
    path: PathBuf,
    pub(super) running: AtomicBool,
}

struct RecoveryBounds {
    capacity: u64,
    ends: Vec<u64>,
}

pub(super) fn shard_range(capacity: u64, count: usize, index: usize) -> Range<u64> {
    let chunks = capacity / CHUNK_BYTES;
    chunks * index as u64 / count as u64 * CHUNK_BYTES..chunks * (index + 1) as u64 / count as u64 * CHUNK_BYTES
}

impl RecoveryBounds {
    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(RECOVERY_FILE_FORMAT_ID);
        bytes.extend_from_slice(&self.capacity.to_le_bytes());
        bytes.extend_from_slice(&(self.ends.len() as u64).to_le_bytes());
        for end in &self.ends {
            bytes.extend_from_slice(&end.to_le_bytes());
        }
        bytes.extend_from_slice(&XxHash64::oneshot(0, &bytes).to_le_bytes());
        bytes
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 40 || &bytes[..8] != RECOVERY_FILE_FORMAT_ID {
            return None;
        }
        let capacity = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let shards = usize::try_from(u64::from_le_bytes(bytes[16..24].try_into().unwrap())).ok()?;
        if !(1..=64).contains(&shards)
            || capacity == 0
            || capacity > i64::MAX as u64
            || !capacity.is_multiple_of(CHUNK_BYTES)
        {
            return None;
        }
        let length = 24 + 8 * shards;
        if bytes.len() != length + 8 || bytes[length..] != XxHash64::oneshot(0, &bytes[..length]).to_le_bytes() {
            return None;
        }
        let ends: Vec<_> = bytes[24..length]
            .as_chunks::<8>()
            .0
            .iter()
            .map(|bytes| u64::from_le_bytes(*bytes))
            .collect();
        for (index, end) in ends.iter().enumerate() {
            let range = shard_range(capacity, shards, index);
            if !end.is_multiple_of(CHUNK_BYTES) || *end < range.start || *end > range.end {
                return None;
            }
        }
        Some(Self { capacity, ends })
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
        // The inventory is tiny even at the maximum shard count. Never read an unbounded file.
        let mut bytes = Vec::new();
        let saved = File::open(&path).and_then(|file| file.take(1024).read_to_end(&mut bytes));
        let bounds = match saved.ok().and_then(|_| RecoveryBounds::decode(&bytes)) {
            Some(bounds) if bounds.capacity == capacity && bounds.ends.len() == shards => bounds,
            previous => {
                if let Some(previous) = previous {
                    let reason = if previous.ends.len() != shards {
                        "shard count changed"
                    } else {
                        "capacity changed"
                    };
                    tracing::warn!(
                        target: "feuer::storage",
                        recovery_file = %path.display(),
                        reason,
                        previous_shard_count = previous.ends.len(),
                        shard_count = shards,
                        previous_capacity_bytes = previous.capacity,
                        capacity_bytes = capacity,
                        "resetting disk recovery scan bounds: incompatible layout"
                    );
                } else {
                    tracing::info!(
                        target: "feuer::storage",
                        recovery_file = %path.display(),
                        "resetting disk recovery scan bounds: recovery ends missing, invalid, or incompatible"
                    );
                }
                let bounds = RecoveryBounds {
                    capacity,
                    ends: (0..shards)
                        .map(|index| shard_range(capacity, shards, index).start)
                        .collect(),
                };
                bounds.save(&path)?;
                bounds
            }
        };
        let running = bounds
            .ends
            .iter()
            .enumerate()
            .any(|(index, &end)| end > shard_range(capacity, shards, index).start);
        Ok((
            Self {
                path,
                running: AtomicBool::new(running),
            },
            bounds.ends,
        ))
    }
}

impl DiskRangeCacheState {
    fn recovery_bounds(&self) -> RecoveryBounds {
        RecoveryBounds {
            capacity: self.file.capacity(),
            ends: self
                .shards
                .iter()
                .map(|shard| shard.written_end.load(Ordering::Relaxed))
                .collect(),
        }
    }

    #[cfg(test)]
    pub(super) fn save_recovery_ends(&self) -> io::Result<()> {
        self.recovery_bounds().save(&self.recovery.path)
    }

    pub(super) fn start_background_tasks(self: &Arc<Self>, shard_scan_ends: Vec<u64>) {
        let mut saved_shard_scan_ends = shard_scan_ends.clone();
        if self.recovery.running.load(Ordering::Relaxed) {
            let weak = Arc::downgrade(self);
            let capacity = self.file.capacity();
            let started = Instant::now();
            tracing::info!(
                target: "feuer::storage",
                recovery_file = %self.recovery.path.display(),
                shard_count = shard_scan_ends.len(),
                capacity_bytes = capacity,
                "starting disk cache recovery"
            );
            tokio::spawn(async move {
                for (shard_index, &scan_end) in shard_scan_ends.iter().enumerate() {
                    let shard_byte_range = shard_range(capacity, shard_scan_ends.len(), shard_index);
                    let scan_start = shard_byte_range.start;
                    tracing::debug!(
                        target: "feuer::storage",
                        shard_index,
                        shard_capacity_bytes = shard_byte_range.end - scan_start,
                        scan_start,
                        scan_end,
                        scan_chunks = (scan_end - scan_start) / CHUNK_BYTES,
                        "starting disk shard recovery scan"
                    );
                    let mut chunk_address = scan_start;
                    while chunk_address < scan_end {
                        {
                            let Some(disk) = weak.upgrade() else { return };
                            chunk_address = disk
                                .recover_chunk(shard_index, chunk_address, scan_end)
                                .await
                                .unwrap_or(chunk_address + CHUNK_BYTES);
                        }
                        // Do not retain the cache between attempts, or monopolize an executor on skipped chunks.
                        tokio::task::yield_now().await;
                    }
                    let Some(disk) = weak.upgrade() else { return };
                    disk.shards[shard_index].allocator.finish_recovery();
                }
                if let Some(disk) = weak.upgrade() {
                    disk.recovery.running.store(false, Ordering::Release);
                    tracing::info!(
                        target: "feuer::storage",
                        recovery_file = %disk.recovery.path.display(),
                        elapsed_seconds = started.elapsed().as_secs_f64(),
                        "disk cache recovery finished"
                    );
                }
            });
        } else {
            tracing::debug!(
                target: "feuer::storage",
                recovery_file = %self.recovery.path.display(),
                "skipping disk cache recovery: no saved chunks to scan"
            );
        }
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(CHECKPOINT_INTERVAL).await;
                let Some(disk) = weak.upgrade() else { return };
                let bounds = disk.recovery_bounds();
                if bounds.ends == saved_shard_scan_ends {
                    continue;
                }
                // Retain the directory lock until the atomic replacement completes, even on cancellation.
                match tokio::task::spawn_blocking(move || {
                    bounds.save(&disk.recovery.path)?;
                    Ok::<_, io::Error>(bounds.ends)
                })
                .await
                {
                    Ok(Ok(shard_scan_ends)) => saved_shard_scan_ends = shard_scan_ends,
                    result => tracing::warn!(target: "feuer::storage", ?result, "could not save disk recovery ends"),
                }
            }
        });
    }

    /// Returns the allocation's end so the scan skips its continuation chunks.
    async fn recover_chunk(&self, shard_index: usize, address: u64, scan_end: u64) -> Option<u64> {
        let shard = &self.shards[shard_index];
        let mut region = shard.allocator.reserve_for_recovery(address / CHUNK_BYTES, 1)?;
        let metadata = self.read_metadata(&region).await?;
        let entries = metadata
            .as_chunks::<{ page_format::ENTRY_METADATA_BYTES }>()
            .0
            .iter()
            .map(|record| decode_entry(record, address..scan_end))
            .collect::<Option<Vec<_>>>()?;
        let mut payload_start = address + metadata_storage_bytes(metadata.len())?;
        for (key, _, _, payload) in &entries {
            // Payloads follow the shared metadata prefix and each other, without overlap or gaps.
            if self.shard_index_for_key(key) != shard_index || payload.start != payload_start {
                return None;
            }
            payload_start = payload.end;
        }
        let end = payload_start.checked_next_multiple_of(CHUNK_BYTES)?;
        if end > scan_end {
            return None;
        }
        let chunk_count = (end - address) / CHUNK_BYTES;
        if chunk_count > 1 {
            if entries.len() != 1 {
                return None;
            }
            drop(region);
            // Any intervening write marks its chunks claimed, even if it releases them again.
            region = shard
                .allocator
                .reserve_for_recovery(address / CHUNK_BYTES, chunk_count)?;
        }
        for (key, object_range, checksum, payload) in entries {
            let storage = ObjectRangeDiskStorage {
                read_result: Weak::new(),
                eviction_position: 0,
                publication_id: 0,
                object_range,
                payload_checksum: checksum,
                payload_region: region.slice(payload.clone()),
            };
            {
                let mut index = shard.entry_index.lock().unwrap();
                // Indexed range ends increase with their starts. One predecessor detects any overlap,
                // without scanning a large object's entire index while blocking foreground lookups.
                if index.ranges_by_key.get(&key).is_some_and(|entries| {
                    entries
                        .range(..object_range.end())
                        .next_back()
                        .is_some_and(|(_, entry)| entry.object_range.end() > object_range.start())
                }) {
                    continue; // Never displace an already indexed entry.
                }
                index.insert(key, storage);
            }
            // Our reservations survive publication and even concurrent eviction. Mark before releasing
            // them, so this scan cannot resurrect the copy after its final owner releases it.
            region.mark_recovered();
        }
        Some(region.range().end)
    }

    /// Reads and validates packed entry records, all of which fit in the first chunk.
    async fn read_metadata(&self, region: &DiskRegion) -> Option<Vec<u8>> {
        let mut address = region.range().start;
        let mut page = self.file.read_recovery_page(region, address).await.ok()?;
        let checksum = read_u64(&page, 8)?;
        let entry_count = read_u64(&page, 32)?;
        // Shared entries each need at least one aligned payload; multi-chunk entries are exclusive.
        if entry_count == 0 || entry_count > CHUNK_BYTES / PAYLOAD_ALIGNMENT_BYTES {
            return None;
        }
        let metadata_bytes = entry_count as usize * page_format::ENTRY_METADATA_BYTES;
        let mut contents = Vec::with_capacity(metadata_bytes);
        for ordinal in 0..metadata_bytes.div_ceil(PAGE_CONTENT_BYTES) {
            if ordinal != 0 {
                address += METADATA_PAGE_BYTES as u64;
                page = self.file.read_recovery_page(region, address).await.ok()?;
            }
            let (count, bytes) = page_format::validate_page(
                &page,
                page_format::ENTRY_METADATA_PAGE_TAG,
                checksum,
                address,
                ordinal as u64,
            )?;
            let take = (metadata_bytes - contents.len()).min(PAGE_CONTENT_BYTES);
            contents.extend_from_slice(&bytes[..take]);
            if count != entry_count || bytes[take..].iter().any(|&byte| byte != 0) {
                return None;
            }
        }
        (XxHash64::oneshot(0, &contents) == checksum).then_some(contents)
    }
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
}

fn valid_allocation(range: &Range<u64>, bounds: Range<u64>) -> bool {
    range.start < range.end
        && range.start >= bounds.start + METADATA_PAGE_BYTES as u64
        && range.end <= bounds.end
        && range.start.is_multiple_of(PAYLOAD_ALIGNMENT_BYTES)
        && range.end.is_multiple_of(PAYLOAD_ALIGNMENT_BYTES)
}

type DecodedEntry = (ObjectKeyHash, ByteRange, u64, Range<u64>);

fn decode_entry(bytes: &[u8], bounds: Range<u64>) -> Option<DecodedEntry> {
    if bytes.len() != page_format::ENTRY_METADATA_BYTES {
        return None;
    }
    let key = ObjectKeyHash(u128::from_le_bytes(bytes[..16].try_into().ok()?));
    let object_start = read_u64(bytes, 16)?;
    let range = ByteRange::new(object_start, object_start.checked_add(read_u64(bytes, 24)?)?).ok()?;
    let checksum = read_u64(bytes, 40)?;
    let start = read_u64(bytes, 32)?;
    let aligned_length =
        range.len().checked_add(PAYLOAD_ALIGNMENT_BYTES - 1)? / PAYLOAD_ALIGNMENT_BYTES * PAYLOAD_ALIGNMENT_BYTES;
    let payload = start..start.checked_add(aligned_length)?;
    if !valid_allocation(&payload, bounds) {
        return None;
    }
    Some((key, range, checksum, payload))
}

#[cfg(test)]
pub(super) mod tests;
