# Feuer Implementation Status

**Status:** implementation is in progress. [`tiered-plan.md`](tiered-plan.md) is the authoritative behavioral
contract; this document records what exists and what remains to build.

## Current state

The public cache is **memory-only**. It does not open or modify the configured disk directory. A standalone
experimental `DiskRangeCache` now connects allocation, metadata writes and integrity-checked disk reads,
but is not integrated with public lookup or population and does not recover entries on reopen.

| Area | Implemented | Remaining |
| --- | --- | --- |
| `feuer` | Cloneable `TieredMemoryDiskCache`, one soft memory target, per-call asynchronous `get_or_fetch`, typed callback and validation errors | Disk lifecycle, I/O mode selection, and tier orchestration |
| `feuer-types` | String-backed fully compared `ObjectKey`, exact non-empty `ByteRange`, keyless `Download` with a derived range, internal shared access evidence and value comparison | None for the current public type boundary |
| `feuer-memory` | Sharded covering-range index, bounded exact access evidence shared with disk, sampled retention policy, pressure-driven compaction, payload accounting, metrics | Wall-clock evidence aging, disk-state inputs, further trace-independent evaluation |
| `feuer-storage` | Fixed-capacity Linux O_DIRECT file, bounded QD64 io_uring driver, experimental sharded `DiskRangeCache` with explicit batches packed into immutable 1-MiB chunks, whole-entry checksums, guarded whole-chunk reuse and bounded value-aware entry eviction | Recovery, buffered mode, retention-policy evaluation, comparative allocator measurements |
| Runtime and tooling | `feuer-tokio`, Feuer-only workspace/CI, memory comparison gate, raw storage benchmarks | End-to-end acceptance and crash tests, examples, tiered and concurrent cache benchmarks |

## Implemented behavior

### Public lookup and memory population

- Memory lookup finds a cached range covering the request before invoking the per-call callback.
- Misses invoke callbacks independently, without internal coordination or source retries. Each callback returns
  one `Download { downloaded_start, bytes }`, whose derived range must cover the request.
- Successful lookups return exactly the requested bytes in one `Bytes`, which may share a larger allocation.
- Population discards a download already contained by cached data. Partially overlapping downloads may remain
  independent; broader downloads replace contained ranges.
- Each successful lookup records its exact requested range once. Population and access are distinct policy
  events, applied atomically under the shard lock for callback results.
- Capacity is a soft payload-byte target divided among shards. An oversized download empties its shard and
  remains admitted even when retained payload exceeds the target. Caller-held results survive eviction and
  are outside cache accounting.

### Memory retention and compaction

- Each key retains at most 64 exact, repeated access events. Events expire after 32,768 later successful
  same-shard accesses. Both tiers use the shared history and cost calculation in `feuer-types::retention`.
  Evidence survives memory eviction while disk entries or active population retain it; releasing the final
  owner removes the weak registry record. Histories are volatile, not persisted. Wall-clock aging is not
  implemented.
- Each event contributes 10,000,000 fixed-cost-equivalent bytes plus its requested bytes. Only cached ranges
  covering the exact event receive credit.
- A dense rotating candidate ring supplies a shared sample of at most 64 live entries per pressure decision.
  The victim has the lowest retrieval value per retained byte, with monotonic entry identity breaking ties.
  Registration and removal are constant-work and leave no stale candidate backlog.
- After a grace of 64 successful same-shard accesses, the selected victim is trimmed to observed requests if
  that releases at least one quarter of its payload; otherwise it is evicted. Grace never prevents eviction.
- Compaction merges only overlapping or adjacent observed intervals and copies them into independent `Bytes`.
  It preserves exact coverage, updates accounting and metrics, creates no access, and leaves caller-held
  slices valid.
- Copying happens outside the shard lock. Generation checks reject output invalidated by concurrent access
  through either tier or same-object structural changes; admission falls back to eviction.

There is no periodic compaction, separate prefetch-promotion state, or public policy configuration.

### Raw direct I/O

- Reads and writes have separate request channels, pending queues, active slots, rings, and worker threads.
  Each queue overlaps up to 64 operations in arrival order and wakes for new requests while I/O is outstanding,
  without busy polling or registered buffers.
- Read and write admission each reserve up to 64 requests and 64 MiB of aligned I/O buffers. Chunks are at most
  1 MiB. Caller inputs and read-result allocations are outside the combined 128-MiB I/O buffer memory budget.
- Reads accept arbitrary byte ranges; write offsets and lengths must be multiples of 4 KiB, enforced by
  assertions. The driver performs no read-modify-write or overlap checks: the upper layer must supply
  complete aligned blocks and prevent conflicting access across physical byte ranges rounded outward
  to 4 KiB. `DataFile` rounds read requests outward and slices the results; the io_uring driver accepts
  only nonempty aligned requests and returns complete aligned read buffers.
- Multi-chunk operations are not atomic. Completion permits subsequent reads but does not guarantee crash
  durability; there are no global scheduling barriers.
- Caller cancellation does not cancel submitted kernel writes. The task owning a write's `DiskRegion` must
  keep awaiting completion and prevent conflicting access or reuse, even when the result is abandoned.
  The standalone `DiskRangeCache` now owns reservation lifetime and publication; public tier scheduling is still pending.
- Submitted buffers survive caller cancellation. Last-handle drop drains and joins the driver. Abnormal
  driver failure retains uncertain active buffers and the directory lock until process exit.
- Raw capacity must be positive, 4-KiB-aligned, and representable as a Linux signed file offset. Opening
  requires usable io_uring and compatible `STATX_DIOALIGN`; there is no backend or buffered fallback.

This is a raw I/O layer, not a disk cache or population queue. Disk storage and its CI target Linux only.

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
  Feuer's median elapsed-time ratio across its 64 cases was 1.153 (about 15% slower); the native Foyer medians
  were approximately unchanged. Shared evidence adds synchronization and metadata; this is not a throughput
  improvement. CSV artifacts on `m8g-32cpu-local-ssd` are
  `/mnt/local-ssd/feuer-value.BkkxPo/memory-gate.csv` and
  `/mnt/local-ssd/feuer-pressure.RgwqyQ/memory-baseline.csv`.
- Storage tests cover independent read/write queue capacity and progress, mixed concurrency,
  caller-serialized shared pages, bounds, short I/O, cancellation, and reopening.
- The [direct-I/O smoke benchmark](benchmarks/storage/README.md) measures the driver on the local SSD, not
  an end-to-end cache or matched backend comparison. Separate [SSD measurements](benchmarks/ssd/ssd-concurrent-read-write.md)
  explore mixed reads and writes; small-read-heavy workloads may still warrant write throttling.

## In progress: disk range-cache prototype

[`feuer-storage/disk-prototype.md`](feuer-storage/disk-prototype.md) specifies the experimental layout and
remaining crash/recovery questions. `DiskRangeCache::insert_batch` groups smaller entries together within each
shard, assembles whole chunks including entry metadata and discovery bitmaps, writes each chunk once, and then
publishes after containment revalidation. Partial final chunks are finalized too; later batches cannot fill them. Full keys and exact
object ranges map to ordered physical regions. Payload bytes have no interleaved headers. Entry metadata stores
one BLAKE3 checksum per entry, also retained in the in-memory index. `get` reads and hashes the entire covering
entry while copying only requested bytes into the result. It does not read neighboring entries or metadata.

Independent arenas have their own allocator and range-index lock. The allocator tracks only coalesced free
whole-chunk runs. There is no persistent bitmap map or async index-write lock. Written chunks remain immutable;
all entry owners and read guards must release a chunk before reuse. Detached writer tasks retain reservations
despite caller cancellation; write errors or abandoned owners during I/O quarantine the shard's batch chunks.
Publication is not transactional across shards. Callers bound batch memory and concurrency.

Entries within a batch share 1-MiB chunks with 4-KiB-aligned storage only when each entry's complete payload
and metadata fit inside that chunk. Multi-chunk entries own their chunks exclusively, including unused tails. Small entries use at least 4 KiB of payload
plus 4 KiB of metadata inside their batch's chunks. Metadata pages are not shared between entries. A single-entry
batch costs at least one chunk; removed entries leave holes that cannot be reused individually.
The v3 format uses a checksum over the complete entry metadata and a bitmap of entry metadata starts,
without write IDs or generation counters.
Read invalidation compares expected payload checksums; discarding a newer identical copy is an allowed miss.
Pressure eviction samples up to 64 live entries and selects the lowest recent retrieval value per payload
byte, using the same history, cost calculation and comparison as memory. Ties choose the oldest publication.
Alignment, metadata and chunk overhead do not enter the score. Only selected entries are removed; neighbors
remain indexed and may keep a partially empty chunk unavailable. No eviction metadata reads are needed.
Each shard batch is limited to 64 sampled decisions and 4,096 removed region references. Guarded or active
storage remains unavailable, and exhausted budgets skip admission. `open_with_access_histories` connects the
standalone disk cache to a memory cache's evidence; public tier orchestration is still pending.
Reopening deliberately starts empty and logs the reset. Recovery and the bounded memory-to-disk queue remain
unimplemented. No comparative layout/performance claim is established.

The range-cache tests cover persisted full-key entry metadata and payload checksums, containment races, caller
cancellation, corruption/reused payload, partial batch failure, metadata-only chunks, disjoint arenas,
mixed-size packing, exclusive multi-chunk ownership, finalized discovery bitmaps, whole-chunk ownership/reuse,
bounded value-aware entry eviction, shared evidence across tiers, payload-only scoring, mixed-size churn,
concurrent eviction/reads,
fragmented chunks and whole-entry
validation of 100-MiB subrange hits. A 1-KiB hit succeeds with only its aligned payload block readable;
unrelated entries and metadata are not loaded. On `m8g-32cpu-local-ssd`, all 71 storage tests and all 135 workspace tests passed with real direct I/O
and io_uring on the local ext4 SSD. Workspace Clippy passed with warnings denied; formatting and whitespace
checks passed for the changed files.

## Later: best-effort disk scheduling

- Bound the pending-write queue by payload bytes and entry count.
- Allow policy to skip or replace candidates without blocking or failing a successful lookup.
- Cancel queued writes when their memory range is evicted before I/O starts.
- Let active writes finish safely, publishing only a current generation still admitted by disk policy.
- Log failed writes and keep them invisible to disk lookup.

Scheduling must integrate with `DiskRangeCache` before the public cache can serve disk hits.

## Remaining implementation

### Disk range engine and I/O modes

- Integrate and measure covering-range lookup and whole-entry validation, including subrange read amplification.
- Add `PayloadIoMode::Buffered` and `PayloadIoMode::Direct`, integrating the existing driver without silent
  backend fallback.
- Keep requested and downloaded ranges arbitrary and unaligned; physical entry allocations are 4-KiB-aligned.
- Evaluate sampled value-aware disk eviction, payload-only scoring and partially empty chunk utilization.
- Bound allocation work, fragmentation reclamation, rewrite traffic, metadata, and physical capacity at the
  contract's target scale of at least 30 TiB; avoid a cache-wide hot lock.

The allocator is undecided. Compare size-segregated slabs, append-packed storage with cleaning, and a
Foyer-style block baseline using trace-driven simulation and focused prototypes. Measure useful utilization,
fragmentation, metadata, allocation latency, and relocation traffic before selecting a design. Layout,
alignment, size classes, and cleaning remain private implementation choices.

Multi-range assembly and physical difference-only storage for overlapping downloads are deferred, not
permanently excluded by the design.

### Integrity and recovery

- Extend the prototype's checked reads and safe publication to tier-policy revalidation.
- Validate the versioned metadata format and whole-entry BLAKE3 checksums against injected crash and reuse cases.
- Recover a safe subset after process and machine crashes; test corruption and torn state.
- Automatically reset unsupported persistent formats and log the reset.

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
  concurrency, and cold-cache start constant; report actual used memory.
- Evaluate prefetch and downloaded-range selection separately. Instrument actual source GETs and bytes in
  the application harness and charge `GETs * 125 ms + bytes / 80 MB/s`; callback counts are not source GETs.
- Report cost-weighted, request, and byte hit rates; useful-payload utilization; fragmentation and metadata;
  read, write, cleaning, and relocation amplification; throughput; and tail latency.
- Keep allocator and policy choices experimental until measurements support them. Make no claim of beating
  Foyer before controlled results demonstrate it.
