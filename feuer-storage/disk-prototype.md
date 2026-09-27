# Disk range-cache prototype

Experimental `DiskRangeCache`; the public `TieredMemoryDiskCache` remains memory-only.
[tiered-plan.md](../tiered-plan.md) remains authoritative.

**Every open starts empty. Recovery is not implemented.** Old on-disk metadata is not cleared;
this is not a durable reset protocol.

## Layout and ownership

- The file contains 1-MiB chunks, each starting with a reserved 4-KiB chunk metadata page.
  The remaining 1,020 KiB holds payload bytes and entry metadata.
- Payload is plain bytes in 4-KiB-aligned allocations. Only metadata uses 4-KiB pages.
  Each entry requires at least one metadata page; entries do not share metadata pages.
- Small entries share a chunk only when each entry's complete payload and metadata fit inside it.
  Multi-chunk entries own all their chunks exclusively, including unused tails.
- Written chunks are immutable. A whole chunk becomes reusable only after all entry owners and
  `DiskRegionReadGuard`s release it. Individual holes are never reused.

`src/allocation.rs` tracks free chunks as coalesced runs. `DiskRegion` subranges share ownership
of their containing chunk. Allocation and the range index are sharded by full-key hash;
free capacity in another shard cannot satisfy an admission. No metadata lock is held across I/O.

## Writes

`insert_batch` accepts explicit `(ObjectKey, Download)` pairs and returns the number published.
Within each shard, entries are sorted smallest first and packed into complete chunk buffers.
Payload, metadata, metadata-start bitmaps, and zero padding are finalized before each chunk's single write.
Partially filled chunks are written too; later batches cannot append. There is no background batching.

All writes for a shard finish before publication. Publication rechecks containment, larger entries
first: broader entries replace contained entries, while partial overlaps coexist. Contained entries
and entries that cannot fit are skipped. Publication is not transactional across shards.

A detached task retains reservations through I/O despite caller cancellation. A write error or
unexpected drop during I/O quarantines every chunk in that shard's batch, including completed and
not-yet-submitted writes.

Callers must bound batch size and concurrency. Complete chunk buffers are outside the raw I/O
queue's memory budget and are copied again into aligned direct-I/O buffers.

## Reads and eviction

`get` returns exactly the requested bytes from a covering entry. Every hit reads and hashes that
entry's entire payload against its BLAKE3 checksum, excluding alignment padding. It reads neither
metadata nor neighboring entries. Small subrange requests can therefore cause large reads, though
only requested bytes are retained in the result.

Reads acquire `DiskRegionReadGuard`s under the range-index lock and retain them through verification.
Returned bytes retain no disk ownership. A checksum mismatch invalidates the indexed entry if its
expected checksum still matches the failed read's expected checksum; this can discard a newer identical copy.

Disk and memory share access history and retention scoring through `feuer-types::retention`.
Successful accesses are recorded by tier orchestration, not raw reads or population.
Eviction samples live entries and removes the lowest recent retrieval value per payload byte.
Metadata, alignment, and unused chunk space count against capacity but not the score.

Eviction removes individual entries, but space remains unavailable until whole chunks are released.
Work per batch is bounded; exhausted budgets or unavailable capacity skip admission rather than wait
for readers or writers. There is no relocation or cleaning.

## Metadata and recovery

`src/range_cache/page_format.rs` defines the experimental v3 format. Linked entry metadata pages
store the full key, object range, ordered payload regions, and payload checksum. Each page has a
checksum, and each chain carries a checksum of the complete entry metadata.

Each chunk metadata page contains a bitmap of metadata starts at 4-KiB-aligned offsets. These are
recovery candidates, not live-entry or free-space bits. Removal does not update the bitmap;
whole-chunk reuse replaces it.

**Chunk writes are neither atomic nor durability barriers.** Recovery must validate metadata chains,
mappings, and ownership conflicts rather than trust bitmap bits. Acceptance rules for torn writes,
missing payload, and reused addresses remain unresolved and may require format or persistence changes.
Current runtime tests do not establish crash guarantees.

## Validation and remaining work

Run on Linux with real io_uring/direct I/O and a fresh test directory:

```sh
mkdir -p /mnt/local-ssd/<isolated-test-directory>
TMPDIR=/mnt/local-ssd/<isolated-test-directory> cargo test --locked -p feuer-storage
```

Reopen tests assert an empty reset, not recovery. Device power-loss and torn-persistence tests remain
outstanding, along with public tier integration, a bounded population queue, and buffered mode.
Measure chunk utilization, metadata overhead, read/write amplification, and retention quality before
selecting this layout over alternatives.
