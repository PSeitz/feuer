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

const BOUNDS_TAG: &[u8; 8] = b"FEUEND04";
const BOUNDS_FILE: &str = "recovery-ends";
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(10);
// Malformed metadata must not turn recovery into an unbounded allocation or chain walk.
// Larger metadata is legal to write but may be skipped by best-effort recovery.
const MAX_RECOVERY_METADATA_BYTES: usize = 16 * 1024 * 1024;

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
        bytes.extend_from_slice(blake3::hash(&bytes).as_bytes());
        bytes
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 80 || &bytes[..8] != BOUNDS_TAG {
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
        if bytes.len() != length + 32 || bytes[length..] != *blake3::hash(&bytes[..length]).as_bytes() {
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
                    for address in (start..end).step_by(CHUNK_BYTES as usize) {
                        {
                            let Some(disk) = weak.upgrade() else { return };
                            disk.recover_chunk(index, address, end).await;
                        }
                        // Do not retain the cache between chunks, or monopolize an executor on skipped chunks.
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

    async fn recover_chunk(&self, shard_index: usize, address: u64, scan_end: u64) -> Option<()> {
        let shard = &self.shards[shard_index];
        let chunk = shard.allocator.reserve_for_recovery(address / CHUNK_BYTES)?;
        let header = self.read_chunk_header(&chunk).await?;
        let mut occupied = Vec::<Range<u64>>::new();
        for bit in 1..256 {
            if header.bitmap[bit / 8] & (1 << (bit % 8)) == 0 {
                continue;
            }
            let head = address + (bit * METADATA_PAGE_BYTES) as u64;
            let Some(entry) = self
                .read_recovery_entry(shard_index, &chunk, &header, head, scan_end)
                .await
            else {
                continue;
            };
            let RecoveredEntry {
                key,
                object_range,
                checksum,
                payload,
                metadata,
                chunks,
            } = entry;
            // Shared-chunk candidates may not overlap one another's payload or metadata.
            if payload
                .iter()
                .chain(&metadata)
                .any(|range| occupied.iter().any(|used| overlaps(range, used)))
            {
                continue;
            }
            let slice = |range: &Range<u64>| {
                let start = range.start / CHUNK_BYTES * CHUNK_BYTES;
                if start == address {
                    chunk.slice(range.clone())
                } else {
                    chunks[&start].slice(range.clone())
                }
            };
            let storage = ObjectRangeDiskStorage {
                read_result: Weak::new(),
                eviction_position: 0,
                publication_id: 0,
                object_range,
                payload_checksum: checksum,
                payload_regions: payload.iter().map(slice).collect(),
                entry_metadata_regions: metadata.iter().map(slice).collect(),
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
            chunk.mark_recovered();
            for region in chunks.values() {
                region.mark_recovered();
            }
            occupied.extend(payload);
            occupied.extend(metadata);
        }
        Some(())
    }

    async fn read_chunk_header(&self, chunk: &DiskRegion) -> Option<ChunkHeader> {
        let address = chunk.range().start;
        let bytes = self.file.read_recovery_page(chunk, address).await.ok()?;
        let checksum = bytes[32..64].try_into().ok()?;
        let (next, contents) = page_format::validate_page(
            &bytes,
            page_format::CHUNK_METADATA_PAGE_TAG,
            &checksum,
            address,
            address / CHUNK_BYTES,
        )?;
        if next != 0
            || contents[0] & 1 != 0
            || contents[32..48] != self.recovery.generation
            || *blake3::hash(&contents[..page_format::CHUNK_METADATA_CONTENT_BYTES]).as_bytes() != checksum
            || contents[page_format::CHUNK_METADATA_CONTENT_BYTES..]
                .iter()
                .any(|&byte| byte != 0)
        {
            return None;
        }
        Some(ChunkHeader {
            bitmap: contents[..32].try_into().ok()?,
            batch_id: contents[48..64].try_into().ok()?,
        })
    }

    async fn read_recovery_entry(
        &self,
        shard_index: usize,
        head_chunk: &DiskRegion,
        header: &ChunkHeader,
        head: u64,
        scan_end: u64,
    ) -> Option<RecoveredEntry> {
        let shard = &self.shards[shard_index];
        let shard_start = shard_range(self.file.capacity(), self.shards.len(), shard_index).start;
        let mut chunks = BTreeMap::<u64, DiskRegion>::new();
        let mut metadata = Vec::new();
        let mut contents = Vec::new();
        let mut address = head;
        let mut checksum = None;
        let mut content_length = 0;
        loop {
            if !valid_allocation(
                &(address..address.checked_add(METADATA_PAGE_BYTES as u64)?),
                shard_start..scan_end,
            ) {
                return None;
            }
            let chunk_address = address / CHUNK_BYTES * CHUNK_BYTES;
            if chunk_address != head_chunk.range().start && !chunks.contains_key(&chunk_address) {
                let chunk = shard.allocator.reserve_for_recovery(chunk_address / CHUNK_BYTES)?;
                let other = self.read_chunk_header(&chunk).await?;
                if other.batch_id != header.batch_id || other.bitmap != [0; 32] {
                    return None;
                }
                chunks.insert(chunk_address, chunk);
            }
            let chunk = if chunk_address == head_chunk.range().start {
                head_chunk
            } else {
                &chunks[&chunk_address]
            };
            let page = self.file.read_recovery_page(chunk, address).await.ok()?;
            let checksum = checksum.get_or_insert(page[32..64].try_into().ok()?);
            let (next, bytes) = page_format::validate_page(
                &page,
                page_format::ENTRY_METADATA_PAGE_TAG,
                checksum,
                address,
                metadata.len() as u64,
            )?;
            if metadata.is_empty() {
                content_length = usize::try_from(integer(bytes, 0)?).ok()?;
                if !(88..=MAX_RECOVERY_METADATA_BYTES).contains(&content_length) {
                    return None;
                }
            }
            let take = (content_length - contents.len()).min(bytes.len());
            contents.extend_from_slice(&bytes[..take]);
            metadata.push(address..address + METADATA_PAGE_BYTES as u64);
            if contents.len() == content_length {
                if next != 0
                    || bytes[take..].iter().any(|&byte| byte != 0)
                    || blake3::hash(&contents).as_bytes() != checksum
                {
                    return None;
                }
                break;
            }
            if next == 0 {
                return None;
            }
            address = next;
        }
        let (key, object_range, checksum, payload) = decode_entry(&contents, &header.batch_id, shard_start..scan_end)?;
        if self.shard_index_for_key(&key) != shard_index {
            return None;
        }
        let mut ranges: Vec<_> = payload.iter().chain(&metadata).cloned().collect();
        ranges.sort_unstable_by_key(|range| range.start);
        if ranges.windows(2).any(|pair| overlaps(&pair[0], &pair[1])) {
            return None;
        }
        for range in &payload {
            let chunk_address = range.start / CHUNK_BYTES * CHUNK_BYTES;
            if chunk_address != head_chunk.range().start && !chunks.contains_key(&chunk_address) {
                let chunk = shard.allocator.reserve_for_recovery(chunk_address / CHUNK_BYTES)?;
                let other = self.read_chunk_header(&chunk).await?;
                if other.batch_id != header.batch_id || other.bitmap != [0; 32] {
                    return None;
                }
                chunks.insert(chunk_address, chunk);
            }
        }
        if !chunks.is_empty() && header.bitmap.iter().map(|byte| byte.count_ones()).sum::<u32>() != 1 {
            return None; // A multi-chunk entry owns every chunk exclusively.
        }
        Some(RecoveredEntry {
            key,
            object_range,
            checksum,
            payload,
            metadata,
            chunks,
        })
    }
}

struct ChunkHeader {
    bitmap: [u8; 32],
    batch_id: [u8; 16],
}

struct RecoveredEntry {
    key: ObjectKey,
    object_range: ByteRange,
    checksum: blake3::Hash,
    payload: Vec<Range<u64>>,
    metadata: Vec<Range<u64>>,
    chunks: BTreeMap<u64, DiskRegion>,
}

fn integer(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
}

fn overlaps(a: &Range<u64>, b: &Range<u64>) -> bool {
    a.start < b.end && b.start < a.end
}

fn valid_allocation(range: &Range<u64>, bounds: Range<u64>) -> bool {
    range.start < range.end
        && range.start >= bounds.start
        && range.end <= bounds.end
        && range.start.is_multiple_of(PAYLOAD_ALIGNMENT_BYTES)
        && range.end.is_multiple_of(PAYLOAD_ALIGNMENT_BYTES)
        && range.start % CHUNK_BYTES >= METADATA_PAGE_BYTES as u64
        && range.start / CHUNK_BYTES == (range.end - 1) / CHUNK_BYTES
}

type DecodedEntry = (ObjectKey, ByteRange, blake3::Hash, Vec<Range<u64>>);

fn decode_entry(bytes: &[u8], batch_id: &[u8; 16], bounds: Range<u64>) -> Option<DecodedEntry> {
    let length = usize::try_from(integer(bytes, 0)?).ok()?;
    let key_length = usize::try_from(integer(bytes, 8)?).ok()?;
    let range = ByteRange::new(integer(bytes, 16)?, integer(bytes, 24)?).ok()?;
    let count = usize::try_from(integer(bytes, 32)?).ok()?;
    if length != bytes.len()
        || count == 0
        || 88usize.checked_add(count.checked_mul(16)?)?.checked_add(key_length)? != length
        || bytes.get(length.checked_sub(16)?..)? != batch_id
    {
        return None;
    }
    let checksum = blake3::Hash::from_bytes(bytes.get(40..72)?.try_into().ok()?);
    let mut payload = Vec::new();
    let mut remaining =
        range.len().checked_add(PAYLOAD_ALIGNMENT_BYTES - 1)? / PAYLOAD_ALIGNMENT_BYTES * PAYLOAD_ALIGNMENT_BYTES;
    for index in 0..count {
        let allocation = integer(bytes, 72 + index * 16)?..integer(bytes, 80 + index * 16)?;
        if !valid_allocation(&allocation, bounds.clone()) {
            return None;
        }
        remaining = remaining.checked_sub(allocation.end - allocation.start)?;
        payload.push(allocation);
    }
    if remaining != 0 {
        return None;
    }
    let key = std::str::from_utf8(&bytes[72 + count * 16..length - 16])
        .ok()?
        .to_owned();
    Some((key, range, checksum, payload))
}

#[cfg(test)]
pub(super) mod tests;
