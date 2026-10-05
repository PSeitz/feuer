# Disk-cache prototype

Experimental `DiskCache`, connected to public tiered lookup and owning background disk writes through a
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
lifetime. Each active shard needs at least one metadata chunk, charged against its capacity. A one-chunk cache consequently has no room for payloads.

Payloads with aligned size below 128 KiB share one unfinished chunk owned by their shard. Larger entries reserve consecutive whole chunks exclusively, including unused
tails. The allocator retains payload reservations; index entries and readers hold only addresses.
Individual payload holes are never reused. `src/allocation.rs` tracks coalesced free chunk runs;
each `ReservedChunks` has one owner: a pending write, the allocator, or a metadata chunk. I/O requests carry
addresses and buffers, not reservations. Scattered chunks are never combined for one entry.
The metadata component assigns entry positions, reserving whole chunks from the allocator as needed.

Each shard's metadata lock protects entry positions, page bytes, and dirty-page tracking;
it and the range-index lock never span I/O. Page bytes consume 1 MiB per metadata chunk.

## Writes

Each shard always owns an initialized 1-MiB small-entry buffer. `DiskCache::write` copies bytes into it and releases incoming buffers.
Buffered entries reserve no disk space or metadata positions. Flushing replaces the buffer with an empty one,
then attempts admission for the whole flush. Empty entry lists skip flushing.
The buffer component flushes every 60 seconds for direct and queued writes; full/no-fit chunks flush immediately.
Larger entries write independently without flushing pending small entries. Closing discards partial chunks.
`insert_batch` shares those chunks and flushes them before returning; its publication count can include other callers' entries.

After a region's payload writes finish, accepted entries update metadata in memory and publish into the index.
One periodic writer persists dirty metadata pages every second, best-effort; see the
[write contract](../format.md#write-ordering-and-reuse). Sleeping does not keep the cache open.

Publication rechecks disk containment: broader entries replace contained entries, while partial overlaps coexist.
Contained entries and entries that cannot fit are skipped.
Later writes may replace or evict entries published earlier in the same batch. A write failure releases
that region without undoing earlier publication. Publication does not check memory residency: the writer
owns prepared small-entry buffers or immutable larger downloads. Superseded entries release their payload occupancy without invalidation writes.

Only payload reservations cross I/O. Metadata chunks may grow before writing, but positions remain free until publication.
Publication checks capacity for the whole flush under the index lock; insufficient metadata capacity discards the written payload.
There are no metadata positions to return on failure or cancellation. Queued I/O retains buffers,
not disk space: an abandoned write can overwrite a reused payload, producing a checksum miss.
Failed metadata writes are not retried and neither reject published entries nor prevent payload reuse.
An abnormal queue failure retains active I/O resources when completion cannot be established.

Callers bound batch size and concurrency. Payload buffers and cached metadata pages are outside the
raw I/O queue's memory budget. Metadata writes share the payload write queue; this layout's write
throughput has not yet been compared against v10.

## Reads and eviction

`get` first copies the requested bytes from a covering buffered entry, without disk I/O. Busy buffers and
chunks detached for flushing may miss until disk publication; no in-flight lookup state is retained.
Otherwise it reads and hashes the complete covering disk entry against its XXHash64 checksum, excluding alignment
padding, then returns exactly the requested bytes. It reads neither metadata nor neighboring entries.
A read copies the address and expected checksum under the range-index lock, then reads into an
owned memory buffer. Concurrent disk overwrite is allowed: checksum validation happens after I/O
and before using the bytes, subject to the usual checksum-collision limitation. A checksum mismatch removes the entry only if its expected
checksum still matches the failed read; discarding a newer identical copy remains an allowed miss.

Disk and memory consult standalone access history through `feuer-types::retention`. Public requests
record accesses before lookup; raw reads and writes do not. Eviction samples live entries and removes
the lowest recent retrieval value per payload byte. Metadata, alignment, and unused chunk space
count against capacity but not the score. Each flush or independent large write allows at most 64 eviction attempts and
charges at most 4,096 chunks to removed entries; admission may be skipped when no contiguous run is free.
Free capacity in another shard cannot satisfy admission.

Index removal and entry destruction change neither metadata nor disk occupancy. Eviction explicitly
releases the allocator's payload reservation without changing the old metadata record. The allocator keeps one reservation and entry count per chunk run. Shared chunks become
free after their last entry is removed, without waiting for readers or queued writes. Payload chunks
described by the same metadata chunk do not share lifetimes. Metadata updates require no metadata reads because pages are cached in memory.
Closing the in-memory index does not invalidate live entries needed by the next open.

## Format and recovery

[format.md](../format.md) documents experimental **v13**: shard boundaries, fixed chain starts,
page headers, entry records, checksums, and write ordering. Each metadata chain starts at the first
chunk of its shard. There is no `recovery-heads` file or periodic address checkpoint.

Recovery follows links using **one 1-MiB read per metadata chunk**. It never scans payload chunks to
find metadata. The read channel holds at most 64 waiting requests and processes them in arrival order.
All metadata chunks in a shard are reserved before records can claim payload addresses. Record
decoding relies on the checksummed page's writer contract for range arithmetic and alignment, while
checking current shard identity. The allocator reserves payload
chunks and counts records sharing them; overlapping stale records are allowed and validated on read.
Entries are indexed during opening, with duplicate starts and contained ranges removed before publication.
Each shard's recovery runs on Tokio's blocking pool, using io_uring for reads. Opening returns only
after every shard has been scanned.

Recovery leaves valid pages clean, marks repaired links dirty, and rewrites corrupt record pages only
when reused. Payload checksums are verified on read, not during recovery.
Missing or torn payloads become misses. This remains a best-effort cache, not a durable object store.
Real device power-loss testing is required before claiming crash hardening.

## Validation

Run on Linux with real io_uring and direct I/O:

```sh
TMPDIR=/mnt/local-ssd cargo test --locked -p feuer-storage --lib
```

Tests cover links between full metadata chunks, one full-chunk recovery read independent of payload
size, metadata updates, independently reclaimable payloads, reuse without waiting for readers
or metadata writes, duplicate-start recovery, stale-record checksum misses, malformed/cyclic links, corrupt record pages,
metadata/payload ownership conflicts, fixed chain starts, replacement after reopening, and payload checksum
failures. Existing tests cover packing, fragmentation, cancellation, eviction, and contiguous payloads
up to 100 MiB. Recovery/write throughput and device power-loss behavior remain to be measured for v13.
