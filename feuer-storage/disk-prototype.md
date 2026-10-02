# Disk range-cache prototype

Experimental `DiskRangeCache`, connected to public tiered lookup and background disk writes through a
256-entry queue.
[tiered-plan.md](../tiered-plan.md) remains authoritative.

**Open waits for recovery to finish.** Every shard's metadata is scanned before the cache becomes
available for reads and writes.

## Layout and ownership

The fixed-capacity backing file contains two kinds of 1-MiB chunks:

```text
metadata chunk: [255 checksummed 4-KiB record pages][4-KiB next-chunk page]
payload chunks: [contiguous, aligned payload bytes, with no metadata gaps]
```

Each metadata page holds 84 fixed 48-byte records. A metadata chunk therefore holds up to **21,420
records**, shared across payload chunks and write batches. Its last page contains the next metadata
chunk address, or `u64::MAX` for the end of the chain. Metadata chunks stay reserved for the cache's
lifetime; cleared record slots are reused in place. Each active shard needs at least one metadata
chunk, charged against its capacity. A one-chunk cache consequently has no room for payloads.

Small payloads share chunks within an explicit batch. Each complete aligned payload must fit inside
its shared chunk. Larger entries reserve consecutive whole chunks exclusively, including unused
tails. Payload chunks are immutable while entry owners, queued I/O, or read guards retain them.
Individual payload holes are never reused. `src/allocation.rs` tracks coalesced free chunk runs;
`DiskRegion` slices share ownership of the entire reserved run. Scattered chunks are never combined
for one entry.

Metadata pages are mutable. A shard's async metadata I/O lock serializes reads and updates, separately
from allocator ownership. Its short synchronous metadata lock protects cached page bytes, free slots,
and pending invalidations; that lock and the range-index lock never span I/O. Cached metadata pages
consume approximately 1 MiB of memory per metadata chunk, plus free-slot bookkeeping.

## Writes

`insert_batch` sorts each shard's entries smallest first, packs aligned payloads into chunks, and
writes those chunks once. Partially filled payload chunks are finalized too; later batches cannot
append. Metadata growth never moves payload addresses. Storage adds no batching delay.

New metadata chunk initialization writes complete before linking them from the preceding chunk.
Each shard's first chunk is its fixed chain start and needs no incoming link. Payload writes finish
before their metadata records are written. Dirty metadata pages are written as 4-KiB updates, not
full-chunk rewrites, and complete before publication. The last-page link has its own checksum.
No `fsync` or `fdatasync` is issued; this completion order is not a persistence-ordering guarantee
after power loss.

Publication rechecks containment, larger entries first: broader entries replace contained entries,
while partial overlaps coexist. Contained entries and entries that cannot fit are skipped.
Publication is not transactional across shards. `insert_batch_checked` retains caller tokens and
invokes the synchronous publication check under the index lock. Rejected records and superseded
entries are invalidated afterward, before the insertion finishes successfully.

Each queued write retains its reserved region through completion despite caller cancellation.
The detached writer finishes submitted I/O. Failed metadata updates leave dirty pages and retired
payload reservations available for retry; uncertain payloads are not released for reuse. An abnormal
queue failure retains active I/O resources when completion cannot be established.

Callers bound batch size and concurrency. Payload buffers and cached metadata pages are outside the
raw I/O queue's memory budget. Metadata writes add foreground write cost; this layout's write
throughput has not yet been compared against v10.

## Reads and eviction

`get` reads and hashes the complete covering entry against its XXHash64 checksum, excluding alignment
padding, then returns exactly the requested bytes. It reads neither metadata nor neighboring entries.
A read acquires one `DiskRegionReadGuard` under the range-index lock and retains it through verification.
Returned bytes retain no disk ownership. A checksum mismatch removes the entry only if its expected
checksum still matches the failed read; discarding a newer identical copy remains an allowed miss.

Disk and memory consult standalone access history through `feuer-types::retention`. Public requests
record accesses before lookup; raw reads and writes do not. Eviction samples live entries and removes
the lowest recent retrieval value per payload byte. Metadata, alignment, and unused chunk space
count against capacity but not the score. Each shard batch allows at most 64 eviction attempts and
charges at most 4,096 chunks to removed entries; admission may be skipped rather than waiting for read guards.
Free capacity in another shard cannot satisfy admission.

Removing an entry clears its metadata record in memory and retains its payload reservation on a
pending-invalidation list. The next write flushes these invalidations **before trying to reuse the
payload space**. Invalidation writes complete before their reservations are released;
read guards can retain the payload longer. Payload chunks described by the same metadata chunk do
not share lifetimes. Metadata updates require no metadata reads because pages are cached in memory.
Closing the in-memory index does not invalidate live entries needed by the next open.

## Format and recovery

[format.md](format.md) documents experimental **v11**: shard boundaries, fixed chain starts,
page headers, entry records, checksums, and write ordering. Each metadata chain starts at the first
chunk of its shard. There is no `recovery-heads` file or periodic address checkpoint.

Recovery follows links using **one 1-MiB read per metadata chunk**. It never scans payload chunks to
find metadata. The read channel holds at most 64 waiting requests and processes them in arrival order.
All metadata chunks in a shard are reserved before records can claim payload addresses; duplicate, cyclic, out-of-shard, and conflicting
claims cannot reserve the same chunks twice. Record decoding checks range arithmetic, alignment,
shard identity, and payload overlap. Payload reservations are shared where entries share a chunk.
Entries are indexed during opening, and temporary allocator claim bits are released afterward.
Each shard's recovery runs on Tokio's blocking pool, using io_uring for reads. Opening returns only
after every shard has been scanned.

Corrupt pages/links are repaired in the in-memory metadata image and flushed before subsequent
writes can reuse space. Recovery does not read payloads: their checksums are verified on every hit.
Missing or torn payloads become misses. This remains a best-effort cache, not a durable object store.
Real device power-loss testing is required before claiming crash hardening.

## Validation

Run on Linux with real io_uring and direct I/O:

```sh
TMPDIR=/mnt/local-ssd cargo test --locked -p feuer-storage --lib
```

Tests cover links between full metadata chunks, one full-chunk recovery read independent of payload
size, mutable record-slot reuse, independently reclaimable payloads, completed invalidation writes plus read
guards before reuse, failed-invalidation retry, malformed/cyclic links, corrupt record pages,
metadata/payload ownership conflicts, fixed chain starts, replacement after reopening, and payload checksum
failures. Existing tests cover packing, fragmentation, cancellation, eviction, and contiguous payloads
up to 100 MiB. Recovery/write throughput and device power-loss behavior remain to be measured for v11.
