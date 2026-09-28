# Feuer

Feuer is a restart-recoverable tiered cache for byte ranges of immutable
objects. It is designed for arbitrary, non-empty ranges and fixed disk
capacities of at least 1 TiB.

> **Status:** active development. Public lookups connect memory, integrity-checked
> disk reads, and per-call callbacks, with bounded best-effort disk population.
> Recovery, buffered I/O, and tier-aware retention tuning remain unimplemented.
> This repository is not ready for production use.

## Contract

- Every successful lookup returns one contiguous `bytes::Bytes` containing exactly the requested object bytes.
- Each `get_or_fetch` supplies its own callback. On a miss, that callback returns one downloaded start and non-empty `Bytes`; Feuer derives the exact range.
- The application download manager owns debouncing, range selection, source scheduling, retries, and source-memory bounds.
- Feuer imposes no source alignment or public cache block size. Its memory target is soft: an oversized download empties its shard and remains cached;
  returned `Bytes` may share a larger heap allocation.
- Disk population uses a bounded best-effort queue. Queue pressure or memory eviction may skip a write without failing the lookup.
- Disk storage targets Linux with usable io_uring. Direct payload I/O never silently falls back to buffered I/O.
- Successful lookups append the exact requested range to the key's accessed ranges; downloaded-range population is separate and creates no access.
- Recovery may lose recent entries, but uncertain or corrupt bytes always miss.
  Persistence, checksums, allocation, and recovery formats remain internal.

The authoritative design and acceptance criteria are in
[`tiered-plan.md`](tiered-plan.md). Current implementation status and the next
work are tracked in [`implementation-status.md`](implementation-status.md).

## Current workspace

The public boundary contains `ObjectKey`, `ByteRange`, a keyless validated
`Download`, explicit capacities in `CacheConfig`, and a cloneable `TieredMemoryDiskCache`. Each
`get_or_fetch` checks memory, then disk, before independently invoking that call's asynchronous callback.
Successful results contain exactly the request and record it once, independently of population.
`TieredMemoryDiskCache::open(config).await` opens the configured directory and can fail;
there is no memory-only fallback. Disk capacity must be a positive multiple of 1 MiB.
Disk hits promote only the requested bytes to memory. Retained callback downloads are queued without
waiting for disk: at most 256 queued entries, 64 entries per active batch, and 64 MiB of queued plus active
payload. Larger downloads remain memory-only. Eviction before a write starts discards it; completed writes
publish only while their original memory admission is still current.

The internal `feuer-storage` crate currently provides an exclusively owned,
fixed-capacity O_DIRECT file and separate read/write io_uring queues (each with
its own thread and QD64), with arbitrary-range reads and 4-KiB-aligned write
offsets and lengths. Each queue schedules requests in arrival order without overlap
checks or a read-triggered write throttle.
Callers must prevent conflicting access to aligned byte ranges and retain a
submitted write's disk region until completion, even after cancellation.
Separate read/write admission
budgets protect read admission from write backlogs. Raw file capacity must be a positive multiple of
4 KiB. Above that, an experimental [`DiskRangeCache`](feuer-storage/disk-prototype.md)
connects allocation, persisted entry metadata, covering-range lookup and integrity-checked subrange reads.
Explicit batches group small entries into immutable 1-MiB chunks, each written once with its metadata.
Entries share a chunk only when their complete payload and metadata fit inside it; multi-chunk entries own
their chunks exclusively. Payload is 4-KiB-aligned with no page headers. Bounded pressure eviction selects
individual entries using the configured policy (cost-aware by default); it does not evict their neighbors as a group.
Chunks are reused only after all entry owners and readers release them.
Each entry has one checksum in its metadata; a hit verifies the whole entry and returns only the requested
bytes without reading neighboring entries. Reopen deliberately starts empty. Recovery and buffered mode
remain unimplemented.

`feuer-memory` provides a sharded soft-capacity covering-range index with bounded ageable request evidence,
configurable eviction. The default cost-aware policy uses sampled retrieval cost per byte, a short
range-trimming grace, and pressure-driven trimming of the selected victim toward observed requests. Its [memory-only comparison](benchmarks/memory/results.md)
records the current policy baseline. `feuer-types` holds shared range
foundations, while `feuer-tokio` remains the Tokio/madsim runtime switch. None of these internal
package boundaries is a public compatibility commitment. The cost-aware memory and disk policies share
volatile per-object access evidence and payload-value scoring through `feuer-types::retention`; evidence
survives memory eviction while disk entries retain it.

## Eviction policy

Set `FEUER_EVICTION_POLICY=s3fifo` to select S3-FIFO for both tiers, or
`FEUER_EVICTION_POLICY=cost-aware` for cost-aware eviction. `CacheConfig::new` reads
this variable, defaulting to `EvictionPolicy::CostAware` when unset and rejecting
invalid values. Override a valid environment setting for an individual cache with:

```rust
use feuer::{CacheConfig, EvictionPolicy};

let config = CacheConfig::new("cache", 1 << 30, 64 << 20)?
    .with_eviction_policy(EvictionPolicy::S3Fifo);
```

S3-FIFO maintains independent queues per shard and tier, weighted by payload bytes:

- A small FIFO targets 10% of shard capacity. Entries with at least two accesses move to the main FIFO.
- Main-queue accesses earn up to three second chances; hits do not reorder entries.
- Cold small-queue victims leave exact key/range ghosts, bounded to 90% of shard capacity in former payload bytes.
  Ghost readmissions go directly to the main FIFO; ghosts retain no payload or disk ownership.
- Memory counts successful covering accesses, including the request recorded with a callback download or disk promotion.
  Disk counts verified disk hits, not memory hits. Population alone does not count as an access.
- S3-FIFO evicts whole entries and disables memory range trimming. Soft memory capacity, bounded disk admission,
  and immutable-chunk/read-guard ownership are unchanged.

### Eviction work limit

Set `FEUER_RECLAIM_SAMPLE_SIZE=128` to inspect up to 128 candidates per memory
or disk eviction decision. `CacheConfig::new` reads this variable, defaulting to
64 when unset. Zero, invalid integers, and values larger than `usize` are rejected.
`config.with_reclaim_sample_size(...)` overrides a valid environment setting for
that cache. With S3-FIFO, this limits queue-head processing per decision instead of sampled candidates;
memory retries after promotions or second chances, while disk retains its per-batch work limits.

### Cost-aware access age

Set `FEUER_MAX_ACCESS_AGE_ACCESSES=65536` to retain request evidence for up to
65,536 later successful accesses to the same shard. The default is 32,768.
The shared memory/disk history reads this process-wide setting once on first use;
it also applies to standalone `MemoryCache` and the memory benchmark. Set it before
starting the process. A set value must be a positive decimal `u64`; invalid values
panic. `18446744073709551615` effectively disables age expiration.

This changes cost-aware scoring and range-trimming evidence, not S3-FIFO counters
or the separate limit of 64 recorded requests per object. Entries can still be
evicted under capacity pressure.

## Metrics

`TieredMemoryDiskCache::open_with_metrics(config, &registry).await` accepts a
`mixtrics::metrics::BoxedRegistry` and enables lookup, callback, memory, disk I/O,
population, capacity and packing metrics. `open(config)` retains no-op metrics.
Labels are bounded and contain no object identities. The application owns metric
export. See [the metric inventory and accounting semantics](metrics.md).

## Development

Storage builds and tests require Linux, enabled io_uring, and a filesystem that
reports compatible direct-I/O alignment through `statx` (for example, ext4 on a
recent kernel). Unsupported environments fail explicitly; tests do not silently
skip real I/O. On other platforms, configuration, memory and shared-type tests still run with
`cargo test -p feuer -p feuer-memory -p feuer-types`.

```console
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

## License

Licensed under the [MIT License](LICENSE). Inspired in part by
[Foyer](https://github.com/foyer-rs/foyer).
