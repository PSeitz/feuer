# Feuer Implementation Status

**Status:** implementation is in progress. [`tiered-plan.md`](tiered-plan.md) is the authoritative behavioral
contract; this document records what exists and what remains to build.

## Current state

The public cache is **memory-only**. It does not open or modify the configured disk directory. The raw
Linux direct-I/O layer is implemented separately but is not yet connected to cache lookup or population.

| Area | Implemented | Remaining |
| --- | --- | --- |
| `feuer` | Cloneable `Cache`, one soft memory target, per-call asynchronous `get_or_fetch`, typed callback and validation errors | Disk lifecycle, I/O mode selection, and tier orchestration |
| `feuer-types` | String-backed fully compared `ObjectKey`, exact non-empty `ByteRange`, keyless `Download` with a derived range | None for the current public type boundary |
| `feuer-memory` | Sharded covering-range index, bounded exact access evidence, sampled retention policy, pressure-driven compaction, payload accounting, metrics | Wall-clock evidence aging, disk-state inputs, further trace-independent evaluation |
| `feuer-storage` | Exclusively locked fixed-capacity Linux O_DIRECT file, bounded QD64 io_uring driver, arbitrary-range reads, aligned writes, tracing, metrics | Buffered mode, range index, allocation, integrity, persistent metadata, recovery |
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
  same-shard accesses; removing the key's last cached range releases its metadata. Wall-clock aging is not
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
- Copying happens outside the shard lock. Generation checks reject output invalidated by concurrent access or
  same-object structural changes; admission falls back to eviction.

There is no periodic compaction, separate prefetch-promotion state, or public policy configuration.

### Raw direct I/O

- One dedicated thread/ring overlaps up to 64 reads and writes in arrival order. It wakes for new requests
  while I/O is outstanding, without polling or registered buffers.
- Read and write admission each reserve up to 64 requests and 64 MiB of staging buffers. Chunks are at most
  1 MiB. Caller inputs and read-result allocations are outside the combined 128-MiB staging budget.
- Reads accept arbitrary byte ranges; write offsets and lengths must be multiples of 4 KiB, enforced by
  assertions. The driver performs no read-modify-write or overlap checks: the upper layer must supply
  complete aligned blocks and prevent conflicting access across physical byte ranges rounded outward
  to 4 KiB. `DataFile` rounds read requests outward and slices the results; the io_uring driver accepts
  only nonempty aligned requests and returns complete aligned read buffers.
- Multi-chunk operations are not atomic. Completion permits subsequent reads but does not guarantee crash
  durability; there are no global scheduling barriers.
- Caller cancellation does not cancel submitted kernel writes. The task owning a write's `DiskRegion` must
  keep awaiting completion and prevent conflicting access or reuse, even when the result is abandoned.
  Publication belongs to the future disk range engine.
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
- Storage tests cover mixed concurrency, caller-serialized shared pages, bounds, short I/O, cancellation,
  and reopening.
- The [direct-I/O smoke benchmark](benchmarks/storage/README.md) measures the driver on the local SSD, not
  an end-to-end cache or matched backend comparison. Separate [SSD measurements](benchmarks/ssd/ssd-concurrent-read-write.md)
  explore mixed reads and writes; small-read-heavy workloads may still warrant write throttling.

## Next: best-effort disk scheduling

- Bound the pending-write queue by payload bytes and entry count.
- Allow policy to skip or replace candidates without blocking or failing a successful lookup.
- Cancel queued writes when their memory range is evicted before I/O starts.
- Let active writes finish safely, publishing only a current generation still admitted by disk policy.
- Log failed writes and keep them invisible to disk lookup.

Scheduling must integrate with the disk engine below before the public cache can serve disk hits.

## Remaining implementation

### Disk range engine and I/O modes

- Add covering-range lookup and request-sized reads with bounded integrity/alignment overhead.
- Add `PayloadIoMode::Buffered` and `PayloadIoMode::Direct`, integrating the existing driver without silent
  backend fallback.
- Keep requested and downloaded ranges arbitrary and unaligned. Pack sub-alignment values without charging
  each one a complete physical alignment unit.
- Implement allocation and safe reuse. A `DiskRegionReadGuard` must prevent overwriting or reusing a disk
  region while a read depends on it, without serializing concurrent reads.
- Bound allocation work, fragmentation reclamation, rewrite traffic, metadata, and physical capacity at the
  contract's target scale of at least 30 TiB; avoid a cache-wide hot lock.

The allocator is undecided. Compare size-segregated slabs, append-packed storage with cleaning, and a
Foyer-style block baseline using trace-driven simulation and focused prototypes. Measure useful utilization,
fragmentation, metadata, allocation latency, and relocation traffic before selecting a design. Layout,
alignment, size classes, and cleaning remain private implementation choices.

Multi-range assembly and physical difference-only storage for overlapping downloads are deferred, not
permanently excluded by the design.

### Integrity and recovery

- Validate disk bytes before returning them; uncertainty becomes a miss.
- Prevent failed, stale, superseded, or partial writes from becoming lookup-visible.
- Choose and version internal checksum and persistent metadata formats.
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
