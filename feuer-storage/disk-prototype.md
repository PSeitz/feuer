# Disk range-cache prototype

Experimental `DiskRangeCache`, connected to public tiered lookup and bounded background population.
[tiered-plan.md](../tiered-plan.md) remains authoritative.

**Open starts background recovery without waiting for the scan.** Recovered entries become readable
incrementally while ordinary reads and population continue.

## Layout and ownership

- The file contains 1-MiB chunks, each starting with a reserved 4-KiB chunk metadata page.
  The remaining 1,020 KiB holds payload bytes and entry metadata.
- Payload is plain bytes in 4-KiB-aligned allocations. Only metadata uses 4-KiB pages.
  Each entry requires at least one metadata page. Entries do not share metadata pages.
- Small entries share a chunk only when each entry's complete payload and metadata fit inside it.
  Multi-chunk entries own all their chunks exclusively, including unused tails.
- Written chunks are immutable. A whole chunk becomes reusable only after all entry owners and
  `DiskRegionReadGuard`s release it. Individual holes are never reused.

`src/allocation.rs` tracks free chunks as coalesced runs. `DiskRegion` subranges share ownership
of their containing chunk. Allocation and the range index are sharded by full-key hash.
Free capacity in another shard cannot satisfy an admission. No metadata lock is held across I/O.

## Writes

`insert_batch` accepts explicit `(ObjectKey, Download)` pairs and returns the number published.
Within each shard, entries are sorted smallest first and packed into complete chunk buffers.
Payload, metadata, metadata-start bitmaps, and zero padding are finalized before each chunk's single write.
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
entry's entire payload against its BLAKE3 checksum, excluding alignment padding. It reads neither
metadata nor neighboring entries. Small subrange requests can therefore cause large reads, though
only requested bytes are retained in the result.

Reads acquire `DiskRegionReadGuard`s under the range-index lock and retain them through verification.
Returned bytes retain no disk ownership. A checksum mismatch invalidates the indexed entry if its
expected checksum still matches the failed read's expected checksum. This can discard a newer identical copy.

Disk and memory share access history and retention scoring through `feuer-types::retention`.
Successful accesses are recorded by tier orchestration, not raw reads or population.
Eviction samples live entries and removes the lowest recent retrieval value per payload byte.
Metadata, alignment, and unused chunk space count against capacity but not the score.

Eviction removes individual entries, but space remains unavailable until whole chunks are released.
Work per batch is bounded. Exhausted budgets or unavailable capacity skip admission rather than wait
for readers or writers. There is no relocation or cleaning.

## Metadata and recovery

`src/range_cache/page_format.rs` defines the experimental v4 format. Linked entry metadata pages
store the full key, object range, ordered payload regions, payload checksum, and batch ID. Each page has
a checksum, and each chain carries a checksum of the complete entry metadata. Chunk metadata records
the cache generation and batch ID, binding an entry to the batch that wrote all its chunks.

Each chunk metadata page contains a bitmap of metadata starts at 4-KiB-aligned offsets. These are
recovery candidates, not live-entry or free-space bits. Removal does not update the bitmap.
Whole-chunk reuse replaces it.

`recovery-ends` is a small checksummed file containing the layout, cache generation, and one scan end
per shard. Growing ends are checkpointed by atomic replacement every ten seconds; stale ends may omit
recent writes. Recovery snapshots those ends at open and stops there, regardless of new population.
Missing, invalid, or incompatible inventory resets the cache generation and logs a cold start. The reset
is synchronized before serving; old-generation chunks cannot reappear on later restarts. No payload
synchronization or final checkpoint on close is promised.

The allocator records chunks claimed during recovery, even if their owners later release them. Population
may claim unscanned chunks immediately. Recovery reserves only chunks being inspected, validates their
metadata chains, generation, batch IDs, mappings, and ownership, then publishes without displacing indexed
ranges. Shared-chunk entries retain shared ownership; multi-chunk entries reserve every chunk exclusively.
Inspection alone does not permanently claim a chunk. These temporary claim bitmaps disappear after the scan.
Metadata chains larger than 16 MiB are skipped to bound decoder memory and work.

The scan issues one 4-KiB read at a time. It retries admission rather than queueing ahead of foreground
waiters, and pending foreground requests precede pending scan requests. Index locks never span scan I/O.
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
metadata corruption, reused multi-chunk addresses, and foreground I/O priority. Real Linux execution of
the new recovery tests and device power-loss testing remain outstanding, along with buffered mode and
tier-aware retention tuning.
Measure chunk utilization, metadata overhead, read/write amplification, and retention quality before
selecting this layout over alternatives.
