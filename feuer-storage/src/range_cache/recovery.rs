//! Best-effort, incremental recovery. New writes may claim any still-free chunk first.

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    ops::Range,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};

use super::*;

const BOUNDS_TAG: &[u8; 8] = b"FEUEND07";
const BOUNDS_FILE: &str = "recovery-ends";
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(10);

pub(super) struct RecoveryState {
    path: PathBuf,
    pub(super) generation: [u8; 16],
    session_id: [u8; 16],
    next_batch_number: AtomicU64,
    pub(super) running: AtomicBool,
}

struct RecoveryBounds {
    generation: [u8; 16],
    capacity: u64,
    ends: Vec<u64>,
}

fn random_identity() -> io::Result<[u8; 16]> {
    let mut bytes = [0; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub(super) fn shard_range(capacity: u64, count: usize, index: usize) -> Range<u64> {
    let chunks = capacity / CHUNK_BYTES;
    chunks * index as u64 / count as u64 * CHUNK_BYTES..chunks * (index + 1) as u64 / count as u64 * CHUNK_BYTES
}

impl RecoveryBounds {
    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(BOUNDS_TAG);
        bytes.extend_from_slice(&self.generation);
        bytes.extend_from_slice(&self.capacity.to_le_bytes());
        bytes.extend_from_slice(&(self.ends.len() as u64).to_le_bytes());
        for end in &self.ends {
            bytes.extend_from_slice(&end.to_le_bytes());
        }
        bytes.extend_from_slice(&XxHash64::oneshot(0, &bytes).to_le_bytes());
        bytes
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 56 || &bytes[..8] != BOUNDS_TAG {
            return None;
        }
        let capacity = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        let shards = usize::try_from(u64::from_le_bytes(bytes[32..40].try_into().unwrap())).ok()?;
        if !(1..=64).contains(&shards)
            || capacity == 0
            || capacity > i64::MAX as u64
            || !capacity.is_multiple_of(CHUNK_BYTES)
        {
            return None;
        }
        let length = 40 + 8 * shards;
        if bytes.len() != length + 8 || bytes[length..] != XxHash64::oneshot(0, &bytes[..length]).to_le_bytes() {
            return None;
        }
        let ends: Vec<_> = bytes[40..length]
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
        Some(Self {
            generation: bytes[8..24].try_into().unwrap(),
            capacity,
            ends,
        })
    }

    fn save(&self, path: &Path) -> io::Result<()> {
        let temporary = path.with_extension("tmp");
        let mut file = File::create(&temporary)?;
        file.write_all(&self.encode())?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        // Especially important for resets: old-generation chunks must not reappear after a crash.
        File::open(path.parent().unwrap())?.sync_all()
    }
}

impl RecoveryState {
    pub(super) fn open(directory: &Path, capacity: u64, shards: usize) -> io::Result<(Self, Vec<u64>)> {
        let path = directory.join(BOUNDS_FILE);
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
                        "discarding entire disk cache: incompatible recovery layout"
                    );
                } else {
                    tracing::info!(
                        target: "feuer::storage",
                        recovery_file = %path.display(),
                        "resetting disk cache: recovery ends missing, invalid, or incompatible"
                    );
                }
                let bounds = RecoveryBounds {
                    generation: random_identity()?,
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
                generation: bounds.generation,
                session_id: random_identity()?,
                next_batch_number: AtomicU64::new(0),
                running: AtomicBool::new(running),
            },
            bounds.ends,
        ))
    }

    pub(super) fn next_batch_id(&self) -> [u8; 16] {
        let ordinal = self.next_batch_number.fetch_add(1, Ordering::Relaxed);
        let mut hasher = blake3::Hasher::new();
        hasher.update(&self.session_id);
        hasher.update(&ordinal.to_le_bytes());
        hasher.finalize().as_bytes()[..16].try_into().unwrap()
    }
}

impl DiskRangeCacheState {
    fn recovery_bounds(&self) -> RecoveryBounds {
        RecoveryBounds {
            generation: self.recovery.generation,
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

    pub(super) fn start_background_tasks(self: &Arc<Self>, ends: Vec<u64>) {
        let mut saved_ends = ends.clone();
        if self.recovery.running.load(Ordering::Relaxed) {
            let weak = Arc::downgrade(self);
            let capacity = self.file.capacity();
            let started = Instant::now();
            tracing::info!(
                target: "feuer::storage",
                recovery_file = %self.recovery.path.display(),
                shard_count = ends.len(),
                capacity_bytes = capacity,
                "starting disk cache recovery"
            );
            tokio::spawn(async move {
                for (index, &end) in ends.iter().enumerate() {
                    let range = shard_range(capacity, ends.len(), index);
                    let start = range.start;
                    tracing::debug!(
                        target: "feuer::storage",
                        shard_index = index,
                        shard_capacity_bytes = range.end - start,
                        scan_start = start,
                        scan_end = end,
                        scan_chunks = (end - start) / CHUNK_BYTES,
                        "starting disk shard recovery scan"
                    );
                    let mut address = start;
                    while address < end {
                        {
                            let Some(disk) = weak.upgrade() else { return };
                            address = disk
                                .recover_chunk(index, address, end)
                                .await
                                .unwrap_or(address + CHUNK_BYTES);
                        }
                        // Do not retain the cache between attempts, or monopolize an executor on skipped chunks.
                        tokio::task::yield_now().await;
                    }
                    let Some(disk) = weak.upgrade() else { return };
                    disk.shards[index].allocator.finish_recovery();
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
                if bounds.ends == saved_ends {
                    continue;
                }
                // Retain the directory lock until the atomic replacement completes, even on cancellation.
                match tokio::task::spawn_blocking(move || {
                    bounds.save(&disk.recovery.path)?;
                    Ok::<_, io::Error>(bounds.ends)
                })
                .await
                {
                    Ok(Ok(ends)) => saved_ends = ends,
                    result => tracing::warn!(target: "feuer::storage", ?result, "could not save disk recovery ends"),
                }
            }
        });
    }

    /// Returns the allocation's end so the scan skips its continuation chunks.
    async fn recover_chunk(&self, shard_index: usize, address: u64, scan_end: u64) -> Option<u64> {
        let shard = &self.shards[shard_index];
        let mut region = shard.allocator.reserve_for_recovery(address / CHUNK_BYTES, 1)?;
        let header = self.read_chunk_header(&region).await?;
        if address.checked_add(header.chunk_count.checked_mul(CHUNK_BYTES)?)? > scan_end {
            return None;
        }
        if header.chunk_count > 1 {
            drop(region);
            // Any intervening write marks its chunks claimed, even if it releases them again.
            region = shard
                .allocator
                .reserve_for_recovery(address / CHUNK_BYTES, header.chunk_count)?;
        }
        let metadata = self.read_metadata(&region, &header).await?;
        let mut records = metadata.as_slice();
        let mut payload_start = address + METADATA_PAGE_BYTES as u64 + metadata_storage_bytes(metadata.len())?;
        while !records.is_empty() {
            let length = usize::try_from(read_u64(records, 0)?).ok()?;
            let (key, object_range, checksum, payload) =
                decode_entry(records.get(..length)?, &header.batch_id, region.range())?;
            // Payloads follow the shared metadata prefix and each other, without overlap or gaps.
            if self.shard_index_for_key(&key) != shard_index
                || payload.start != payload_start
                || (header.chunk_count > 1
                    && (length != metadata.len() || payload.end.next_multiple_of(CHUNK_BYTES) != region.range().end))
            {
                return None;
            }
            records = &records[length..];
            payload_start = payload.end;
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

    async fn read_chunk_header(&self, chunk: &DiskRegion) -> Option<ChunkHeader> {
        let address = chunk.range().start;
        let bytes = self.file.read_recovery_page(chunk, address).await.ok()?;
        let checksum = read_u64(&bytes, 8)?;
        let (next, contents) = page_format::validate_page(
            &bytes,
            page_format::CHUNK_METADATA_PAGE_TAG,
            checksum,
            address,
            address / CHUNK_BYTES,
        )?;
        if next != 0
            || contents[..16] != self.recovery.generation
            || XxHash64::oneshot(0, &contents[..page_format::CHUNK_METADATA_CONTENT_BYTES]) != checksum
            || contents[page_format::CHUNK_METADATA_CONTENT_BYTES..]
                .iter()
                .any(|&byte| byte != 0)
        {
            return None;
        }
        let chunk_count = read_u64(contents, 32)?;
        let metadata_bytes = usize::try_from(read_u64(contents, 40)?).ok()?;
        let prefix_bytes = (METADATA_PAGE_BYTES as u64).checked_add(metadata_storage_bytes(metadata_bytes)?)?;
        if chunk_count == 0 || metadata_bytes < 72 || prefix_bytes >= chunk_count.checked_mul(CHUNK_BYTES)? {
            return None;
        }
        Some(ChunkHeader {
            batch_id: contents[16..32].try_into().ok()?,
            chunk_count,
            metadata_bytes,
        })
    }

    /// Reads and validates the allocation's packed records before publishing any entries.
    async fn read_metadata(&self, region: &DiskRegion, header: &ChunkHeader) -> Option<Vec<u8>> {
        let mut contents = Vec::new();
        let mut checksum = None;
        for ordinal in 0..header.metadata_bytes.div_ceil(PAGE_CONTENT_BYTES) {
            let address = region.range().start + ((ordinal + 1) * METADATA_PAGE_BYTES) as u64;
            let page = self.file.read_recovery_page(region, address).await.ok()?;
            let checksum = *checksum.get_or_insert(read_u64(&page, 8)?);
            let (next, bytes) = page_format::validate_page(
                &page,
                page_format::ENTRY_METADATA_PAGE_TAG,
                checksum,
                address,
                ordinal as u64,
            )?;
            let take = (header.metadata_bytes - contents.len()).min(bytes.len());
            contents.extend_from_slice(&bytes[..take]);
            let expected_next = if contents.len() == header.metadata_bytes {
                0
            } else {
                address + METADATA_PAGE_BYTES as u64
            };
            if next != expected_next || bytes[take..].iter().any(|&byte| byte != 0) {
                return None;
            }
        }
        (XxHash64::oneshot(0, &contents) == checksum?).then_some(contents)
    }
}

struct ChunkHeader {
    batch_id: [u8; 16],
    chunk_count: u64,
    metadata_bytes: usize,
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

type DecodedEntry = (ObjectKey, ByteRange, u64, Range<u64>);

fn decode_entry(bytes: &[u8], batch_id: &[u8; 16], bounds: Range<u64>) -> Option<DecodedEntry> {
    let length = usize::try_from(read_u64(bytes, 0)?).ok()?;
    let key_length = usize::try_from(read_u64(bytes, 8)?).ok()?;
    let range = ByteRange::new(read_u64(bytes, 16)?, read_u64(bytes, 24)?).ok()?;
    if length != bytes.len()
        || 72usize.checked_add(key_length)? != length
        || bytes.get(length.checked_sub(16)?..)? != batch_id
    {
        return None;
    }
    let checksum = read_u64(bytes, 48)?;
    let start = read_u64(bytes, 32)?;
    let payload_length = read_u64(bytes, 40)?;
    let payload = start..start.checked_add(payload_length)?;
    let aligned_length =
        range.len().checked_add(PAYLOAD_ALIGNMENT_BYTES - 1)? / PAYLOAD_ALIGNMENT_BYTES * PAYLOAD_ALIGNMENT_BYTES;
    if payload_length != aligned_length || !valid_allocation(&payload, bounds) {
        return None;
    }
    let key = std::str::from_utf8(&bytes[56..length - 16]).ok()?.to_owned();
    Some((key, range, checksum, payload))
}

#[cfg(test)]
pub(super) mod tests;
