//! Open generated disk images, then insert, remove, write metadata, and reopen with changed capacities
//! or only some metadata writes persisted. Images include invalid links, tags, checksums, and entry fields.
//! Check chunk ownership, metadata chains, agreement between entries and metadata, and read lengths.
//! Published writes must read back their bytes. Payload bytes must not be recovered as metadata.

use std::{collections::BTreeMap, ops::Range, path::Path, time::Duration};

use proptest::prelude::*;

use super::{
    metadata::EntryMetadata,
    page_format::*,
    recovery::shard_disk_range,
    tests::{download, with_manual_metadata_writes},
    *,
};

const PAGE_BYTES: u64 = METADATA_PAGE_BYTES as u64;
/// Upper 64 key bits of entry metadata forged in payload bytes. These keys must never become entries.
const FORGED_KEY_PREFIX: u128 = 0xFA4E_FA4E_FA4E_FA4E;

/// A byte change to an encoded page.
#[derive(Clone, Debug)]
enum PageChange {
    None,
    /// Replaces the tag and computes a matching checksum. The replacement may match the original tag.
    ReplaceTag([u8; 8]),
    FlippedChecksum,
}

/// The target of a next-chunk link.
#[derive(Clone, Debug)]
enum LinkTarget {
    /// The image's metadata chunk at this index, modulo their count.
    MetadataChunkIndex(usize),
    DiskAddress(u64),
    NoChunk,
}

/// One entry's metadata fields, written without validation so they can overflow.
#[derive(Clone, Debug)]
struct EntryFields {
    index_in_page: usize,
    key: u128,
    start: u64,
    length: u64,
    payload_address: u64,
    /// When absent, hash the existing payload bytes if readable and at most four chunks, otherwise use zero.
    payload_checksum: Option<u64>,
}

/// Entry metadata and byte changes for one page in an image.
#[derive(Clone, Debug)]
struct EntryPageImage {
    page_index: usize,
    entries: Vec<EntryFields>,
    change: PageChange,
}

/// One metadata chunk's entry pages and optional next-chunk page.
#[derive(Clone, Debug)]
struct MetadataChunkImage {
    chunk_number: u64,
    entry_pages: Vec<EntryPageImage>,
    link: Option<(LinkTarget, PageChange)>,
}

/// File contents written before the cache opens.
#[derive(Clone, Debug)]
struct DiskImage {
    capacity_chunks: u64,
    metadata_chunks: Vec<MetadataChunkImage>,
}

/// A change to cache entries, metadata, or capacity.
#[derive(Clone, Debug)]
enum CacheChange {
    Insert {
        key: u128,
        start: u64,
        length: usize,
    },
    /// Inserts a 1-MiB object whose bytes are valid metadata pages with forged entries.
    InsertPayloadWithMetadata {
        key: u128,
        next_chunk_number: u64,
    },
    Remove {
        entry_index: usize,
    },
    /// Attempts to reserve one more metadata chunk without writing its entry pages.
    ReserveMetadataChunk {
        shard_index: usize,
    },
    WriteDirtyMetadataPages,
    Reopen {
        capacity_chunks: u64,
    },
    /// Writes a pseudorandom subset of dirty metadata pages, then reopens without the rest.
    ReopenAfterPartialMetadataWrite {
        persisted_page_seed: u64,
    },
}

/// Chunk numbers near the file start, the two-shard boundary at chunk 128, and the end of a 256-chunk file.
fn chunk_number() -> impl Strategy<Value = u64> {
    prop_oneof![0..10u64, 126..131u64, 254..258u64]
}

/// One shard of 3 or 8 chunks, or two shards. Between 256 and 258 chunks, shard 1's head moves from chunk 128 to 129.
fn capacity_chunks() -> impl Strategy<Value = u64> {
    prop_oneof![Just(3u64), Just(8), Just(256), Just(258)]
}

/// Disk byte addresses within the selected chunks, optionally including addresses throughout `u64`.
fn disk_address(allow_overflow: bool) -> BoxedStrategy<u64> {
    let page = prop_oneof![Just(0u64), Just(1), Just(255), 0..256u64];
    let chunk_relative = prop_oneof![
        (chunk_number(), page).prop_map(|(chunk, page)| chunk * CHUNK_BYTES + page * PAGE_BYTES),
        (chunk_number(), 0..CHUNK_BYTES).prop_map(|(chunk, offset)| chunk * CHUNK_BYTES + offset),
    ];
    if !allow_overflow {
        return chunk_relative.boxed();
    }
    prop_oneof![
        1 => chunk_relative,
        1 => prop_oneof![
            Just(u64::MAX),
            Just(u64::MAX - (PAGE_BYTES - 1)),
            Just(u64::MAX / CHUNK_BYTES * CHUNK_BYTES),
            any::<u64>(),
        ],
    ]
    .boxed()
}

/// An object's byte range start and length. Unless overflow is allowed, their sum fits in `u64`.
fn object_start_and_length(allow_overflow: bool) -> BoxedStrategy<(u64, u64)> {
    let start = prop_oneof![0..9000u64, Just(u64::MAX), u64::MAX - 9000..=u64::MAX, any::<u64>()];
    let length = prop_oneof![
        0..9000u64,
        CHUNK_BYTES - PAGE_BYTES..=CHUNK_BYTES + PAGE_BYTES,
        2 * CHUNK_BYTES..=3 * CHUNK_BYTES + 1,
        Just(1 << 40),
    ];
    if !allow_overflow {
        return (start, length)
            .prop_map(|(start, length)| (start.min(u64::MAX - length), length))
            .boxed();
    }
    let length = prop_oneof![
        1 => length,
        1 => prop_oneof![Just(u64::MAX), Just(u64::MAX - (PAGE_BYTES - 1)), Just(u64::MAX - PAGE_BYTES)],
    ];
    (start, length).boxed()
}

fn entry_fields(allow_overflow: bool) -> impl Strategy<Value = EntryFields> {
    (
        0..ENTRIES_PER_METADATA_PAGE,
        prop_oneof![0..6u128, any::<u128>()],
        object_start_and_length(allow_overflow),
        disk_address(allow_overflow),
        prop::option::weighted(0.3, any::<u64>()),
    )
        .prop_map(
            |(index_in_page, key, (start, length), payload_address, payload_checksum)| EntryFields {
                index_in_page,
                key,
                start,
                length,
                payload_address,
                payload_checksum,
            },
        )
}

fn page_change() -> impl Strategy<Value = PageChange> {
    prop_oneof![
        6 => Just(PageChange::None),
        1 => prop_oneof![
            Just(*NEXT_CHUNK_PAGE_TAG),
            Just(*b"FEUDES12"),
            Just(*b"FEUNXT12"),
            Just([0; 8]),
        ]
        .prop_map(PageChange::ReplaceTag),
        1 => Just(PageChange::FlippedChecksum),
    ]
}

fn disk_image(allow_overflow: bool) -> impl Strategy<Value = DiskImage> {
    let page_index = prop_oneof![Just(0usize), Just(1), Just(254), 0..ENTRY_METADATA_PAGES_PER_CHUNK];
    let page = (
        page_index,
        prop::collection::vec(entry_fields(allow_overflow), 0..6),
        page_change(),
    )
        .prop_map(|(page_index, entries, change)| EntryPageImage {
            page_index,
            entries,
            change,
        });
    let link_target = prop_oneof![
        2 => (0..6usize).prop_map(LinkTarget::MetadataChunkIndex),
        1 => disk_address(allow_overflow).prop_map(LinkTarget::DiskAddress),
        1 => Just(LinkTarget::NoChunk),
    ];
    let chunk = (
        prop_oneof![chunk_number(), Just(128)],
        prop::collection::vec(page, 0..4),
        prop::option::weighted(0.9, (link_target, page_change())),
    )
        .prop_map(|(chunk_number, entry_pages, link)| MetadataChunkImage {
            chunk_number,
            entry_pages,
            link,
        });
    (capacity_chunks(), prop::collection::vec(chunk, 1..5)).prop_map(|(capacity_chunks, mut metadata_chunks)| {
        // Start shard 0's metadata chain at the chunk recovery reads first.
        metadata_chunks[0].chunk_number = 0;
        DiskImage {
            capacity_chunks,
            metadata_chunks,
        }
    })
}

fn cache_changes(include_metadata_payloads: bool) -> impl Strategy<Value = Vec<CacheChange>> {
    let length = prop_oneof![0..9000usize, 512 * 1024..=CHUNK_BYTES as usize + 9000];
    let insert = (
        0..6u128,
        prop_oneof![Just(0u64), Just(50), Just(100), Just(5000)],
        length,
    )
        .prop_map(|(key, start, length)| CacheChange::Insert { key, start, length });
    let metadata_payload = (0..6u128, chunk_number())
        .prop_map(|(key, next_chunk_number)| CacheChange::InsertPayloadWithMetadata { key, next_chunk_number });
    let metadata_payload_weight = if include_metadata_payloads { 3 } else { 0 };
    let change = prop_oneof![
        3 => insert,
        metadata_payload_weight => metadata_payload,
        3 => any::<usize>().prop_map(|entry_index| CacheChange::Remove { entry_index }),
        2 => any::<usize>().prop_map(|shard_index| CacheChange::ReserveMetadataChunk { shard_index }),
        1 => Just(CacheChange::WriteDirtyMetadataPages),
        2 => capacity_chunks().prop_map(|capacity_chunks| CacheChange::Reopen { capacity_chunks }),
        1 => any::<u64>().prop_map(|persisted_page_seed| CacheChange::ReopenAfterPartialMetadataWrite { persisted_page_seed }),
    ];
    prop::collection::vec(change, 0..16)
}

proptest! {
    #[test]
    fn malformed_metadata_preserves_chunk_ownership(
        image in disk_image(false),
        capacity_chunks in capacity_chunks(),
        changes in cache_changes(false),
    ) {
        check_disk_image(image, capacity_chunks, changes);
    }

    #[test]
    fn overflowing_fields_preserve_chunk_ownership(image in disk_image(true), capacity_chunks in capacity_chunks()) {
        check_disk_image(image, capacity_chunks, Vec::new());
    }

    #[test]
    fn payload_bytes_never_recover_as_metadata(
        capacity_chunks in capacity_chunks(),
        changes in cache_changes(true),
    ) {
        let image = DiskImage { capacity_chunks, metadata_chunks: Vec::new() };
        check_disk_image(image, capacity_chunks, changes);
    }
}

/// Checks an image before and after each generated cache change. Proptest shrinks assertion failures.
fn check_disk_image(image: DiskImage, capacity_chunks: u64, changes: Vec<CacheChange>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(120),
            check_cache_changes(image, capacity_chunks, changes),
        )
        .await
        .expect("cache I/O timed out");
    });
}

async fn check_cache_changes(image: DiskImage, capacity_chunks: u64, changes: Vec<CacheChange>) {
    let directory = tempfile::tempdir().unwrap();
    let directory = directory.path();
    write_image(directory, &image).await;
    let mut cache = open_with_manual_metadata_writes(directory, capacity_chunks).await;
    check_cache(&cache).await;
    for change in changes {
        match change {
            CacheChange::Insert { key, start, length } => {
                insert_and_check_bytes(&cache, ObjectKeyHash(key), download(start, length)).await;
            }
            CacheChange::InsertPayloadWithMetadata { key, next_chunk_number } => {
                let bytes = payload_with_metadata(next_chunk_number);
                insert_and_check_bytes(&cache, ObjectKeyHash(key), Download::new(0, bytes).unwrap()).await;
            }
            CacheChange::Remove { entry_index } => {
                let mut entries = entry_ranges(&cache);
                entries.sort_by_key(|(key, range)| (*key, range.start()));
                if let Some((key, range)) = entries.get(entry_index % entries.len().max(1)) {
                    cache.disk.shards[cache.disk.shard_index_for_key(key)].remove_entry(key, range.start());
                }
            }
            CacheChange::ReserveMetadataChunk { shard_index } => {
                let shard = &cache.disk.shards[shard_index % cache.disk.shards.len()];
                let mut metadata = shard.metadata_pages.lock().unwrap();
                let count = metadata.free_slot_count() + 1;
                let _ = metadata.ensure_free_slots(count, &shard.allocator);
            }
            CacheChange::WriteDirtyMetadataPages => cache.write_dirty_metadata_pages().await,
            CacheChange::Reopen { capacity_chunks } => {
                cache.write_dirty_metadata_pages().await;
                drop(cache);
                cache = open_with_manual_metadata_writes(directory, capacity_chunks).await;
            }
            CacheChange::ReopenAfterPartialMetadataWrite { persisted_page_seed } => {
                for shard in &cache.disk.shards {
                    shard.metadata_pages.lock().unwrap().dirty_pages.retain(|page| {
                        let page_number = u64::from(page.chunk_index) << 8 | u64::from(page.page_index);
                        (page_number ^ persisted_page_seed).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 63 == 0
                    });
                }
                cache.write_dirty_metadata_pages().await;
                let capacity_chunks = cache.disk.file.capacity() / CHUNK_BYTES;
                drop(cache);
                cache = open_with_manual_metadata_writes(directory, capacity_chunks).await;
            }
        }
        check_cache(&cache).await;
    }
}

async fn open_with_manual_metadata_writes(directory: &Path, capacity_chunks: u64) -> DiskCache {
    let cache = DiskCache::open(directory, capacity_chunks * CHUNK_BYTES, DiskCacheOptions::default())
        .await
        .unwrap();
    with_manual_metadata_writes(cache)
}

async fn insert_and_check_bytes(cache: &DiskCache, key: ObjectKeyHash, download: Download) {
    let range = download.downloaded_range();
    let bytes = download.bytes().clone();
    if cache.insert(key, download).await.unwrap() {
        assert_eq!(
            cache.get(&key, range).await,
            Some(bytes),
            "published entry did not read back"
        );
    }
}

/// Writes the image into a fresh file through an empty cache.
async fn write_image(directory: &Path, image: &DiskImage) {
    let cache = DiskCache::open(
        directory,
        image.capacity_chunks * CHUNK_BYTES,
        DiskCacheOptions::default(),
    )
    .await
    .unwrap();
    let file = &cache.disk.file;
    let fits = |address: u64, length: u64| address.checked_add(length).is_some_and(|end| end <= file.capacity());
    let mut pages = Vec::new();
    for chunk in &image.metadata_chunks {
        let chunk_address = chunk.chunk_number * CHUNK_BYTES;
        for page in &chunk.entry_pages {
            let mut contents = [0; PAGE_CONTENT_BYTES];
            for entry in &page.entries {
                let checksum = match entry.payload_checksum {
                    Some(checksum) => checksum,
                    None if entry.length <= 4 * CHUNK_BYTES && fits(entry.payload_address, entry.length) => {
                        let bytes = file
                            .read_at(entry.payload_address, entry.length as usize)
                            .await
                            .unwrap();
                        XxHash64::oneshot(0, &bytes)
                    }
                    None => 0,
                };
                let raw = &mut contents[entry.index_in_page * ENTRY_METADATA_BYTES..][..ENTRY_METADATA_BYTES];
                raw[..16].copy_from_slice(&entry.key.to_le_bytes());
                raw[16..24].copy_from_slice(&entry.start.to_le_bytes());
                raw[24..32].copy_from_slice(&entry.length.to_le_bytes());
                raw[32..40].copy_from_slice(&entry.payload_address.to_le_bytes());
                raw[40..48].copy_from_slice(&checksum.to_le_bytes());
            }
            let bytes = encode_changed_page(
                ENTRY_METADATA_PAGE_TAG,
                ENTRIES_PER_METADATA_PAGE as u64,
                &contents,
                &page.change,
            );
            pages.push((chunk_address + page.page_index as u64 * PAGE_BYTES, bytes));
        }
        if let Some((target, change)) = &chunk.link {
            let address = match *target {
                LinkTarget::MetadataChunkIndex(index) => {
                    image.metadata_chunks[index % image.metadata_chunks.len()].chunk_number * CHUNK_BYTES
                }
                LinkTarget::DiskAddress(address) => address,
                LinkTarget::NoChunk => NO_CHUNK,
            };
            let bytes = encode_changed_page(NEXT_CHUNK_PAGE_TAG, 1, &address.to_le_bytes(), change);
            let link_page = ENTRY_METADATA_PAGES_PER_CHUNK as u64 * PAGE_BYTES;
            pages.push((chunk_address + link_page, bytes));
        }
    }
    for (address, bytes) in pages {
        if fits(address, PAGE_BYTES) {
            file.write_at(address, &Bytes::from(bytes)).await.unwrap();
        }
    }
}

fn encode_changed_page(tag: &[u8; 8], entry_count: u64, contents: &[u8], change: &PageChange) -> Vec<u8> {
    let mut page = vec![0; METADATA_PAGE_BYTES];
    let tag = match change {
        PageChange::ReplaceTag(tag) => tag,
        _ => tag,
    };
    encode_page(&mut page, tag, entry_count, contents);
    if matches!(change, PageChange::FlippedChecksum) {
        page[0] ^= 1;
    }
    page
}

/// One chunk of payload bytes containing valid metadata pages. The forged entries claim every chunk
/// of a 256-chunk file, with keys for both shards. The link page points to `next_chunk_number`.
fn payload_with_metadata(next_chunk_number: u64) -> Bytes {
    let mut bytes = vec![0; CHUNK_BYTES as usize];
    for (page_index, page) in bytes.chunks_mut(METADATA_PAGE_BYTES).enumerate() {
        if page_index == ENTRY_METADATA_PAGES_PER_CHUNK {
            encode_page(
                page,
                NEXT_CHUNK_PAGE_TAG,
                1,
                &(next_chunk_number * CHUNK_BYTES).to_le_bytes(),
            );
            continue;
        }
        let mut contents = [0; PAGE_CONTENT_BYTES];
        for (entry_index, raw) in contents
            .as_chunks_mut::<ENTRY_METADATA_BYTES>()
            .0
            .iter_mut()
            .enumerate()
        {
            let entry_number = (page_index * ENTRIES_PER_METADATA_PAGE + entry_index) as u64;
            let entry = EntryMetadata {
                key: ObjectKeyHash(FORGED_KEY_PREFIX << 64 | entry_number as u128),
                object_range: ByteRange::new(0, PAGE_BYTES).unwrap(),
                payload_address: entry_number % 256 * CHUNK_BYTES + entry_number / 256 % 4 * PAGE_BYTES,
                payload_checksum: 0,
            };
            encode_entry_metadata(raw, &entry);
        }
        encode_page(
            page,
            ENTRY_METADATA_PAGE_TAG,
            ENTRIES_PER_METADATA_PAGE as u64,
            &contents,
        );
    }
    Bytes::from(bytes)
}

/// Every indexed entry's key and object byte range.
fn entry_ranges(cache: &DiskCache) -> Vec<(ObjectKeyHash, ByteRange)> {
    let mut ranges = Vec::new();
    for shard in &cache.disk.shards {
        let disk_index = shard.entry_index.lock().unwrap();
        ranges.extend(
            disk_index
                .entries_by_key
                .iter()
                .flat_map(|(key, entries)| entries.values().map(|entry| (*key, entry.object_range))),
        );
    }
    ranges
}

async fn check_cache(cache: &DiskCache) {
    check_entries_and_chunks(cache);
    // Checksum misses remove entries, so check ownership both before and after reading.
    for (key, range) in entry_ranges(cache) {
        if let Some(bytes) = cache.get(&key, range).await {
            assert_eq!(bytes.len() as u64, range.len());
        }
    }
    check_entries_and_chunks(cache);
}

/// Checks chunk ownership, metadata chains, entry metadata, and payload counts.
fn check_entries_and_chunks(cache: &DiskCache) {
    let disk = &cache.disk;
    for (shard_index, shard) in disk.shards.iter().enumerate() {
        let shard_range = shard_disk_range(disk.file.capacity(), disk.shards.len(), shard_index);
        let shard_chunks = shard_range.start / CHUNK_BYTES..shard_range.end / CHUNK_BYTES;
        let disk_index = shard.entry_index.lock().unwrap();
        let metadata = shard.metadata_pages.lock().unwrap();
        let ownership = shard.allocator.free_and_payload_chunks();
        let payload_reservations: BTreeMap<u64, (Range<u64>, usize)> = ownership
            .iter()
            .filter_map(|(chunks, count)| count.map(|count| (chunks.start, (chunks.clone(), count))))
            .collect();
        let mut owners: Vec<(Range<u64>, &str)> = ownership
            .into_iter()
            .map(|(chunks, count)| (chunks, if count.is_some() { "payload" } else { "free" }))
            .collect();
        let chunk_numbers = |range: Range<u64>| range.start / CHUNK_BYTES..range.end / CHUNK_BYTES;
        owners.extend(
            metadata
                .chunks
                .iter()
                .map(|chunk| (chunk_numbers(chunk.reserved_chunk.disk_byte_range()), "metadata")),
        );
        owners.sort_by_key(|(chunks, _)| chunks.start);
        let mut next_chunk = shard_chunks.start;
        for (chunks, _) in &owners {
            assert!(
                chunks.start == next_chunk && chunks.start < chunks.end,
                "shard {} chunk owners overlap or leave a gap: {:?}",
                shard_index,
                owners
            );
            next_chunk = chunks.end;
        }
        assert_eq!(
            next_chunk, shard_chunks.end,
            "shard {} owners: {:?}",
            shard_index, owners
        );

        if let Some(head) = metadata.chunks.first() {
            assert_eq!(
                head.reserved_chunk.disk_byte_range().start,
                shard_range.start,
                "the metadata chain does not start at the shard's first chunk, so recovery cannot find it"
            );
        }
        for (index, chunk) in metadata.chunks.iter().enumerate() {
            let next = metadata.chunks.get(index + 1);
            let expected = next.map_or(NO_CHUNK, |next| next.reserved_chunk.disk_byte_range().start);
            assert_eq!(
                chunk.next_chunk_address, expected,
                "metadata chunk {} links to an address other than the next reserved metadata chunk",
                index
            );
        }

        let mut payload_counts = BTreeMap::<u64, usize>::new();
        let mut entry_count = 0;
        for (key, entries) in &disk_index.entries_by_key {
            assert_ne!(
                key.0 >> 64,
                FORGED_KEY_PREFIX,
                "payload bytes were recovered as metadata"
            );
            assert_eq!(disk.shard_index_for_key(key), shard_index);
            let mut previous_end = None;
            for entry in entries.values() {
                entry_count += 1;
                let range = entry.object_range;
                assert!(
                    previous_end.is_none_or(|end| end < range.end()),
                    "object ranges contain one another"
                );
                previous_end = Some(range.end());
                let first_chunk = entry.payload_address / CHUNK_BYTES;
                let storage_end = range
                    .len()
                    .max(1)
                    .checked_next_multiple_of(PAGE_BYTES)
                    .and_then(|length| entry.payload_address.checked_add(length));
                let reservation = payload_reservations.get(&first_chunk);
                assert!(
                    reservation
                        .is_some_and(|(chunks, _)| storage_end.is_some_and(|end| end <= chunks.end * CHUNK_BYTES)),
                    "payload {:#x}+{} lies outside its reservation {:?}",
                    entry.payload_address,
                    range.len(),
                    reservation
                );
                *payload_counts.entry(first_chunk).or_default() += 1;
                let location = entry.metadata;
                let entry_metadata = metadata
                    .chunks
                    .get(location.chunk_index as usize)
                    .and_then(|chunk| chunk.slots.get(location.entry_index as usize))
                    .copied()
                    .flatten();
                let expected = EntryMetadata {
                    key: *key,
                    object_range: range,
                    payload_address: entry.payload_address,
                    payload_checksum: entry.payload_checksum,
                };
                assert_eq!(entry_metadata, Some(expected));
            }
        }
        let reservation_counts: BTreeMap<u64, usize> = payload_reservations
            .iter()
            .map(|(&first_chunk, &(_, count))| (first_chunk, count))
            .collect();
        assert_eq!(payload_counts, reservation_counts);
        let metadata_entry_count = metadata.chunks.iter().flat_map(|chunk| &chunk.slots).flatten().count();
        assert_eq!(metadata_entry_count, entry_count);
        assert_eq!(
            metadata.free_slot_count(),
            metadata.chunks.len() * ENTRIES_PER_METADATA_CHUNK - metadata_entry_count
        );
    }
}
