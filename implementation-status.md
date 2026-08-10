# Feuer Implementation Status

**Status:** implementation is in progress. [`tiered-plan.md`](tiered-plan.md) is the authoritative behavioral contract. This document records the current implementation boundary and next work.

## Current state

| Area | Implemented | Still missing |
| --- | --- | --- |
| `feuer` | One soft memory target, cloneable `Cache`, and per-call asynchronous `get_or_fetch` with typed callback and validation errors | Disk open/close, mode selection, `flush`, and memory/disk orchestration |
| `feuer-types` | String-backed fully compared `ObjectKey`, exact non-empty `ByteRange`, and keyless `Download { downloaded_start, bytes }` with a derived range | No work remaining for the current public type boundary |
| `feuer-memory` | Sharded soft-capacity covering-range index, bounded and aged exact evidence, sampled value-aware retention, short compaction grace, pressure trimming of the selected victim, payload accounting, and metrics | Later tier-aware disk-state inputs and further trace-independent policy tuning |
| Memory/disk orchestration | Public callback-to-memory path, including independent misses and redundant-population suppression | Best-effort bounded disk scheduling, cancellation on memory eviction, active-write generations, disk publication, and queue flushing |
| `feuer-storage` | Exclusively locked fixed-capacity file, buffered positional I/O, synchronization, tracing, and metrics | Linux/macOS direct mode, range lookup, allocation, integrity validation, persistent metadata, and recovery |
| Runtime and tooling | `feuer-tokio`, Feuer-only workspace/CI, repository metadata, and a documented [memory-only comparison gate](benchmarks/memory/results.md) against pinned native Foyer | End-to-end acceptance tests, crash tests, examples, and disk/concurrent benchmarks |

The public cache currently constructs the in-memory path only; it does not open or modify the configured disk directory before the disk lifecycle exists.

## Behavioral boundary

The completed callback path and the remaining implementation slices follow these boundaries:

- A download callback is supplied to each `get_or_fetch`; there is no constructor-registered callback.
- Feuer performs no callback debouncing, leader election, waiter coordination, or source retries.
- The application download manager owns range batching, downloaded-range selection, source work, and pre-return memory bounds.
- Each callback returns exactly one keyless `Download`; the `get_or_fetch` call already fixes the object key.
- A successful result is one contiguous `Bytes` containing exactly the requested object bytes. It may share a larger heap allocation.
- Disk population is bounded and best-effort. Downloads waiting for disk are not protected from memory eviction, and disk queue pressure never blocks a successful lookup.
- Persistence layout, allocator design, checksum details, and recovery mechanism are internal implementation choices rather than product-contract commitments.

## Implemented slices

### 1. Public callback and memory path

- Replaced the two temporary memory capacities with one Foyer-style soft payload-byte target divided among shards.
- Replaced `ExpandedDownload` with keyless `Download { downloaded_start, bytes }`; its exact range is derived from the payload length.
- Added the cache handle and per-call `get_or_fetch(object_key, requested_range, callback)` API.
- Added a covering memory-range lookup before callback invocation.
- Invoke each missed call's callback independently and accept exactly one covering `Download` or callback error.
- Admit downloads larger than their shard target after evicting the shard, allowing retained usage to exceed the configured target as in Foyer.
- Discard redundant population when existing cached data contains the returned downloaded range, while still returning the call's requested bytes.
- Permit returned `Bytes` to share a larger backing allocation.
- Append every successful requested range exactly once to its key's accessed ranges, independently of whichever
  downloaded range satisfies it; population itself creates no access.

This slice introduces no fill context, pre-fetch reservation, internal miss singleflight, source retry, or mandatory disk-write protection.

### 2. Complete and evaluate the in-memory cache

- Replaced each unbounded per-key access sequence with at most 64 exact, repeated events in deterministic
  4,096-successful-access shard-local epochs. Evidence decays over eight epochs and expires, while removing a
  key's last extent releases its metadata.
- Replaced arbitrary `HashMap` victim selection with deterministic aged-retrieval-value-per-retained-byte
  ordering. Only an extent that covers an exact requested event receives its source-cost-weighted credit;
  repetition increases retention and stale popularity loses to fresh evidence.
- Added a pure compaction planner that projects active exact requests onto one downloaded extent, merges only
  overlapping or adjacent intervals, and requires a private minimum saving before copying.
- Compaction runs opportunistically on a shard-local access cadence and before pressure eviction. Replacement
  copies produce independent `Bytes`, preserve surviving exact coverage and containment invariants, update
  payload/entry accounting and compaction metrics, create no access, and cannot invalidate caller-held slices.
- Added atomic callback population plus attribution under one shard lock, while keeping population and access as
  distinct policy events and preserving access evidence across same-object replacement.
- Extended the [memory benchmark](benchmarks/memory/README.md) with a source-bounded expanded strategy and an
  exact-range control over the complete captured trace. On a miss, expanded replay looks 5 ms ahead and
  coalesces same-object ranges whose gaps are below the 10,000,000-byte source-cost break-even distance.
  Downloads default to whole splits below 8 MiB and exact ranges otherwise; environment variables can change
  the whole-split threshold and coalescing distance.
- Ran the gate at 256, 512, 1,024, 2,048, 4,096, 8,192, 16,384, and 32,768 MiB with 16 shards against pinned
  native Foyer. The [configuration and results](benchmarks/memory/results.md) report cache-hit and source-cost
  hit rates, end-of-run used-memory accounting, and throughput, exposing further policy and compaction tuning
  opportunities.

This slice adds no disk queue, storage lifecycle, or public policy configuration.

### 3. Scalable and simplified memory policy

- Replaced full-shard policy scans with one dense rotating candidate ring and one shared sample of at most 64
  live entries per decision. Registration and removal are constant-work and leave no stale candidate backlog.
- Removed access-cadence compaction and reduced pressure policy to one decision: choose the sampled extent with
  the lowest aged retrieval value per retained byte. After 64 successful same-shard accesses, trim that same
  victim to observed exact requests when the planner can reclaim at least one quarter; otherwise evict it.
- Removed the separate compaction-candidate ranking, demonstrated-prefetch tracking, promotion state, and
  benchmark-only policy switch. Replay showed that selecting and trimming one victim preserved the checked hit
  rates without those mechanisms.
- Kept compaction copying outside the shard lock with generation-checked publication. Concurrent access or
  same-object structural mutation rejects stale copied output, and admission falls back to eviction.
- Preserved exact results, containment replacement, independent partial overlaps, oversized soft-target
  admission, bounded exact evidence, deterministic tie-breaking, and distinct population/access events.
- Re-ran expanded and exact controls over all eight capacities with 16 shards; the
  [full results](benchmarks/memory/results.md) report request and source-cost hit rates, actual used memory, and
  throughput without making a general superiority claim.

This slice adds no disk lifecycle, public policy setting, or wall-clock timer.

## Next coherent implementation slice

### 4. Best-effort disk scheduling

- Add a disk-write queue bounded by payload bytes and entry count.
- Let queue policy skip or replace disk candidates without affecting lookup success.
- Cancel queued writes when their memory range is evicted before I/O starts.
- Let active writes finish, but publish only a current generation still admitted by disk policy.
- Log failed writes and never expose them through disk lookup.
- Implement `flush` as a wait for currently queued and running writes, without retrying skipped or canceled work.

## Subsequent implementation slices

### 5. Range disk engine and I/O modes

- Add covering range lookup and request-sized positional reads with bounded integrity/alignment overhead.
- Add `PayloadIoMode::Buffered` and `PayloadIoMode::Direct`.
- Implement direct mode on Linux and macOS and reject it when the platform or filesystem cannot honor it.
- Keep arbitrary requested and downloaded ranges independent of internal alignment.
- Support packing retained values below the physical I/O alignment rather than charging every small value one
  complete alignment unit.
- Select allocation, extent, overlap, rewrite, and multi-extent strategies behind private boundaries.
- Keep fragmentation reclamation, rewrite traffic, allocator latency, the disk-write queue, and physical
  capacity bounded at 1-TiB-plus sizes.

The allocator mechanism is deliberately unsettled. Non-authoritative candidates include size-segregated slabs
with larger extents, append-packed storage with cleaning, and a Foyer-style block baseline. Use trace-driven
simulation and focused prototypes to compare useful utilization, fragmentation, metadata, allocation latency,
and cleaning or relocation traffic before selecting one. No slab size, size class, region, alignment quantum,
or cleaning algorithm belongs in the product contract.

The MVP need not guarantee multi-extent assembly or physical difference-only storage for partially overlapping downloads, but the design must not make either a permanent public impossibility.

### 6. Integrity and recovery

- Validate disk-backed bytes before returning them and turn every uncertainty into a miss.
- Prevent failed, stale, superseded, or partially completed writes from becoming lookup-visible.
- Choose and version the checksum and persistent metadata formats internally.
- Recover a safe subset after process and machine crashes, with corruption and torn-state injection.
- Automatically reset and log unsupported persistent formats.

### 7. Tier-aware policy, observability, and hardening

- Keep policy internal for the MVP.
- Extend the completed memory policy with `no disk copy`, `queued`, `active`, and `disk resident` state so memory
  retention can account for disk-versus-network cost.
- Bound the disk share of prefetched bytes that have not yet received an observed request.
- Evaluate tiered policy with the target source-time model of one 125-ms request plus transfer at 80 MB/s; do not
  expose those constants as public cache configuration.
- Provide low-overhead signals for tier outcomes, callback invocations and returned bytes, latency, memory
  pressure, compaction, disk queue outcomes, allocator utilization and fragmentation, rewrite traffic,
  integrity, and recovery without object-key labels. Instrument actual source GETs and bytes in the application
  benchmark harness, where application-owned coordination is visible.
- Add concurrency, randomized, target-scale, and end-to-end acceptance tests against the behavioral criteria in `tiered-plan.md`.

### 8. End-to-end comparative benchmark gate

- Build on and rerun the memory-only suite from slices 2 and 3; do not tune only to the captured sample.
- Run a controlled tiered cache-engine benchmark with identical callback download ranges, application
  coordination, configured memory target, disk capacity, concurrency, and cold-cache start; report actual used
  memory.
- Pin and report the native Foyer revision and tuning.
- Run prefetch and downloaded-range selection as a separate end-to-end benchmark, charging actual source GETs
  and bytes to `GETs * 125 ms + bytes / 80 MB/s`.
- Report cost-weighted, request, and byte hit rates; useful-payload utilization; fragmentation and metadata;
  read, write, cleaning, and relocation amplification; throughput; and tail latency.
- Treat slabs, regions, size thresholds, compaction, and policy data structures as experiments until these
  measurements select them. Publish no claim that Feuer beats Foyer before the controlled results support it.

## Guardrails

Until the remaining work lands:

- every successful lookup must return exactly its requested object bytes in one contiguous `Bytes`;
- requested and downloaded ranges remain arbitrary and unaligned;
- each per-call callback returns one downloaded start and non-empty payload, from which its exact range is derived;
- callback batching and source work remain application-owned;
- every successful lookup records one requested-range event;
- compaction considers only the selected sampled victim after 64 successful same-shard accesses;
- memory pressure may evict any extent and never waits for disk throughput;
- disk population remains bounded and best-effort;
- physical I/O alignment does not force one full allocation unit per small retained value;
- never-requested prefetch consumes bounded disk capacity;
- uncertain disk bytes always miss; and
- implementation choices must not become accidental public format or policy commitments.
