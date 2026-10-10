# Feuer Implementation Status

**Status:** implementation is in progress. [`tiered-plan.md`](tiered-plan.md) is the authoritative behavioral
contract. This document records what exists and what remains to build.

## Current state

The public cache now connects **memory → integrity-checked disk → callback**. The fallible asynchronous
`TieredMemoryDiskCache::open` opens the configured directory using Linux direct I/O and io_uring, with no
silent fallback. One worker drains a 512-entry queue into per-shard small-entry chunks, flushing
on full/no-fit or every minute; larger entries write separately. Reopening reads one 1-MiB metadata chunk at a time per shard and waits for
all shards to recover before returning.
The recovery additions cross-compile for Linux; real io_uring execution and device-crash testing remain outstanding.

| Area | Implemented | Remaining |
| --- | --- | --- |
| `feuer` | Fallible async open, cloneable tiered handle, memory/disk/callback lookup, disk writes through a 512-entry queue with per-shard small-entry chunks, typed callback and validation errors | I/O mode selection, recovery crash testing, tier-aware retention tuning |
| `feuer-types` | XXH3-128 object identity, exact non-empty `ByteRange`, keyless `Download` with a derived range | None for the current public type boundary |
| `feuer-historian` | Standalone request history, decayed access counts, and recent request events | History is volatile; distinct counters have no capacity limit |
| `feuer-memory` | Sharded covering-range index, exact access counts shared with disk, per-object trimming-event limits, sampled retention policy, pressure-driven compaction, payload accounting, metrics | Wall-clock evidence aging, disk-state inputs, further trace-independent evaluation |
| `feuer-storage` | Fixed-capacity Linux O_DIRECT file, io_uring driver with up to 64 active reads and 8 active writes, experimental sharded `DiskCache` with small entries packed into immutable 1-MiB chunks, whole-entry checksums, whole-chunk reuse and per-admission eviction limits | Recovery crash testing, buffered mode, retention-policy evaluation, comparative allocator measurements |
| Runtime and tooling | Feuer-only workspace/CI, memory comparison gate, raw storage benchmarks | End-to-end acceptance and crash tests, examples, tiered and concurrent cache benchmarks |

## Implemented behavior

### Public lookup and memory insertion

- Lookup checks memory, then integrity-checked disk, before invoking the per-call callback. Disk hits promote
  only the requested bytes to memory without scheduling another write. Read uncertainty invalidates disk
  state through the existing storage path and falls through to the callback.
- Misses invoke callbacks independently, without internal coordination or source retries. Each callback returns
  one `Download { downloaded_start, bytes }`, whose derived range must cover the request.
- Successful lookups return exactly the requested bytes in one `Bytes`, which may share a larger allocation.
- Insertion discards a download already contained by cached data. Partially overlapping downloads may remain
  independent. Broader downloads replace contained ranges.
- Tier orchestration records each started request's exact range once, before lookup, in standalone shared
  history. Failures, invalid downloads, and cancellation after starting still count as demand.
  Raw memory/disk lookups and insertions do not record accesses; history takes no cache shard lock.
- Cached allocations and idle buffers share the configured memory capacity. Disk promotions copy the
  requested slice into the pool if its allocation capacity is smaller than the original backing
  capacity; allocation failure retains the original slice. Admission charges the chosen allocation's
  whole capacity. Whole-entry reads and checksums are unchanged. Callback downloads use payload
  length when allocation capacity is unknown. Entry targets remain divided among shards, preserving the
  existing oversized-entry exception. Caller-only results survive eviction outside cache accounting.
- `feuer-memory` owns one aligned buffer pool per cache instance: 32 KiB, 256 KiB, 512 KiB, 1 MiB,
  2 MiB, 4 MiB, 8 MiB, 16 MiB, 32 MiB, and 64 MiB. The idle pool is capped at 7% of memory capacity
  by default, shared by all buckets without per-bucket caps or reservations. `FEUER_IDLE_BUFFER_POOL_PERCENT` configures this percentage
  (0–100; zero disables idle retention).
  Admission frees idle buffers as needed; returns never evict cached entries. Above 64 MiB, buffers share one exact-size bucket and are resized on reuse.

### Memory retention and compaction

- Cost-aware scoring uses exact-range access counts with a 262,144-request half-life by default,
  measured across all keys. Both tiers use the shared policy in `feuer-memory::retention` to score
  standalone history in `feuer-historian`, with 64
  independently locked maps selected by object key and one global atomic request clock, separate from cache
  shards. Every distinct counter survives all cache evictions for the
  history object's lifetime; metadata has no capacity limit. History is volatile, not persisted.
  Separately, range trimming retains at most 64 repeated events per object by default
  (`FEUER_MAX_ACCESS_EVENTS_PER_KEY`), expiring after 262,144 requests across all keys
  (`FEUER_MAX_ACCESS_AGE_ACCESSES`). These limits do not truncate scoring counters. Wall-clock aging is not
  implemented.
- Decayed counts weight the sum of fixed-cost-equivalent bytes (default zero,
  configurable per cache via `CacheConfig::with_fixed_retrieval_equivalent_bytes`) and requested bytes. Only cached ranges
  covering the exact request receive credit. Counter metadata is not capped per object.
- A dense rotating candidate ring supplies a shared sample of at most 64 live entries per pressure decision.
  The victim has the lowest retrieval value per charged allocation byte, with object key and range breaking ties.
  Registration and removal are constant-work and leave no stale candidate backlog.
- After a grace of 64 requests across all keys, the selected victim is trimmed to observed requests if
  that releases at least one quarter of its payload. Otherwise it is evicted. Grace never prevents eviction.
- Compaction merges only overlapping or adjacent observed intervals and copies them into independent `Bytes`.
  It preserves exact coverage, updates accounting and metrics, creates no access, and leaves caller-held
  slices valid.
- Copying happens outside the shard lock. Publication requires the exact source range to still be cached.
  Reinsertion of that immutable range, changes to neighboring ranges, and new accesses do not invalidate
  a trimming snapshot. Replacement ranges already covered by another entry are skipped.
  Admission falls back to eviction if the source was removed or replaced by a different range.

There is no periodic compaction, separate prefetch-promotion state, or public policy configuration.

### Raw direct I/O

- Reads and writes have separate request channels, pending queues, active slots, rings, and worker threads.
  The queues overlap up to 64 reads and 8 writes and wake for new requests while I/O is outstanding,
  without busy polling or registered buffers.
- Request slots cover preparing, queued, and active I/O. Each request transfers at most 1 MiB.
  These limits do not bound caller inputs or full read-result allocations; there is no buffer-memory semaphore.
- Reads accept arbitrary byte ranges. Write offsets and lengths must be multiples of 4 KiB, enforced by
  assertions. The driver performs no read-modify-write or overlap checks: the upper layer must supply
  complete aligned bytes and either serialize conflicting access or validate read checksums. `DataFile` rounds read requests outward and slices the results. The io_uring driver accepts
  only nonempty aligned requests and returns complete aligned read buffers.
- Multi-chunk operations are not atomic. Completion permits subsequent reads but does not guarantee crash
  durability. There are no global scheduling barriers.
- Caller cancellation does not cancel submitted kernel writes. `DiskCache`'s detached writer owns the
  current payload reservation while awaiting I/O. Late writes after failure or abandonment can cause
  checksum misses on reused payloads. Publication rechecks disk containment, not memory residency.
- Submitted buffers survive caller cancellation. Last-handle drop drains and joins the driver. Abnormal
  driver failure retains uncertain active buffers and the directory lock until process exit.
- Raw capacity must be positive, 4-KiB-aligned, and representable as a Linux signed file offset. Opening
  requires usable io_uring and compatible `STATX_DIOALIGN`. There is no backend or buffered fallback.

This is a raw I/O layer, not a disk cache or disk-write queue. Disk storage and its CI target Linux only.

## Validation and benchmarks

- The [memory suite](benchmarks/memory/README.md) compares exact-range and expanded downloads
  against native Foyer S3FIFO and a local exact-key cost-aware Foyer policy. The expanded strategy looks 5 ms
  ahead, coalesces gaps below the 10,000,000-byte source-cost break-even distance, and defaults to whole splits
  below 8 MiB and exact ranges otherwise.
- [Memory results](benchmarks/memory/results.md) cover configured targets from 256 MiB through 32 GiB and
  report request/source-cost hit rates, actual cached payload bytes, throughput, and shard-count sensitivity.
  These results do not establish general or tiered superiority over Foyer.
- The complete memory gate was rerun after sharing access evidence (eight capacities through 32 GiB, exact
  and expanded downloads, 1/4/16/64 shards). All hit counts, hit bytes, source GETs/bytes and cached-payload
  values matched the preceding implementation across all 256 engine rows. In this single same-host comparison,
  Feuer's median elapsed-time ratio across its 64 cases was 1.153 (about 15% slower). The native Foyer medians
  were approximately unchanged. Shared evidence adds synchronization and metadata. This is not a throughput
  improvement. CSV artifacts on `m8g-32cpu-local-ssd` are
  `/mnt/local-ssd/feuer-value.BkkxPo/memory-gate.csv` and
  `/mnt/local-ssd/feuer-pressure.RgwqyQ/memory-baseline.csv`.
- Storage tests cover independent read/write queue capacity and progress, mixed concurrency,
  caller-serialized shared pages, bounds, short I/O, cancellation, and reopening.
- The [direct-I/O smoke benchmark](benchmarks/storage/README.md) measures the driver on the local SSD, not
  an end-to-end cache or matched backend comparison. Separate [SSD measurements](benchmarks/ssd/ssd-concurrent-read-write.md)
  explore mixed reads and writes. Small-read-heavy workloads may still warrant write throttling.

## In progress: disk-cache prototype

[`format.md`](format.md) describes the experimental disk format;
[`feuer-storage/disk-prototype.md`](feuer-storage/disk-prototype.md) covers runtime behavior and remaining crash testing.
Each shard owns an initialized 1-MiB buffer shared by all insertion paths; the buffer component drives its flush timer.
Buffering reserves no disk space. A flush reserves payload chunks; metadata positions remain free until publication.
Payload completion precedes publication; a separate writer persists metadata every second.
Reads first copy requested bytes from buffered entries; busy/flushing chunks may miss until publication.
Disk reads verify the whole covering entry's XXHash64 checksum, without reading neighbors or metadata.

Independent shards have their own allocator and range-index lock. The allocator tracks coalesced free
whole-chunk runs; the metadata component reserves entry positions and grows metadata capacity internally.
Detached payload writers retain reservations through completion despite caller cancellation.
Failed payload writes release their chunks without metadata rollback. Insufficient metadata capacity at publication discards the whole written payload.
Publication is not transactional across shards. Callers bound batch memory and concurrency.

Entries with aligned size below 512 KiB share 1-MiB payload chunks. Larger entries own consecutive
chunks exclusively, including unused tails. Payload chunks contain no metadata. Removed payload holes cannot be reused individually.
The v13 format stores fixed 48-byte records with 128-bit key hashes in separate 1-MiB metadata chunks. The first 255
pages hold up to 21,420 records; the final 4-KiB page stores the next metadata chunk address. Metadata chunks remain
reserved, but removed entries' metadata positions are reused across batches. An active shard needs at least one
metadata chunk in addition to its payload chunks. One periodic writer persists dirty 4-KiB pages every second;
see the [write contract](format.md#write-ordering-and-reuse). Payload chunks remain independently reclaimable.
Each page and payload has its own XXHash64 checksum. Older formats are not migrated.
Read invalidation compares expected payload checksums. Discarding a newer identical copy is an allowed miss.
Pressure eviction samples up to 64 live entries and selects the lowest recent retrieval value per payload
byte, using the same history, cost calculation and comparison as memory. Ties choose the oldest publication.
Alignment, metadata and chunk overhead do not enter the score. Only selected entries are removed. Neighbors
remain indexed and may keep a partially empty chunk unavailable. No eviction metadata reads are needed.
Each flush or independent large write is limited to 64 sampled decisions and 4,096 chunks charged to removed entries. Active
storage remains unavailable, and exhausted budgets skip admission. `DiskCacheOptions::access_histories` connects the
disk cache to a memory cache's evidence. Public tier orchestration now uses it.
Recovery starts at each shard's first chunk and follows last-page links, reading exactly 1 MiB per metadata chunk
without scanning payload chunks. Fixed starts need no `recovery-heads` file or periodic checkpoint. Open waits for
all shards to recover before reads and writes become available. Corrupt record pages are discarded independently;
invalid or cyclic links terminate the chain. See the format document for write ordering and recovery details.
No comparative layout/performance claim is established.

The disk-cache tests cover persisted key-hash entry metadata and payload checksums, containment races, caller
cancellation, corruption/reused payload, partial batch failure, metadata-only chunks, disjoint shards,
mixed-size packing, exclusive multi-chunk ownership, finalized entry metadata, whole-chunk ownership/reuse,
per-admission eviction limits, shared evidence across tiers, payload-only scoring, mixed-size churn,
concurrent eviction/reads,
rejection of scattered free chunks, contiguous payloads across chunk boundaries, and whole-entry
validation of 100-MiB subrange hits. A 1-KiB hit succeeds with only its aligned payload block readable.
Unrelated entries and metadata are not loaded. Before background recovery was added, on `m8g-32cpu-local-ssd`,
all 71 storage tests and all 135 workspace tests passed with real direct I/O
and io_uring on the local ext4 SSD. Workspace Clippy passed with warnings denied. Formatting and whitespace
checks passed for the changed files.

## Implemented: best-effort disk scheduling

- A nonblocking queue allows at most 256 pending entries. Queued and active payload bytes are tracked
  but not limited or charged to the memory-cache capacity.
- Each shard owns its small-entry chunk; one worker submits downloads and drives the 60-second flush sweep.
  Larger entries write separately. Queue saturation skips candidates; closing discards partial chunks.
- Exact-range downloads and whole objects are queued after memory admission. Expanded downloads stay
  memory-only until successful compaction queues newly published ranges outside the memory shard lock.
- Small entries release incoming buffers after copying into aligned chunk buffers; larger writes own downloads.
  Admitted entries may publish after memory eviction; publication rechecks only disk containment.
  Neither publication nor disk eviction takes a memory shard lock.
- Failed writes are logged and remain invisible. Shared evidence survives queued and active disk writes.
- Requests record exactly once before lookup, including requests that fail or are later canceled. Callback results already
  covered by disk are discarded without memory admission or another write.

On `m8g-32cpu-local-ssd`, all 145 workspace tests passed with real direct I/O and io_uring after integration.
New tests cover disk hits after memory pressure, requested-range promotion, exactly-once evidence, directory
ownership, byte/count saturation, oversized candidates, queued eviction/readmission, batching, callback and
writer cancellation, memory eviction after write admission, and disk-contained callback suppression. No new corruption tests were
added in this slice. Workspace Clippy passed with warnings denied. Formatting checks passed for changed Rust
files, and whitespace checks passed.
The isolated validation checkout is `/mnt/local-ssd/feuer-tiered.iB2JNA`.

Small-entry writer validation: all 221 workspace library tests pass on `m8g-32cpu-local-ssd-2`
with real direct I/O and io_uring. Coverage includes full/no-fit chunks, the aligned cutoff,
independent larger writes, 60-second flushing, incoming-buffer release, deferred disk admission,
cancellation cleanup, and discarding partial chunks on close. Linux-target workspace Clippy passes with warnings denied.

## Implemented: cache metrics

- `open` accepts an optional `mixtrics` metrics registry, independently of optional shared I/O queues,
  wiring metrics through public lookups, callbacks, both cache tiers and disk writes. Without a registry,
  metrics are no-op. Labels contain only fixed operation/outcome/source values.
- Added lookup latency/outcomes and served bytes, callback counts/latency/download bytes, disk read-error
  and integrity outcomes, disk-write admission/skip/terminal outcomes, queue pressure/wait time,
  chunk capacity states, indexed payload/entries, pressure eviction and byte-weighted batch packing.
- Queue gauges track entries and logical payload through preparation and completion; capacity gauges track
  payload reservations through completion, failure, and discard, with metadata positions taken only at publication. See [`metrics.md`](metrics.md) for exact accounting semantics.
- Validation on macOS: portable Feuer/memory/types tests pass. Linux workspace all-target checks and Clippy
  pass. New Linux metric integration tests are compile-checked but have not been executed on this host.

## Remaining implementation

### Disk range engine and I/O modes

- Integrate and measure covering-range lookup and whole-entry validation, including subrange read amplification.
- Add `PayloadIoMode::Buffered` and `PayloadIoMode::Direct`, integrating the existing driver without silent
  backend fallback.
- Keep requested and downloaded ranges arbitrary and unaligned. Physical entry allocations are 4-KiB-aligned.
- Evaluate sampled value-aware disk eviction, payload-only scoring and partially empty chunk utilization.
- Bound allocation work, fragmentation reclamation, rewrite traffic, metadata, and physical capacity at the
  contract's target scale of at least 30 TiB. Avoid a cache-wide hot lock.

The allocator is undecided. Compare size-segregated slabs, append-packed storage with cleaning, and a
Foyer-style block baseline using trace-driven simulation and focused prototypes. Measure useful utilization,
fragmentation, metadata, allocation latency, and relocation traffic before selecting a design. Layout,
alignment, size classes, and cleaning remain private implementation choices.

Multi-range assembly and physical difference-only storage for overlapping downloads are deferred, not
permanently excluded by the design.

### Integrity and recovery

- Validate the versioned metadata format and whole-entry XXHash64 checksums against injected crash and reuse cases.
- Exercise incremental recovery and scan-end resets on real Linux direct I/O and io_uring.
- Validate recovery after process and machine crashes, beyond metadata corruption and reuse tests.

### Tier-aware policy and hardening

- Account for ranges with no disk copy, queued or active writes, and disk-resident copies when valuing memory
  retention. Keep policy internal.
- Bound the disk share of prefetched bytes with no observed request.
- Evaluate beyond the captured trace, including wall-clock evidence aging based on object lifecycle.
- Add low-overhead signals for tier outcomes, callbacks, latency, memory pressure, compaction, disk queues,
  allocator utilization, fragmentation, rewrite traffic, integrity, and recovery, without object-key labels.
- Add concurrency, randomized, target-scale, and end-to-end tests against the acceptance criteria in
  [`tiered-plan.md`](tiered-plan.md).

### Comparative benchmark gate

- Rerun the memory-only gate and add a controlled tiered comparison against native Foyer at a pinned revision
  and reported tuning. Hold callback ranges, application coordination, memory target, disk capacity,
  concurrency, and cold-cache start constant. Report actual used memory.
- Evaluate prefetch and downloaded-range selection separately. Instrument actual source GETs and bytes in
  the application harness and charge `GETs * 125 ms + bytes / 80 MB/s`. Callback counts are not source GETs.
- Report cost-weighted, request, and byte hit rates, useful-payload utilization, fragmentation and metadata,
  read/write/cleaning/relocation amplification, throughput, and tail latency.
- Keep allocator and policy choices experimental until measurements support them. Make no claim of beating
  Foyer before controlled results demonstrate it.
