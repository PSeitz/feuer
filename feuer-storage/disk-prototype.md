# Disk range-cache prototype

Experimental `DiskRangeCache`, connected to public tiered lookup and bounded background disk writes.
[tiered-plan.md](../tiered-plan.md) remains authoritative.

**Open starts background recovery without waiting for the scan.** Recovered entries become readable
incrementally while ordinary reads and writes continue.

## Layout and ownership

- The file contains 1-MiB chunks. Each allocation reserves one consecutive run of whole chunks
  and starts with one 4-KiB metadata page. Continuation chunks have no headers.
- Fixed 48-byte entry records, with 128-bit key hashes, are packed into shared 4-KiB metadata pages
  immediately after the allocation header. Records never cross page boundaries.
- All payloads follow this metadata prefix. Each payload is one uninterrupted, 4-KiB-aligned disk
  byte range; no metadata is inserted between payloads or at payload chunk boundaries.
- Small entries share a chunk only when each entry's complete payload and metadata fit inside it.
  Multi-chunk entries own all their chunks exclusively, including unused tails.
- Written chunks are immutable. The entire allocation becomes reusable only after all entry owners and
  `DiskRegionReadGuard`s release it. Individual holes and individual chunks of a live allocation are never reused.

`src/allocation.rs` tracks free chunks as coalesced runs. `DiskRegion` subranges share ownership
of the entire contiguous allocation. Scattered free chunks are never combined for an entry.
Allocation and the range index are sharded by full-key hash.
Free capacity in another shard cannot satisfy an admission. No metadata lock is held across I/O.

## Writes

`insert_batch` accepts explicit `(ObjectKey, Download)` pairs and returns the number published.
Within each shard, entries are sorted smallest first and packed into complete chunk buffers.
Payload addresses, packed metadata, and zero padding are finalized before each chunk's single write.
Growing the metadata prefix during packing shifts payload addresses without copying retained payload bytes.
Partially filled chunks are written too. Later batches cannot append. The public tier supplies bounded
batches from its background worker. Storage itself adds no batching delay.

All writes for a shard finish before publication. Publication rechecks containment, larger entries
first: broader entries replace contained entries, while partial overlaps coexist. Contained entries
and entries that cannot fit are skipped. Publication is not transactional across shards.
`insert_batch_checked` additionally retains caller tokens through detached I/O and invokes a synchronous
publication check. The public tier uses this to hold the memory shard lock while validating the original
admission identity and publishing, so stale writes cannot become visible.

Each queued write retains its chunk ownership through completion despite caller cancellation.
Failed or abandoned batches release their chunks once all owners and in-flight I/O release them.
An abnormal queue failure retains active I/O resources when completion cannot be established.

Callers must bound batch size and concurrency. Complete chunk buffers are outside the raw I/O
queue's memory budget and are copied again into aligned direct-I/O buffers.

## Reads and eviction

`get` returns exactly the requested bytes from a covering entry. Every hit reads and hashes that
entry's entire payload against its XXHash64 checksum, excluding alignment padding. It reads neither
metadata nor neighboring entries. Small subrange requests can therefore cause large reads, though
only requested bytes are retained in the result.

Each read acquires one `DiskRegionReadGuard` under the range-index lock and retains it through verification.
Returned bytes retain no disk ownership. A checksum mismatch invalidates the indexed entry if its
expected checksum still matches the failed read's expected checksum. This can discard a newer identical copy.

Disk and memory consult standalone access history through `feuer-types::retention`.
Requests are recorded by tier orchestration before lookup, regardless of outcome, not by raw reads or writes. History owns its
counters independently of both tiers: eviction never deletes them, and cache entries hold no history handles.
Its clock counts requests across all keys. History is in-memory only and has no counter capacity limit.
Eviction samples live entries and removes the lowest recent retrieval value per payload byte.
Metadata, alignment, and unused chunk space count against capacity but not the score.

Eviction removes individual entries, but space remains unavailable until whole chunks are released.
Work per batch is bounded. Exhausted budgets or unavailable capacity skip admission rather than wait
for readers or writers. There is no relocation or cleaning.

## Metadata and recovery

`src/range_cache/page_format.rs` defines the experimental v9 format. All on-disk checksums use
XXHash64 with seed zero, stored as 8-byte little-endian integers. Metadata page headers are 48 bytes;
each entry record uses 48 bytes. Opening an older cache resets its generation rather than recovering the
old format. Records store the 128-bit key hash, object offset and length, one payload address, and payload
checksum. Aligned payload length is derived from object length. Each page has a checksum and carries the
checksum of the complete packed metadata prefix. Corruption in the prefix rejects its allocation during recovery.

The allocation header records the cache generation, consecutive chunk count, and packed record
byte length, excluding page headers and padding. Recovery reads the prefix and scans records sequentially;
there is no entry-start bitmap. Removal does not modify metadata. Whole-chunk reuse replaces it.

`recovery-ends` is a small checksummed file containing the layout, cache generation, and one scan end
per shard. Growing ends are checkpointed by atomic replacement every ten seconds; stale ends may omit
recent writes. Recovery snapshots those ends at open and stops there, regardless of new writes.
Missing, invalid, or incompatible inventory resets the cache generation and logs a cold start. A changed
shard count or capacity logs a warning with the previous and current layout: the entire old cache is discarded.
Recovery logs startup and completion time at info level; per-shard capacity and scan bounds are debug details. The reset
is synchronized before serving; old-generation chunks cannot reappear on later restarts. No payload
synchronization or final checkpoint on close is promised.

The allocator records chunks claimed during recovery, even if their owners later release them. New writes
may claim unscanned chunks immediately. Recovery reserves only chunks being inspected, validates their
packed metadata, generation, sequential payload ranges, and ownership, then publishes without displacing indexed
ranges. Shared-chunk entries retain shared ownership; multi-chunk entries reserve their complete contiguous run.
Inspection alone does not permanently claim a chunk. These temporary claim bitmaps disappear after the scan.

The scan issues one 4-KiB read at a time. It retries admission when the bounded read channel is full;
admitted requests run in FIFO order. Index locks never span scan I/O.
Already-submitted scan I/O can still contend for the device.

**Chunk writes are neither atomic nor durability barriers.** Bad metadata is skipped. Payload integrity is
checked on every disk hit, including recovered hits; missing or torn payload becomes a miss. Real device
power-loss testing is still required before claiming crash hardening.

## Validation and remaining work

Run on Linux with real io_uring/direct I/O and a fresh test directory:

```sh
mkdir -p /mnt/local-ssd/<isolated-test-directory>
TMPDIR=/mnt/local-ssd/<isolated-test-directory> cargo test --locked -p feuer-storage
```

Tests cover incremental recovery, concurrent allocation claims, stale scan ends, generation resets,
metadata corruption, reused multi-chunk addresses, contiguous payloads, rejection of fragmented free space,
and bounded FIFO I/O admission. Real Linux execution of
the new recovery tests and device power-loss testing remain outstanding, along with buffered mode and
tier-aware retention tuning.
Measure chunk utilization, metadata overhead, read/write amplification, and retention quality before
selecting this layout over alternatives.
