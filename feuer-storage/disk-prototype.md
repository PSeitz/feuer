# Disk allocation and range-cache prototype

Experimental `DiskRangeCache` in `feuer-storage`, not a selected production design. The public
`TieredMemoryDiskCache` remains memory-only. `tiered-plan.md` remains authoritative.

## Implemented slice

`src/range_cache.rs` connects whole-chunk allocation to a sharded covering-range index and real `DataFile`
I/O. `open`, `insert_batch`, and `get` are available independently of the public cache. Insertion accepts an
explicit `Vec<(ObjectKey, Download)>` and returns the number of entries published. Contained entries and
entries that do not fit are skipped. Get returns exact requested bytes or a miss on uncertainty.
Neither operation records accesses; that belongs to tier orchestration. `open_with_access_histories` accepts
`MemoryCache::access_histories()` so both tiers consult the same per-object evidence. A successful lookup
records once through that shared state; raw disk reads and population do not create a second event.

Within each shard, a batch sorts entries by payload size, smallest first, to group small entries together.
It reserves whole chunks and assembles payload, entry metadata chains, and discovery bitmaps in memory.
Each complete 1-MiB chunk is written once, including its index page and zero-filled unused space.
Entries share a chunk only if each entry's complete payload and metadata fit inside that chunk. An entry
spanning multiple chunks owns all of those chunks exclusively; its unused tail cannot hold another entry.
Partially filled chunks are written too; later batches cannot append to them. A single-entry batch
therefore costs at least one chunk. There is no background batching or flush timer.

All chunk writes for a shard finish before any of its batch entries publish. Publication rechecks containment,
with larger entries published first. Broader entries replace contained entries; partial overlaps coexist.
Publication is not transactional across shards: earlier shards may publish before a later shard fails.
Reads acquire `DiskRegionReadGuard`s before releasing the range-index lock. Returned `Bytes` retain no disk ownership.

A detached owner task retains chunk reservations through I/O despite requester cancellation. A write error or
unexpected drop during I/O conservatively quarantines all chunks in that shard's batch. This includes completed
or not-yet-submitted chunks; error classification can be refined later. Unwritten reservations can be released
normally. Distinct batches own disjoint chunks, so there is no shared-index write lock or persistent bitmap map.

Removing an entry never frees a hole inside a written chunk. The whole chunk becomes reusable only after all
entry owners and read guards release it. Pressure eviction removes live entries but does not wait for guards
or active writers. Callers must bound batch size and concurrency, including memory for complete chunk buffers;
the future memory-to-disk queue remains separate.

**Every open deliberately starts empty and logs this reset. Recovery is not implemented.** Persisting the
metadata connection now is not a promise that these entries survive reopen or that this format is final.

## Allocation and sharding

- The file is divided into 1-MiB chunks. Each starts with a permanently reserved 4-KiB embedded index page;
  255 blocks remain for payload and entry metadata. File capacity must be a positive multiple of 1 MiB.
- Whole free chunks are tracked as coalesced runs. There are no free-block bitmaps or partial-chunk reuse.
  A chunk returns to the free pool only when its last entry region or read guard is released.
- Large entries can span nonadjacent chunks, but those chunks are exclusive to that entry. Smaller entries
  can share a chunk only when each entry's payload and metadata stay entirely inside it. `DiskRegion`
  subranges share ownership of their containing whole-chunk reservation.
- The current private arena count is `clamp(capacity / 128 MiB, 1, 64)`. Each arena owns disjoint whole chunks,
  an allocator mutex and a range-index mutex. Full-key hashing selects an arena; full-key equality selects
  entries. There is no cache-wide metadata mutex and no metadata lock held across I/O.
- This split is experimental: an entry may fail admission in its arena while other arenas have free space.
  The in-memory shard hash is not an on-disk identity and is not stable across Rust releases.
- Each entry's payload starts and allocated length are 4-KiB-aligned. Replacements use fresh chunks; existing
  chunks remain unchanged. Each small entry uses at least 4 KiB of payload plus one 4-KiB metadata page inside
  its batch's chunks. Metadata for different entries is not packed into shared metadata pages yet.

## Bounded pressure eviction

Each shard maintains a dense rotating list of live entry candidates alongside its range index. Publication,
replacement, corruption invalidation and eviction update both under the existing shard index lock; removal
uses swap-removal, with no stale candidate backlog or shard-wide owner scan. Rotation supplies a bounded sample,
not the eviction order: each decision scores at most 64 entries and selects the lowest recent retrieval value
per payload byte, breaking ties by oldest publication. Alignment, metadata and chunk overhead are charged to
physical capacity but not the score's denominator.

The access history, retrieval-cost calculation, sampling bound and ratio comparison are shared with memory
through `feuer-types::retention`. Each key retains at most 64 exact requests, aged by the same successful-access
clock across both tiers. Only requests fully covered by an entry contribute value. Memory and disk entries
hold the same history; removing memory entries does not erase a disk-only key's evidence. Active population
keeps evidence alive through replacement. The weak key registry removes its record when the final history
owner drops. Evidence is volatile and starts empty on restart; snapshots are not implemented.

Eviction removes individual entries, never their neighbors as a group. No disk metadata reads, decoder or
reverse chunk-to-entry index are needed. A partially empty chunk remains unavailable while another entry or
read guard owns it. Each shard batch allows at most 64 sampled eviction decisions and 4,096 removed
payload/metadata region references. Oversized admissions that cannot fit beside the current unwritten batch
are skipped without eviction. Active batches are not eviction candidates. Exhausted budgets or unavailable
capacity skip admission rather than wait for ownership release. There is no relocation or cleaning, and no
claim that these budgets or this policy are performance-selected.

## Plain payload and per-entry checksums

Payload is stored unchanged, with no interleaved headers or per-page checksums. Final alignment padding and
unused chunk space are zero-filled. Payload is copied into complete chunk buffers; the raw direct-I/O layer
then copies those into aligned I/O buffers. Batch buffers are outside the raw I/O queue's memory budget.

Entry metadata stores a 32-byte BLAKE3 checksum over exactly its entry's payload, excluding alignment padding.
The in-memory entry index retains that expected checksum. Every hit reads and hashes the entire covering entry
in region order, copying only the requested bytes into the result. It does not read neighboring entries or
metadata. Thus a 1-KiB entry needs one aligned 4-KiB read, not a 1-MiB read. A small request within a 40-MiB entry
reads all 40 MiB for validation, but does not allocate a 40-MiB result buffer. All payload regions remain guarded
until verification completes. A mismatch anywhere in the entry invalidates the indexed entry only if its
expected payload checksum still matches the failed read. A newer identical copy may also be discarded;
that is an acceptable false miss, not a reason to distinguish write attempts.

## Experimental v3 metadata format

Only metadata uses 4-KiB pages. Integers are little-endian u64s. `src/range_cache/page_format.rs` defines the
encoding. There are no population IDs, write generations, or opening identities. Checksums identify contents,
not individual write attempts.

| Bytes | Meaning |
| --- | --- |
| 0..32 | BLAKE3 checksum of bytes 32..4096 |
| 32..64 | BLAKE3 checksum of the complete entry metadata, or the index's 32-byte bitmap of entry metadata starts |
| 64..72 | Physical page address |
| 72..80 | Logical page ordinal, or chunk number for embedded indexes |
| 80..88 | Next entry metadata page address; zero for index pages and the entry metadata chain's end |
| 88..96 | Kind/version tag: `FEUDES03` or `FEUIDX03` |
| 96..4096 | Contents followed by zero padding |

Entry metadata contains its total content length, full UTF-8 key length, exact object start/end, payload-region
count, the 32-byte expected payload checksum, ordered `(physical start, physical end)` pairs, then the complete
key bytes. Logical object bytes follow these regions in order; only the last region may have alignment padding.
Long keys/mapping lists span linked entry metadata pages; the links also identify the metadata's own allocations.
All entry metadata storage is charged to the same fixed capacity as payload. Every page in an entry metadata chain
carries the same complete-entry-metadata checksum. It covers the full key, exact object range, payload checksum,
and physical mappings. A future recovery decoder must check this checksum over assembled entry metadata,
not merely validate each page independently.

Each embedded index contains a 256-bit bitmap of entry metadata starts, with bit zero unused. Bit `i` identifies
candidate entry metadata at `chunk_start + i * 4096`. The bitmap is temporary chunk-construction state,
finalized before that chunk's only write. Payload-continuation chunks without metadata starts have an empty
bitmap. Removing an entry does not modify its chunk or bitmap. Bits identify recovery candidates, not liveness.
Whole-chunk reuse writes a completely new chunk, including a new bitmap.

Runtime allocation uses shared chunk ownership, not index bits. Opening starts with an empty in-memory lookup
index without writing or clearing old metadata. This is not a durable reset protocol for future recovery.

## Crash/reuse rules and remaining recovery work

Each chunk write includes its discovery index, payload and entry metadata. **A chunk write is not atomic or
a durability barrier.** A valid index checksum does not prove that entry metadata or payload persisted, even
within the same write. Multi-chunk entries can also be partially persisted. Recovery must not simply trust
index bits and reserve their mappings.

The connected format makes the next recovery prototype possible: scan embedded index pages, check their
version, address and bitmap checksum, then traverse each candidate's checksummed entry metadata chain without
scanning payload. Verify the complete-entry-metadata checksum before trusting its key, range, or mappings.
Before admitting candidates, validate all lengths, arithmetic, pointers, ordinals, chain termination,
file/arena bounds, full keys, ranges, and ownership conflicts. Reconstructed occupancy must protect whole
chunks containing either payload or entry metadata, allowing disjoint live entries to share a chunk. Stale index bits identifying reused addresses must never authorize
conflicting allocations. Payload integrity remains checked against the entry metadata's expected whole-entry
checksum at read time; stale mappings to different payload bytes cannot pass that check.

The next slice must decide and test the exact acceptance/ordering rules for new index bits with missing payload,
old index bits with reused payload, conflicting entry metadata chains, torn chunk writes, and restart after reuse.
This may require format changes or persistence barriers; none of those crash guarantees is established by the
current runtime tests. A future recovery implementation must reset unsupported or structurally uncertain
formats safely.

## Costs and validation

The reserved index pages alone cost 0.390625% of capacity. At 40 TiB, scanning them would read 160 GiB before
reading entry metadata. Payload has no header overhead and wastes at most 4,095 alignment bytes per entry.
Entry metadata still costs at least 4 KiB per entry, including a 32-byte payload checksum, and fragmented payloads
need more mapping records. Partially filled final chunks consume their full size, and holes left by removed
entries remain unavailable until whole-chunk reclamation. Batch buffers consume additional memory proportional
to their reserved chunks. Exclusive multi-chunk tails and entries that cannot fit a shared chunk's tail
increase unused capacity. Batch utilization, metadata packing, and read/write amplification need measurement;
none of these choices is benchmark-selected. Sparse empty-arena accounting does not bound live-index memory.

Run on Linux with real io_uring/direct I/O and a freshly created test directory:

```sh
mkdir -p /mnt/local-ssd/<isolated-test-directory>
TMPDIR=/mnt/local-ssd/<isolated-test-directory> cargo test --locked -p feuer-storage
```

Tests on `m8g-32cpu-local-ssd` use the local ext4 SSD. They cover allocation/reuse, long-key linked entry metadata,
full-key range lookup, containment races, caller cancellation, corruption/reused payload, partial population
failure, charged metadata capacity, disjoint arenas, mixed-size batches, finalized discovery bitmaps,
whole-chunk reuse delayed by entry owners and readers, exclusive multi-chunk ownership, packing boundaries,
bounded value-aware entry eviction, shared evidence across tiers, payload-only scoring, mixed-size churn,
concurrent eviction/reads,
fragmented chunks, and whole-entry validation of
100-MiB subrange hits. A truncated-file test verifies that a 1-KiB entry read does not
require its entry metadata or the rest of the chunk. Reopen tests assert the current intentional empty reset,
not recovery. Injected bounds failures exercise a successful write
followed by a failed write; actual device power-loss and torn-persistence tests remain outstanding.

Still required: retention-policy evaluation, recovery/crash injection, buffered mode, comparative allocator measurements,
public tier integration and its bounded population queue. Compare this candidate with
size-segregated slabs, append-packed cleaning and a Foyer-style block baseline before choosing a layout.
