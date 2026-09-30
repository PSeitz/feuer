# Feuer Implementation Status

**Status:** implementation is in progress. [`tiered-plan.md`](tiered-plan.md) is the authoritative behavioral
contract. This document records what exists and what remains to build.

## Current state

The public cache now connects **memory → integrity-checked disk → callback**. The fallible asynchronous
`TieredMemoryDiskCache::open` opens the configured directory using Linux direct I/O and io_uring, with no
silent fallback. A bounded worker batches retained downloads into the experimental `DiskRangeCache`.
Reopening starts bounded background recovery, publishing entries incrementally alongside reads and writes.
The recovery additions cross-compile for Linux; real io_uring execution and device-crash testing remain outstanding.

| Area | Implemented | Remaining |
| --- | --- | --- |
| `feuer` | Fallible async open, cloneable tiered handle, memory/disk/callback lookup, bounded batched disk writes, typed callback and validation errors | I/O mode selection, recovery crash testing, tier-aware retention tuning |
| `feuer-types` | String-backed fully compared `ObjectKey`, exact non-empty `ByteRange`, keyless `Download` with a derived range, internal shared access evidence and value comparison | None for the current public type boundary |
| `feuer-memory` | Sharded covering-range index, bounded exact access evidence shared with disk, sampled retention policy, pressure-driven compaction, payload accounting, metrics | Wall-clock evidence aging, disk-state inputs, further trace-independent evaluation |
| `feuer-storage` | Fixed-capacity Linux O_DIRECT file, bounded QD64 io_uring driver, experimental sharded `DiskRangeCache` with explicit batches packed into immutable 1-MiB chunks, whole-entry checksums, guarded whole-chunk reuse and bounded value-aware entry eviction | Recovery crash testing, buffered mode, retention-policy evaluation, comparative allocator measurements |
| Runtime and tooling | `feuer-tokio`, Feuer-only workspace/CI, memory comparison gate, raw storage benchmarks | End-to-end acceptance and crash tests, examples, tiered and concurrent cache benchmarks |

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
- Cached allocations and idle buffers share the configured memory capacity. Disk promotions carry the
  whole backing allocation capacity through slicing into memory admission. Callback downloads use payload
  length when allocation capacity is unknown. Entry targets remain divided among shards, preserving the
  existing oversized-entry exception. Caller-only results survive eviction outside cache accounting.
- `feuer-memory` owns one aligned buffer pool per cache instance: 32 KiB, 256 KiB, 4 MiB, 16 MiB,
  32 MiB, and 64 MiB. The idle pool is capped at 7% of memory capacity by default, shared by all six
  buckets without per-bucket caps or reservations. `FEUER_IDLE_BUFFER_POOL_PERCENT` configures this percentage
  (0–100; zero disables idle retention).
  Admission frees idle buffers as needed; returns never evict cached entries. Above 64 MiB is unpooled.

### Memory retention and compaction

- Cost-aware scoring uses exact-range access counts with an 8,192-request half-life by default,
  measured across all keys. Both tiers consult standalone history in `feuer-types::retention`, with 64
  independently locked maps selected by object key and one global atomic request clock, separate from cache
  shards. Every distinct counter survives all cache evictions for the
  history object's lifetime; metadata has no capacity limit. History is volatile, not persisted.
  Separately, range trimming retains at most 64 repeated events per object by default
  (`FEUER_MAX_ACCESS_EVENTS_PER_KEY`), expiring after 262,144 requests across all keys
  (`FEUER_MAX_ACCESS_AGE_ACCESSES`). These limits do not truncate scoring counters. Wall-clock aging is not
  implemented.
- Decayed counts weight the sum of fixed-cost-equivalent bytes (default 10,000,000,
  configurable via `FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES`) and requested bytes. Only cached ranges
  covering the exact request receive credit. Counter metadata is not capped per object.
- A dense rotating candidate ring supplies a shared sample of at most 64 live entries per pressure decision.
  The victim has the lowest retrieval value per retained byte, with monotonic entry identity breaking ties.
  Registration and removal are constant-work and leave no stale candidate backlog.
- After a grace of 64 requests across all keys, the selected victim is trimmed to observed requests if
  that releases at least one quarter of its payload. Otherwise it is evicted. Grace never prevents eviction.
- Compaction merges only overlapping or adjacent observed intervals and copies them into independent `Bytes`.
  It preserves exact coverage, updates accounting and metrics, creates no access, and leaves caller-held
  slices valid.
- Copying happens outside the shard lock. Entry identity and structural generation checks reject output
  invalidated by same-object cached-range changes. New accesses do not invalidate a trimming snapshot.
  Admission falls back to eviction when cached-range changes invalidate a copy.

There is no periodic compaction, separate prefetch-promotion state, or public policy configuration.

### Raw direct I/O

- Reads and writes have separate request channels, pending queues, active slots, rings, and worker threads.
  The queues overlap up to 64 reads and 8 writes and wake for new requests while I/O is outstanding,
  without busy polling or registered buffers.
- Request slots cover preparing, queued, and active I/O. Each request transfers at most 1 MiB.
  These limits do not bound caller inputs or full read-result allocations; there is no buffer-memory semaphore.
- Reads accept arbitrary byte ranges. Write offsets and lengths must be multiples of 4 KiB, enforced by
  assertions. The driver performs no read-modify-write or overlap checks: the upper layer must supply
  complete aligned blocks and prevent conflicting access across physical byte ranges rounded outward
  to 4 KiB. `DataFile` rounds read requests outward and slices the results. The io_uring driver accepts
  only nonempty aligned requests and returns complete aligned read buffers.
- Multi-chunk operations are not atomic. Completion permits subsequent reads but does not guarantee crash
  durability. There are no global scheduling barriers.
- Caller cancellation does not cancel submitted kernel writes. The task owning a write's `DiskRegion` must
  keep awaiting completion and prevent conflicting access or reuse, even when the result is abandoned.
  `DiskRangeCache` owns reservation lifetime. Public scheduling additionally revalidates the memory-entry
  identity under its shard lock throughout disk publication.
- Submitted buffers survive caller cancellation. Last-handle drop drains and joins the driver. Abnormal
  driver failure retains uncertain active buffers and the directory lock until process exit.
- Raw capacity must be positive, 4-KiB-aligned, and representable as a Linux signed file offset. Opening
  requires usable io_uring and compatible `STATX_DIOALIGN`. There is no backend or buffered fallback.

This is a raw I/O layer, not a disk cache or disk-write queue. Disk storage and its CI target Linux only.

## Validation and benchmarks

- The [memory suite](benchmarks/memory/README.md) compares exact-range and source-bounded expanded downloads
  against native Foyer S3FIFO and a local exact-key cost-aware Foyer policy. The expanded strategy looks 5 ms
  ahead, coalesces gaps below the 10,000,000-byte source-cost break-even distance, and defaults to whole splits
  below 8 MiB and exact ranges otherwise.
- [Memory results](benchmarks/memory/results.md) cover configured targets from 256 MiB through 32 GiB and
  report request/source-cost hit rates, actual retained memory, throughput, and shard-count sensitivity.
  These results do not establish general or tiered superiority over Foyer.
- The complete memory gate was rerun after sharing access evidence (eight capacities through 32 GiB, exact
  and expanded downloads, 1/4/16/64 shards). All hit counts, hit bytes, source GETs/bytes and retained-payload
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

## In progress: disk range-cache prototype

[`feuer-storage/disk-prototype.md`](feuer-storage/disk-prototype.md) specifies the experimental layout and
background recovery and remaining crash testing. `DiskRangeCache::insert_batch` groups smaller entries together within each
shard, assembles whole chunks including entry metadata and chunk metadata, writes each chunk once, and then
publishes after containment revalidation. Partial final chunks are finalized too. Later batches cannot fill them. Full keys and exact
object ranges map to one contiguous physical range each. Payload bytes have no metadata gaps, including at chunk boundaries. Entry metadata stores
one XXHash64 checksum per entry, also retained in the in-memory index. `get` reads and hashes the entire covering
entry while copying only requested bytes into the result. It does not read neighboring entries or metadata.

Independent shards have their own allocator and range-index lock. The allocator tracks coalesced free
whole-chunk runs and temporary recovery-claim bitmaps. There is no persistent bitmap map or async index-write lock. Written chunks remain immutable.
All entry owners, queued writes, and read guards must release a chunk before reuse. Queued writes retain
chunk ownership through completion despite caller cancellation. Failed batches release chunks normally.
Publication is not transactional across shards. Callers bound batch memory and concurrency.

Entries within a batch share 1-MiB chunks with 4-KiB-aligned storage only when each entry's complete payload
and metadata fit inside that chunk. Multi-chunk entries own their chunks exclusively, including unused tails. Small entries use at least 4 KiB of payload
and share metadata pages. Variable-length records with inline keys are packed before all payloads in an allocation.
A single-entry batch costs at least one chunk. Removed entries leave holes that cannot be reused individually.
The v7 format uses XXHash64 for all on-disk checksums. Each contiguous allocation starts with one metadata page
recording its chunk count and packed record byte length. Recovery scans the records sequentially, without a bitmap.
Each record stores one payload address and length; continuation chunks have no headers. Allocation refuses scattered free chunks.
Cache generations isolate resets; batch IDs bind allocation headers to entry metadata. Older formats cold-reset.
Read invalidation compares expected payload checksums. Discarding a newer identical copy is an allowed miss.
Pressure eviction samples up to 64 live entries and selects the lowest recent retrieval value per payload
byte, using the same history, cost calculation and comparison as memory. Ties choose the oldest publication.
Alignment, metadata and chunk overhead do not enter the score. Only selected entries are removed. Neighbors
remain indexed and may keep a partially empty chunk unavailable. No eviction metadata reads are needed.
Each shard batch is limited to 64 sampled decisions and 4,096 chunks charged to removed entries. Guarded or active
storage remains unavailable, and exhausted budgets skip admission. `open_with_access_histories` connects the
disk cache to a memory cache's evidence. Public tier orchestration now uses it.
Recovery scans up to saved per-shard ends and publishes validated entries incrementally. New writes can
claim unscanned chunks; recovery retries when the read channel is full and admitted reads run in FIFO order. Layout changes start a new cache
generation and log a cold reset. See the prototype document for checkpoint and validation details.
No comparative layout/performance claim is established.

The range-cache tests cover persisted full-key entry metadata and payload checksums, containment races, caller
cancellation, corruption/reused payload, partial batch failure, metadata-only chunks, disjoint shards,
mixed-size packing, exclusive multi-chunk ownership, finalized chunk metadata, whole-chunk ownership/reuse,
bounded value-aware entry eviction, shared evidence across tiers, payload-only scoring, mixed-size churn,
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
- One worker drains up to 64 entries into an explicit immutable batch. There is no batching timer or flush API.
  Queue saturation skips candidates without blocking or failing successful lookups.
- Queued writes are discarded if their exact memory admission was evicted, replaced, or compacted.
- Active writes retain reservations through completion despite cancellation. Publication checks the original
  memory-entry identity under the memory shard lock, within the disk index lock. No memory operation takes
  a disk lock. Stale writes are discarded, and disk containment/allocation policy can still skip entries.
- Failed writes are logged and remain invisible. Shared evidence survives queued and active disk writes.
- Requests record exactly once before lookup, including requests that fail or are later canceled. Callback results already
  covered by disk are discarded without memory admission or another write.

On `m8g-32cpu-local-ssd`, all 145 workspace tests passed with real direct I/O and io_uring after integration.
New tests cover disk hits after memory pressure, requested-range promotion, exactly-once evidence, directory
ownership, byte/count saturation, oversized candidates, queued eviction/readmission, batching, callback and
writer cancellation, stale publication, and disk-contained callback suppression. No new corruption tests were
added in this slice. Workspace Clippy passed with warnings denied. Formatting checks passed for changed Rust
files, and whitespace checks passed.
The isolated validation checkout is `/mnt/local-ssd/feuer-tiered.iB2JNA`.

## Implemented: cache metrics

- `open_with_metrics` wires a `mixtrics` registry through public lookups, callbacks, both cache tiers and
  disk writes. `open` remains no-op. Labels contain only fixed operation/outcome/source values.
- Added lookup latency/outcomes and served bytes, callback counts/latency/download bytes, disk read-error
  and integrity outcomes, disk-write admission/skip/terminal outcomes, queue pressure/wait time,
  chunk capacity states, indexed payload/entries, pressure eviction and byte-weighted batch packing.
- Queue and capacity gauges follow ownership, including canceled work, detached writes, read guards,
  write failure and cache shutdown. See [`metrics.md`](metrics.md) for exact accounting semantics.
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

- Extend current memory-identity publication checks when tier-aware retention policy is implemented.
- Validate the versioned metadata format and whole-entry XXHash64 checksums against injected crash and reuse cases.
- Exercise incremental recovery and generation resets on real Linux direct I/O and io_uring.
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
