# Disk allocation and range-cache prototype

Experimental `DiskRangeCache` in `feuer-storage`, not a selected production design. The public
`TieredMemoryDiskCache` remains memory-only. `tiered-plan.md` remains authoritative.

## Implemented slice

`src/range_cache.rs` connects the two-level allocator to a sharded covering-range index and real `DataFile`
I/O. `open`, `insert`, and `get` are available independently of the public cache. Insert returns false for
contained downloads or insufficient arena space. Get returns exact requested bytes or a miss on uncertainty.
Neither operation records accesses; that belongs to later tier orchestration.

An insertion reserves both payload and entry metadata storage, writes plain payload bytes and the entry metadata
chain, then updates the embedded entry metadata index. Only successful writes may publish an in-memory mapping, after
containment revalidation. Broader entries replace contained entries; partial overlaps coexist. Reads acquire
`DiskRegionReadGuard`s before releasing the range-index lock. Returned `Bytes` retain no disk ownership.

A detached owner task keeps reservations through write completion despite requester cancellation. Any write
error or unexpected owner-task drop conservatively quarantines its allocations, including when completion
might be known; error classification can be refined later. A failed/abandoned embedded-index write disables
further index writes in that arena. Payload writes and reads do not hold the index-page write mutex.

Entries share 1-MiB chunks, with independently owned 4-KiB-aligned allocations. There is no pressure eviction
or write batching yet. Released storage is reused after redundant publication, broader replacement, or
corruption invalidation, subject to outstanding read guards. Callers must bound population concurrency;
the future memory-to-disk queue remains separate.

**Every open deliberately starts empty and logs this reset. Recovery is not implemented.** Persisting the
metadata connection now is not a promise that these entries survive reopen or that this format is final.

## Allocation and sharding

- The file is divided into 1-MiB chunks. Each starts with a permanently reserved 4-KiB embedded index page;
  255 blocks remain for payload and entry metadata. File capacity must be a positive multiple of 1 MiB.
- Whole free chunks are tracked as coalesced runs. Only partly free chunks need a 256-bit free-block bitmap.
  When the last allocated payload/entry metadata block is released, the chunk rejoins the whole-chunk pool.
- Large allocations take multiple chunks. Tails use 4-KiB blocks shared at chunk granularity with other entries.
  Scattered free blocks may satisfy an allocation without relocation, at the cost of more mapping records.
- The current private arena count is `clamp(capacity / 128 MiB, 1, 64)`. Each arena owns disjoint whole chunks,
  an allocator mutex, a range-index mutex and an asynchronous index-write mutex. Full-key hashing selects an
  arena; full-key equality selects entries. There is no cache-wide metadata mutex.
- This split is experimental: an entry may fail admission in its arena while other arenas have free space.
  The in-memory shard hash is not an on-disk identity and is not stable across Rust releases.
- Each entry's payload starts and allocated length are 4-KiB-aligned. Different entries never share an aligned
  write area, so replacing one does not require rewriting its neighbors. The minimum cost is 4 KiB of payload
  storage plus one 4-KiB entry metadata page; metadata for different entries is not packed together yet.

## Plain payload and per-entry checksums

Payload is stored unchanged, with no interleaved headers or per-page checksums. Only the last 4-KiB alignment block
is zero-padded. Complete regions use slices of the source buffer for writes; the raw direct-I/O layer still
copies into aligned I/O buffers. A final partial region is padded separately.

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
and physical mappings. A future decoder must check this checksum over the assembled entry metadata, not merely
validate each page independently.

Each embedded index contains a 256-bit bitmap of entry metadata starts, with bit zero unused. Bit `i` identifies
candidate entry metadata at `chunk_start + i * 4096`. Index-page updates are serialized within their arena.
Bits are not currently cleared when an entry is removed. These are recovery candidates, not a persistent
free-space map or proof of liveness. A reused address may contain different valid entry metadata; its complete
metadata and payload must be validated on their own merits, with ownership conflicts rejected.

Runtime allocation uses owned regions, not stale index bits. Opening deliberately starts with an empty
in-memory index and writes an empty first entry metadata index, but does not clear every index in the file.
This is not a durable reset protocol for future recovery.

## Crash/reuse rules and remaining recovery work

Writes complete in payload → entry metadata → embedded-index order before runtime publication. **Completion is
not persistence order.** There are no durability barriers. A valid embedded-index checksum does not prove that
its entry metadata or payload persisted. Recovery must not simply trust these index slots and reserve their mappings.

The connected format makes the next recovery prototype possible: scan embedded index pages, check their
version, address and bitmap checksum, then traverse each candidate's checksummed entry metadata chain without
scanning payload. Verify the complete-entry-metadata checksum before trusting its key, range, or mappings.
Before admitting candidates, validate all lengths, arithmetic, pointers, ordinals, chain termination,
file/arena bounds, full keys, ranges, and ownership conflicts. Reconstructed occupancy must protect both
payload and entry metadata storage. Stale index bits identifying reused addresses must never authorize
conflicting allocations. Payload integrity remains checked against the entry metadata's expected whole-entry
checksum at read time; stale mappings to different payload bytes cannot pass that check.

The next slice must decide and test the exact acceptance/ordering rules for new index bits with missing payload,
old index bits with reused payload, conflicting entry metadata chains, torn index updates, and restart after reuse.
This may require format changes or persistence barriers; none of those crash guarantees is established by the
current runtime tests. A future recovery implementation must reset unsupported or structurally uncertain
formats safely.

## Costs and validation

The reserved index pages alone cost 0.390625% of capacity. At 40 TiB, scanning them would read 160 GiB before
reading entry metadata. Payload has no header overhead and wastes at most 4,095 alignment bytes per entry.
Entry metadata still costs at least 4 KiB per entry, including a 32-byte payload checksum, and fragmented payloads
need more mapping records. Metadata packing and read amplification need measurement; none of these choices
is benchmark-selected. Sparse empty-arena accounting is not a claim of bounded live-index memory.

Run on Linux with real io_uring/direct I/O and a freshly created test directory:

```sh
mkdir -p /mnt/local-ssd/<isolated-test-directory>
TMPDIR=/mnt/local-ssd/<isolated-test-directory> cargo test --locked -p feuer-storage
```

Tests on `m8g-32cpu-local-ssd` use the local ext4 SSD. They cover allocation/reuse, long-key linked entry metadata,
full-key range lookup, containment races, caller cancellation, corruption/reused payload, partial population
failure, charged metadata capacity, disjoint arenas, aligned entries sharing a chunk, fragmented payloads, and
whole-entry validation of 100-MiB subrange hits. A truncated-file test verifies that a 1-KiB entry read does not
require its entry metadata or the rest of the chunk. Reopen tests assert the current intentional empty reset,
not recovery. Injected bounds failures exercise a successful write
followed by a failed write; actual device power-loss and torn-persistence tests remain outstanding.

Still required: pressure eviction, recovery/crash injection, buffered mode, comparative allocator measurements,
public tier integration and its bounded population queue. Compare this candidate with
size-segregated slabs, append-packed cleaning and a Foyer-style block baseline before choosing a layout.
